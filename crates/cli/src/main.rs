//! `exif-cleaner`: strip `photoshop:DocumentAncestors` from PSD/PSB files and their smart objects.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use clap::Parser;
use exif_cleaner_core::job::{self, is_psd_path, OutputRule, Plan, Status};
use exif_cleaner_core::{Event, FileReport, Options};

/// Removes photoshop:DocumentAncestors from PSD/PSB files and every embedded PSD/PSB smart
/// object inside them, without Photoshop. Without -o each file is overwritten in place.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Files and/or folders (folders are searched recursively for .psd/.psb).
    #[arg(required = true)]
    paths: Vec<PathBuf>,
    /// Report what would be removed and the resulting sizes; write nothing.
    #[arg(short = 'n', long)]
    dry_run: bool,
    /// Write the result here instead of overwriting: a new file path for a single input file,
    /// otherwise a folder (sub-folders of folder inputs are kept).
    #[arg(short, long, value_name = "PATH")]
    output: Option<PathBuf>,
    /// Maximum smart-object nesting depth.
    #[arg(long, default_value_t = 32)]
    max_depth: usize,
    /// Print each smart object as it is cleaned.
    #[arg(short, long)]
    verbose: bool,
    /// Print one JSON report per file instead of text.
    #[arg(long)]
    json: bool,
}

fn human(n: u64) -> String {
    let n = n as f64;
    if n >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GB", n / (1024.0 * 1024.0 * 1024.0))
    } else if n >= 1024.0 * 1024.0 {
        format!("{:.2} MB", n / (1024.0 * 1024.0))
    } else {
        format!("{:.1} KB", n / 1024.0)
    }
}

fn print_report(i: usize, total: usize, r: &FileReport) {
    let name = r.path.display();
    let secs = r.millis as f64 / 1000.0;
    match r.status {
        Status::Cleaned | Status::AlreadyClean => {
            let before = r.size_before;
            let after = r.size_after.unwrap_or(before);
            let pct = if before > 0 { 100.0 * (before as f64 - after as f64) / before as f64 } else { 0.0 };
            let verb = match (&r.status, r.dry_run) {
                (Status::AlreadyClean, _) => "already clean",
                (_, true) => "would clean",
                _ => "cleaned",
            };
            let s = &r.stats;
            println!(
                "[{i}/{total}] {name}: {verb}  {} -> {} (-{pct:.1}%)  smart objects: {} ({} shared)  ancestors removed: {}  {secs:.1}s",
                human(before),
                human(after),
                s.smart_objects,
                s.shared_smart_objects,
                s.ancestors_removed
            );
            if !s.skipped_types.is_empty() {
                let t: Vec<String> = s.skipped_types.iter().map(|(k, v)| format!("{k} x{v}")).collect();
                println!("      untouched non-PSD smart objects: {}", t.join(", "));
            }
            if let (Some(o), false) = (&r.output, r.output.as_ref() == Some(&r.path)) {
                // A new file was written.
                println!("      output: {}", o.display());
            }
            for w in &s.warnings {
                println!("      warning: {w}");
            }
        }
        Status::Missing => println!("[{i}/{total}] {name}: file not found"),
        Status::Cancelled => println!("[{i}/{total}] {name}: stopped, file left unchanged"),
        Status::Failed => println!("[{i}/{total}] {name}: FAILED, file left unchanged: {}", r.error.as_deref().unwrap_or("unknown error")),
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let c = cancel.clone();
        let _ = ctrlc::set_handler(move || {
            if c.swap(true, Ordering::SeqCst) {
                std::process::exit(130);
            }
            eprintln!("\nstopping after the current step (Ctrl+C again to quit now)...");
        });
    }
    let opts = Options { max_depth: args.max_depth };
    let tasks = job::scan(&args.paths);
    if tasks.is_empty() {
        eprintln!("no .psd/.psb files found");
        return ExitCode::from(2);
    }
    // `-o` names a file when there is exactly one input file and the path looks like a PSD/PSB
    // that is not an existing folder; otherwise it is a folder.
    let single_file_out = match &args.output {
        Some(o) => args.paths.len() == 1 && !args.paths.iter().any(|p| p.is_dir()) && !o.is_dir() && is_psd_path(o),
        None => false,
    };
    let rule = OutputRule { folder: args.output.clone(), keep_structure: true, suffix: String::new() };
    let plan_for = |task: &job::Task| Plan {
        dry_run: args.dry_run,
        output: if single_file_out { args.output.clone() } else { rule.resolve(task) },
    };
    let total = tasks.len();
    let (mut ok, mut failed, mut before, mut after) = (0usize, 0usize, 0u64, 0u64);
    for (i, task) in tasks.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let verbose = args.verbose && !args.json;
        let mut on_event = |e: &Event| {
            if !verbose {
                return;
            }
            match e {
                Event::Enter { depth, name } => eprintln!("{}[{name}]", "  ".repeat(*depth)),
                Event::Shared { depth, name } => eprintln!("{}[{name}] (same content as one already cleaned)", "  ".repeat(*depth)),
                Event::Leave { .. } | Event::Progress { .. } => {}
            }
        };
        if !args.json {
            eprintln!("[{}/{total}] {} ...", i + 1, task.path.display());
        }
        let r = job::process(task, &plan_for(task), &opts, Some(&cancel), &mut on_event);
        if args.json {
            println!("{}", serde_json::to_string(&r).unwrap_or_default());
        } else {
            print_report(i + 1, total, &r);
        }
        match r.status {
            Status::Cleaned | Status::AlreadyClean => {
                ok += 1;
                before += r.size_before;
                after += r.size_after.unwrap_or(r.size_before);
            }
            Status::Cancelled => break,
            Status::Missing | Status::Failed => failed += 1,
        }
    }
    if !args.json {
        println!(
            "\ndone: {ok} ok, {failed} failed{}  total {} -> {}",
            if cancel.load(Ordering::SeqCst) { ", stopped" } else { "" },
            human(before),
            human(after)
        );
    }
    if cancel.load(Ordering::SeqCst) {
        ExitCode::from(130)
    } else if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
