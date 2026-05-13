use anyhow::Result;
use bstr::ByteSlice;
use memchr::memchr_iter;
use memmap2::Mmap;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use colored::*;
use tempfile::NamedTempFile;

// ГЛАВНАЯ ФУНКЦИЯ АНАЛИЗА
pub fn analyze_and_fix(path: &Path, max_line_len: usize, should_fix: bool) -> Result<(Vec<String>, usize)> {
    let file = File::open(path)?;
    let mmap = unsafe { Mmap::map(&file)? };
    let content = &mmap[..];

    let mut reports = Vec::new();
    let mut fixed_lines = 0usize;
    let mut new_lines = Vec::with_capacity(512);

    let is_python = path.extension().and_then(|s| s.to_str()) == Some("py");
    let comment_byte = if is_python { b'#' } else { b'/' };

    let mut line_start = 0;
    let mut line_num = 1;

    for pos in memchr_iter(b'\n', content) {
        let line = &content[line_start..pos];
        process_line(line, line_num, path, max_line_len, comment_byte, &mut reports, &mut new_lines, should_fix, &mut fixed_lines);
        line_start = pos + 1;
        line_num += 1;
    }

    if line_start < content.len() {
        let line = &content[line_start..];
        process_line(line, line_num, path, max_line_len, comment_byte, &mut reports, &mut new_lines, should_fix, &mut fixed_lines);
    }

    if should_fix && fixed_lines > 0 {
        let mut new_content = new_lines.join_vec(b"\n");
        if content.ends_with(b"\n") {
            new_content.push(b'\n');
        }

        let mut temp = NamedTempFile::new_in(path.parent().unwrap_or_else(|| Path::new(".")))?;
        temp.write_all(&new_content)?;
        temp.persist(path)?;
    }

    Ok((reports, if should_fix && fixed_lines > 0 { 1 } else { 0 }))
}

#[inline(always)]
fn process_line(
    line: &[u8],
    line_num: usize,
    path: &Path,
    max_line_len: usize,
    comment_byte: u8,
    reports: &mut Vec<String>,
    new_lines: &mut Vec<Vec<u8>>,
    should_fix: bool,
    fixed_lines: &mut usize,
) {
    if line.len() > max_line_len {
        reports.push(format!(
            "{}:{} {} ({} chars)",
            path.display().to_string().dimmed(),
            line_num,
            "Long line".yellow(),
            line.len()
        ));
    }

    if line.trim_ascii_start().first() == Some(&comment_byte) {
        new_lines.push(line.to_vec());
        return;
    }

    let mut new_line = line.to_vec();

    if contains_debug_output(line) {
        reports.push(format!(
            "{}:{} {} → commented",
            path.display().to_string().dimmed(),
            line_num,
            "Debug output".blue()
        ));

        if should_fix {
            let mut commented = vec![comment_byte, b' '];
            commented.extend_from_slice(line);
            new_line = commented;
            *fixed_lines += 1;
        }
    }

    if contains_potential_secret(line) {
        reports.push(format!(
            "{}:{} {}",
            path.display().to_string().dimmed(),
            line_num,
            "Potential secret detected!".red().bold()
        ));
    }

    new_lines.push(new_line);
}

#[inline]
pub fn contains_debug_output(line: &[u8]) -> bool {
    let l = line.to_ascii_lowercase();
    (l.contains_str("print(") && !l.contains_str("fingerprint") && !l.contains_str("blueprint"))
        || l.contains_str("println!")
        || l.contains_str("dbg!")
        || l.contains_str("console.log")
        || l.contains_str("console.debug")
}

#[inline]
pub fn contains_potential_secret(line: &[u8]) -> bool {
    let l = line.to_ascii_lowercase();
    (l.contains_str("sk-") && !l.contains_str("task-") && !l.contains_str("ask-"))
        || l.contains_str("akia")
        || (line.contains_str(b"-----BEGIN") && (line.contains_str(b"PRIVATE KEY") || line.contains_str(b"RSA")))
  }
