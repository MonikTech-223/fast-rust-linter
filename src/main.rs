use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::*;
use jwalk::WalkDir;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};
use tempfile::NamedTempFile;

#[derive(Parser)]
#[command(author, version, about = "BlazeLint — Ultra Fast Security & Style Linter")]
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
    },
}

fn main() -> Result<()> {
    let start = Instant::now();
    let cli = Cli::parse();

    match cli.command {
        Commands::Check { path, max_line, fix } => {
            run_check(&path, max_line, fix, start)?;
        }
    }
    Ok(())
}

fn run_check(path: &str, max_line: usize, fix: bool, start: Instant) -> Result<()> {
    let files: Vec<PathBuf> = WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .collect();

    let all_reports = Arc::new(Mutex::new(Vec::new()));
    let fixed_count = Arc::new(Mutex::new(0usize));

    files.par_iter().for_each(|file_path| {
        if let Ok((reports, fixed)) = analyze_and_fix(file_path, max_line, fix) {
            if !reports.is_empty() {
                all_reports.lock().unwrap().extend(reports);
            }
            if fixed > 0 {
                *fixed_count.lock().unwrap() += 1;
            }
        }
    });

    let reports = all_reports.lock().unwrap();
    for report in reports.iter() {
        println!("{}", report);
    }

    let elapsed = start.elapsed();
    println!("\n{} Analysis finished in: {:.2?}", "✔".green(), elapsed);
    if fix {
        println!("{} Files fixed: {}", "🔧".cyan(), fixed_count.lock().unwrap());
    }

    Ok(())
}

fn analyze_and_fix(path: &Path, max_line_len: usize, should_fix: bool) -> Result<(Vec<String>, usize)> {
    let content = fs::read_to_string(path)?;
    let lines: Vec<&str> = content.lines().collect();
    let mut reports = Vec::new();
    let mut fixed_lines = 0usize;
    let mut new_lines = Vec::with_capacity(lines.len());

    for (i, line) in lines.iter().enumerate() {
        let line_num = i + 1;
        let trimmed = line.trim_start();

        if line.len() > max_line_len {
            reports.push(format!("{}:{} {} ({} chars)", path.display().to_string().dimmed(), line_num, "Long line".yellow(), line.len()));
        }

        if trimmed.starts_with('#') || trimmed.starts_with("//") {
            new_lines.push(line.to_string());
            continue;
        }

        let lower = line.to_lowercase();
        let mut new_line = line.to_string();

        // FIX LOGIC
        if lower.contains("print(") || lower.contains("println!") || lower.contains("dbg!") || lower.contains("console.log") {
            reports.push(format!("{}:{} {} -> commented out", path.display().to_string().dimmed(), line_num, "Debug output found".blue()));
            if should_fix {
                new_line = if path.extension().and_then(|s| s.to_str()) == Some("py") {
                    format!("# {}", line)
                } else {
                    format!("// {}", line)
                };
                fixed_lines += 1;
            }
        }

        if lower.contains("sk-") || lower.contains("akia") {
            reports.push(format!("{}:{} {}", path.display().to_string().dimmed(), line_num, "Potential Secret/API Key found!".red().bold()));
        }

        if lower.contains("openai.chatcompletion") || lower.contains("langchain.llms") {
            reports.push(format!("{}:{} {}", path.display().to_string().dimmed(), line_num, "Deprecated API usage (OpenAI/LangChain)".magenta()));
        }

        new_lines.push(new_line);
    }

    if should_fix && fixed_lines > 0 {
        let new_content = new_lines.join("\n") + if content.ends_with('\n') { "\n" } else { "" };
        
        // Atomic write via tempfile
        let mut temp_file = NamedTempFile::new_in(path.parent().unwrap_or_else(|| Path::new(".")))?;
        temp_file.write_all(new_content.as_bytes())?;
        temp_file.persist(path)?;
    }

    Ok((reports, if should_fix && fixed_lines > 0 { 1 } else { 0 }))
  }
