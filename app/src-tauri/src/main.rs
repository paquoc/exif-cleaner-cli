//! Exif Cleaner PSD desktop app: a Tauri shell over `exif-cleaner-core`. The web UI lists files and
//! options; this side scans folders, checks files, runs the cleaner on a worker thread and
//! streams progress events back.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use exif_cleaner_core::job::{self, OutputRule, Plan, Task};
use exif_cleaner_core::{Event, Options};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

/// The cancel flag of the run in progress, if any.
#[derive(Default)]
struct Runner(Mutex<Option<Arc<AtomicBool>>>);

#[derive(Serialize)]
struct FileEntry {
    path: PathBuf,
    rel: PathBuf,
    size: Option<u64>,
}

#[derive(Serialize)]
struct FileCheck {
    path: PathBuf,
    exists: bool,
    size: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunOptions {
    dry_run: bool,
    same_folder: bool,
    output_folder: String,
    keep_structure: bool,
    suffix: String,
    max_depth: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileStart {
    index: usize,
    output: Option<PathBuf>,
}

#[derive(Clone, Serialize)]
struct Progress {
    index: usize,
    event: Event,
}

#[derive(Clone, Serialize)]
struct FileDone {
    index: usize,
    report: job::FileReport,
}

#[derive(Clone, Serialize)]
struct Finished {
    cancelled: bool,
}

fn file_size(p: &std::path::Path) -> Option<u64> {
    std::fs::metadata(p).ok().filter(|m| m.is_file()).map(|m| m.len())
}

/// Expands dropped or picked files and folders into PSD/PSB entries.
#[tauri::command]
async fn scan_paths(paths: Vec<PathBuf>) -> Vec<FileEntry> {
    job::scan(&paths).into_iter().map(|t| FileEntry { size: file_size(&t.path), path: t.path, rel: t.rel }).collect()
}

/// Paths passed on the command line (files dropped on the exe, "Open with").
#[tauri::command]
fn initial_paths() -> Vec<PathBuf> {
    std::env::args_os().skip(1).map(PathBuf::from).filter(|p| !p.as_os_str().to_string_lossy().starts_with('-')).collect()
}

/// Whether each file still exists, with its current size.
#[tauri::command]
async fn check_files(paths: Vec<PathBuf>) -> Vec<FileCheck> {
    paths.into_iter().map(|p| FileCheck { exists: p.is_file(), size: file_size(&p), path: p }).collect()
}

#[tauri::command]
fn start_run(app: AppHandle, runner: State<'_, Runner>, tasks: Vec<Task>, options: RunOptions) -> Result<(), String> {
    let mut slot = runner.0.lock().unwrap_or_else(PoisonError::into_inner);
    if slot.is_some() {
        return Err("A run is already in progress.".into());
    }
    if !options.same_folder && options.output_folder.trim().is_empty() {
        return Err("Choose an output folder, or tick \"Same folder as input\".".into());
    }
    let rule = OutputRule {
        folder: (!options.same_folder).then(|| PathBuf::from(options.output_folder.trim())),
        keep_structure: options.keep_structure,
        suffix: options.suffix.clone(),
    };
    let opts = Options { max_depth: options.max_depth.clamp(1, 256) };
    let cancel = Arc::new(AtomicBool::new(false));
    *slot = Some(cancel.clone());
    drop(slot);

    std::thread::spawn(move || {
        for (index, task) in tasks.iter().enumerate() {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            let plan = Plan { dry_run: options.dry_run, output: rule.resolve(task) };
            let _ = app.emit("run:file-start", FileStart { index, output: plan.output.clone() });
            // Throttle smart-object notifications: a file can hold thousands.
            let mut last = Instant::now() - Duration::from_secs(1);
            let mut on_event = |e: &Event| {
                if matches!(e, Event::Leave { .. }) || (last.elapsed() < Duration::from_millis(60) && !matches!(e, Event::Progress { fraction } if *fraction >= 1.0)) {
                    return;
                }
                last = Instant::now();
                let _ = app.emit("run:progress", Progress { index, event: e.clone() });
            };
            let report = job::process(task, &plan, &opts, Some(&cancel), &mut on_event);
            let _ = app.emit("run:file-done", FileDone { index, report });
        }
        let cancelled = cancel.load(Ordering::SeqCst);
        if let Some(state) = app.try_state::<Runner>() {
            *state.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
        }
        let _ = app.emit("run:finished", Finished { cancelled });
    });
    Ok(())
}

/// Stops the run after the current step; the file being processed is left unchanged.
#[tauri::command]
fn stop_run(runner: State<'_, Runner>) {
    if let Some(c) = runner.0.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
        c.store(true, Ordering::SeqCst);
    }
}

fn main() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Runner::default())
        .setup(|app| {
            let title = format!("{} v{}", app.package_info().name, app.package_info().version);
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_title(&title);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![initial_paths, scan_paths, check_files, start_run, stop_run])
        .run(tauri::generate_context!());
    if let Err(e) = result {
        eprintln!("Exif Cleaner PSD failed to start: {e}");
        std::process::exit(1);
    }
}
