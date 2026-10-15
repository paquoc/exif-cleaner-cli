//! File-level work shared by the CLI and the GUI: finding PSD/PSB files, deciding where each
//! cleaned file goes, verifying the result and writing it atomically. Originals are overwritten
//! or left alone, never moved or deleted: backups are up to the user.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::clean::{self, Cleaner, Event, Options, Stats};
use crate::error::{Error, Result};

const TMP_SUFFIX: &str = ".exifclean.tmp";

/// A file to process. `rel` is its path relative to the folder it was found in (just the file
/// name for a file given directly); it is what "keep folder structure" reproduces.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub path: PathBuf,
    pub rel: PathBuf,
}

/// Where cleaned files are written.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OutputRule {
    /// Output folder; `None` writes next to the input.
    pub folder: Option<PathBuf>,
    /// Recreate each file's sub-folders (relative to the folder it was found in) under `folder`.
    pub keep_structure: bool,
    /// Added to the file stem, e.g. `_clean` gives `name_clean.psd`.
    pub suffix: String,
}

impl OutputRule {
    /// The output path for `task`; `None` means the input is overwritten.
    pub fn resolve(&self, task: &Task) -> Option<PathBuf> {
        let input_dir = task.path.parent().unwrap_or(Path::new(""));
        let dir = match &self.folder {
            None => input_dir.to_path_buf(),
            Some(f) if self.keep_structure => f.join(task.rel.parent().unwrap_or(Path::new(""))),
            Some(f) => f.clone(),
        };
        let stem = task.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = task.path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let out = dir.join(format!("{stem}{}{ext}", self.suffix));
        if same_file(&out, &task.path) {
            None
        } else {
            Some(out)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Written (or, in a dry run, would be written).
    Cleaned,
    /// Nothing to remove (copied to the output path when there is one).
    AlreadyClean,
    /// The file does not exist (any more).
    Missing,
    Failed,
    Cancelled,
}

/// The outcome for one file.
#[derive(Clone, Debug, Serialize)]
pub struct FileReport {
    pub path: PathBuf,
    pub status: Status,
    pub dry_run: bool,
    pub size_before: u64,
    /// Size of the cleaned file (also in a dry run).
    pub size_after: Option<u64>,
    /// Where the result was written (the input itself when overwritten).
    pub output: Option<PathBuf>,
    pub stats: Stats,
    pub error: Option<String>,
    pub millis: u64,
}

/// `true` for `.psd` / `.psb` (any case).
pub fn is_psd_path(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("psd") || e.eq_ignore_ascii_case("psb"))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Expands files and folders (recursively) into PSD/PSB tasks, skipping temporary files. Paths that do not exist are kept, so they report `Missing`.
pub fn scan(inputs: &[PathBuf]) -> Vec<Task> {
    let mut out = Vec::new();
    for input in inputs {
        if input.is_dir() {
            let mut found = Vec::new();
            walk_dir(input, 0, &mut found);
            found.sort();
            for path in found {
                let rel = path.strip_prefix(input).map(Path::to_path_buf).unwrap_or_else(|_| path.clone());
                out.push(Task { path, rel });
            }
        } else {
            let rel = input.file_name().map(PathBuf::from).unwrap_or_else(|| input.clone());
            out.push(Task { path: input.clone(), rel });
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|t| seen.insert(fs::canonicalize(&t.path).unwrap_or_else(|_| t.path.clone())));
    out
}

fn walk_dir(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 64 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            walk_dir(&path, depth + 1, out);
        } else if ft.is_file() && is_psd_path(&path) && !path.to_string_lossy().ends_with(TMP_SUFFIX) {
            out.push(path);
        }
    }
}

/// How one file is processed.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    /// Clean in memory and report; write and move nothing.
    pub dry_run: bool,
    /// Output path; `None` overwrites the input.
    pub output: Option<PathBuf>,
}

/// Cleans one file. Never panics; every failure is in the report and leaves the input as it was.
pub fn process(task: &Task, plan: &Plan, opts: &Options, cancel: Option<&AtomicBool>, on_event: &mut dyn FnMut(&Event)) -> FileReport {
    let started = Instant::now();
    let mut report = FileReport {
        path: task.path.clone(),
        status: Status::Failed,
        dry_run: plan.dry_run,
        size_before: 0,
        size_after: None,
        output: None,
        stats: Stats::default(),
        error: None,
        millis: 0,
    };
    match run(task, plan, opts, cancel, on_event, &mut report) {
        Ok(status) => report.status = status,
        Err(Error::Cancelled) => report.status = Status::Cancelled,
        Err(e) => {
            report.status = Status::Failed;
            report.error = Some(e.to_string());
        }
    }
    report.millis = started.elapsed().as_millis() as u64;
    report
}

