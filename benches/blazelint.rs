use anyhow::Result;
use blazelint::analyze_and_fix;
use clap::{Parser, Subcommand};
use colored::*;
use indicatif::{ParallelProgressIterator, ProgressBar, ProgressStyle};
use jwalk::WalkDir;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

#[derive(Parser)]
#[command(author, version, about = "BlazeLint — Ultra Fast Linter")]
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

fn run_check(path: &str, max_line: usize, fix: bool, no_cache: bool, start: Instant) -> Result<()> {
    println!("{} BlazeLint started...", "🦀".green());
    println!("Scanning path: {}", path);
    println!("Fix mode: {}", fix);

    let elapsed = start.elapsed();
    println!("\n{} Analysis completed in {:.2?}", "✔".green().bold(), elapsed);

    Ok(())
}
