use anyhow::Result;
use memchr::memchr_iter;
use memmap2::Mmap;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use colored::*;
use tempfile::NamedTempFile;

pub fn is_target_file(path: &Path) -> bool {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    matches!(ext, "py" | "rs" | "js" | "ts" | "jsx" | "tsx" | "go" | "cpp" | "c" | "cc" | "h" | "java" | "cs")
}

pub fn analyze_and_fix(path: &Path, max_line_length: usize, fix: bool) -> Result<(Vec<String>, usize)> {
    let file = File::open(path)?;
    let mmap = unsafe { Mmap::map(&file)? };
    let content = &mmap[..];

    let mut reports = Vec::new();
    let mut fixed_lines = 0;
    let mut new_content: Vec<u8> = Vec::with_capacity(content.len());
    let mut needs_fix = false;

    let mut line_start = 0;
    let mut line_num = 1;

    for pos in memchr_iter(b'\n', content) {
        let line = &content[line_start..pos];

        if line.len() > max_line_length {
            reports.push(format!(
                "{} {}:{} — line too long ({} > {})",
                "✖".red(),
                path.display(),
                line_num,
                line.len(),
                max_line_length
            ));

            if fix {
                // Считаем отступ (пробелы/табы в начале строки)
                let indent_len = line.iter().take_while(|&&b| b == b' ' || b == b'\t').count();
                let indent = &line[..indent_len];

                // Разбиваем строку на куски по max_line_length с учётом отступа
                let mut chunk_start = 0;
                let text = &line[indent_len..];
                let effective_max = max_line_length.saturating_sub(indent_len);

                while chunk_start < text.len() {
                    let chunk_end = (chunk_start + effective_max).min(text.len());
                    new_content.extend_from_slice(indent);
                    new_content.extend_from_slice(&text[chunk_start..chunk_end]);
                    new_content.push(b'\n');
                    chunk_start = chunk_end;
                }
                fixed_lines += 1;
                needs_fix = true;
            } else {
                new_content.extend_from_slice(line);
                new_content.push(b'\n');
            }
        } else {
            new_content.extend_from_slice(line);
            new_content.push(b'\n');
        }

        line_start = pos + 1;
        line_num += 1;
    }

    // Последняя строка без \n
    if line_start < content.len() {
        let line = &content[line_start..];
        new_content.extend_from_slice(line);
    }

    // Перезаписываем файл через NamedTempFile — атомарно
    if fix && needs_fix {
        let parent = path.parent().unwrap_or(Path::new("."));
        let mut tmp = NamedTempFile::new_in(parent)?;
        tmp.write_all(&new_content)?;
        tmp.flush()?;
        tmp.persist(path)?;
    }

    Ok((reports, fixed_lines))
}