fn cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|c| c.load(Ordering::Relaxed))
}

fn run(task: &Task, plan: &Plan, opts: &Options, cancel: Option<&AtomicBool>, on_event: &mut dyn FnMut(&Event), report: &mut FileReport) -> Result<Status> {
    let path = &task.path;
    let meta = match fs::metadata(path) {
        Ok(m) if m.is_file() => m,
        Ok(_) => return Err(Error::Invalid("not a file".into())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Status::Missing),
        Err(e) => return Err(Error::io("cannot read file info", e)),
    };
    report.size_before = meta.len();
    let stamp = (meta.len(), meta.modified().ok());
    let bytes = fs::read(path).map_err(|e| Error::io("cannot read file", e))?;

    let mut cleaner = Cleaner::new(opts.clone()).with_events(on_event);
    if let Some(c) = cancel {
        cleaner = cleaner.with_cancel(c);
    }
    let result = cleaner.clean(&bytes);
    report.stats = std::mem::take(&mut cleaner.stats);
    let cleaned = result?;
    if let Some(out) = &cleaned {
        verify(out, opts, &report.stats)?;
    }
    report.size_after = Some(cleaned.as_ref().map_or(report.size_before, |o| o.len() as u64));
    let status = if cleaned.is_some() { Status::Cleaned } else { Status::AlreadyClean };
    if plan.dry_run || cancelled(cancel) {
        return if cancelled(cancel) { Err(Error::Cancelled) } else { Ok(status) };
    }

    let output = plan.output.as_ref().filter(|o| !same_file(o, path));
    match (output, &cleaned) {
        // Overwrite, nothing to change: leave the file alone.
        (None, None) => return Ok(status),
        (None, Some(out)) => {
            let meta = fs::metadata(path).map_err(|e| Error::io("file disappeared while cleaning", e))?;
            if (meta.len(), meta.modified().ok()) != stamp {
                return Err(Error::Invalid("the file changed on disk while it was being cleaned; run again".into()));
            }
            // Keep the original's creation time, so the cleaned file does not look new.
            write_atomic(path, out, meta.created().ok())?;
            report.output = Some(path.clone());
        }
        (Some(target), out) => {
            if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
                fs::create_dir_all(parent).map_err(|e| Error::io(format!("cannot create {}", parent.display()), e))?;
            }
            // An already clean file is copied as is, so the output folder is complete.
            write_atomic(target, out.as_deref().unwrap_or(&bytes), None)?;
            report.output = Some(target.clone());
        }
    }
    Ok(status)
}

/// Re-reads the cleaned bytes before anything is written.
fn verify(out: &[u8], opts: &Options, stats: &Stats) -> Result<()> {
    let s = clean::summarize(out, opts).map_err(|e| Error::Invalid(format!("verification failed, nothing written: {e}")))?;
    if stats.warnings.is_empty() && (s.errors > 0 || s.ancestors > 0) {
        return Err(Error::Invalid(format!("verification failed, nothing written: {} unreadable parts, {} ancestors left", s.errors, s.ancestors)));
    }
    Ok(())
}

fn tmp_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(format!(".{}{TMP_SUFFIX}", std::process::id()));
    target.with_file_name(name)
}

/// Writes `bytes` to `target` through a temp file next to it, so a failure never leaves a
/// half-written file. `created` becomes the result's creation time where the platform allows
/// setting one (Windows, macOS).
fn write_atomic(target: &Path, bytes: &[u8], created: Option<SystemTime>) -> Result<()> {
    let tmp = tmp_path(target);
    let res = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        if let Some(t) = created {
            set_created(&f, t)?;
        }
        f.sync_all()
    })();
    if let Err(e) = res {
        let _ = fs::remove_file(&tmp);
        return Err(Error::io(format!("cannot write {}", tmp.display()), e));
    }
    fs::rename(&tmp, target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        Error::io(format!("cannot write {} (is it open in another program?)", target.display()), e)
    })
}

#[cfg(windows)]
fn set_created(f: &fs::File, t: SystemTime) -> std::io::Result<()> {
    use std::os::windows::fs::FileTimesExt;
    f.set_times(fs::FileTimes::new().set_created(t))
}

#[cfg(target_os = "macos")]
fn set_created(f: &fs::File, t: SystemTime) -> std::io::Result<()> {
    use std::os::macos::fs::FileTimesExt;
    f.set_times(fs::FileTimes::new().set_created(t))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn set_created(_: &fs::File, _: SystemTime) -> std::io::Result<()> {
    Ok(())
}
