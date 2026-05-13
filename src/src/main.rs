use anyhow::Result;
use blazelint::analyze_and_fix;
use clap::{Parser, Subcommand};
use colored::*;
use indicatif::{ParallelProgressIterator, ProgressBar, ProgressStyle};
use jwalk::WalkDir;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

#[derive(Parser)]
#[command(author, version, about = "BlazeLint — Zero-Copy SIMD Linter")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Check {
        #[arg(default_value = ".")]
        path: String,
        #[arg(short, long, default_value_t = 100)]
        max_line: usize,
        #[arg(long)]
        fix: bool,
        #[arg(long)]
        no_cache: bool,
    },
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct FileCache {
    mtime: SystemTime,
    size: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct BlazeCache {
    files: HashMap<String, FileCache>,
}

fn main() -> Result<()> {
    let start = Instant::now();
    let cli = Cli::parse();
    match cli.command {
        Commands::Check { path, max_line, fix, no_cache } => {
            run_check(&path, max_line, fix, no_cache, start)?
        }
    }
    Ok(())
}

fn is_target_file(path: &Path) -> bool {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    matches!(ext, "py" | "rs" | "js" | "ts" | "jsx" | "tsx" | "go" | "cpp" | "c" | "cc" | "h" | "java" | "cs")
}

fn load_cache() -> BlazeCache {
    std::fs::read_to_string(".blazelint-cache.json")
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_cache(cache: &BlazeCache) -> Result<()> {
    std::fs::write(".blazelint-cache.json", serde_json::to_string_pretty(cache)?)?;
    Ok(())
}

fn run_check(path: &str, max_line: usize, fix: bool, no_cache: bool, start: Instant) -> Result<()> {
    let cache = if no_cache { BlazeCache::default() } else { load_cache() };
    let new_cache = Arc::new(Mutex::new(BlazeCache::default()));

    let files: Vec<PathBuf> = WalkDir::new(path)
        .skip_hidden(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && is_target_file(e.path()))
        .map(|e| e.into_path())
        .collect();

    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
        .unwrap()
        .progress_chars("█▓░"));

    let all_reports = Arc::new(Mutex::new(Vec::new()));
    let fixed_count = Arc::new(Mutex::new(0usize));

    files.par_iter().progress_with(pb).for_each(|file_path| {
        let metadata = match file_path.metadata() {
            Ok(m) => m,
            Err(_) => return,
        };
        let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let size = metadata.len();
        let path_str = file_path.to_string_lossy().to_string();

        if !no_cache {
            if let Some(cached) = cache.files.get(&path_str) {
                if cached.mtime == mtime && cached.size == size {
                    new_cache.lock().unwrap().files.insert(path_str, FileCache { mtime, size });
                    return;
                }
            }
        }

        if let Ok((reports, fixed)) = analyze_and_fix(file_path, max_line, fix) {
            if !reports.is_empty() {
                all_reports.lock().unwrap().extend(reports);
            }
            if fixed > 0 {
                *fixed_count.lock().unwrap() += 1;
            }
        }
        new_cache.lock().unwrap().files.insert(path_str, FileCache { mtime, size });
    });

    let reports = all_reports.lock().unwrap();
    for report in reports.iter() { println!("{}", report); }

    let elapsed = start.elapsed();
    println!("\n{}", "═".repeat(60).cyan());
    println!("{} Analysis finished in: {:.2?}", "✔".green().bold(), elapsed);
    if fix && *fixed_count.lock().unwrap() > 0 {
        println!("{} Files fixed: {}", "🔧".cyan(), fixed_count.lock().unwrap());
    }
    println!("{} Files processed: {}", "📊".blue(), files.len());
    println!("{}", "═".repeat(60).cyan());

    let final_cache = Arc::try_unwrap(new_cache).unwrap().into_inner().unwrap();
    let _ = save_cache(&final_cache);
    Ok(())
}
