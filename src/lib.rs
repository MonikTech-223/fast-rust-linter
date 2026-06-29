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

#[inline(always)]
fn contains(line: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || line.len() < needle.len() { return false; }
    line.windows(needle.len()).any(|w| w == needle)
}

#[inline(always)]
fn find_col(line: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() { return 1; }
    line.windows(needle.len())
        .position(|w| w == needle)
        .unwrap_or(0) + 1
}

#[inline(always)]
fn leading_spaces(line: &[u8]) -> usize {
    line.iter().take_while(|&&b| b == b' ').count()
}

#[inline(always)]
fn leading_tabs(line: &[u8]) -> usize {
    line.iter().take_while(|&&b| b == b'\t').count()
}

#[inline(always)]
fn trim(line: &[u8]) -> &[u8] {
    let start = line.iter().position(|&b| b != b' ' && b != b'\t').unwrap_or(line.len());
    let end = line.iter().rposition(|&b| b != b' ' && b != b'\t' && b != b'\r').map(|i| i + 1).unwrap_or(0);
    if start >= end { b"" } else { &line[start..end] }
}

#[inline(always)]
fn is_py_comment(line: &[u8]) -> bool {
    trim(line).starts_with(b"#")
}

fn is_py(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()) == Some("py")
}

fn is_rust(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()) == Some("rs")
}

fn is_js_ts(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("js") | Some("ts") | Some("jsx") | Some("tsx")
    )
}

pub fn analyze_and_fix(path: &Path, max_line_length: usize, fix: bool) -> Result<(Vec<String>, usize)> {
    let file = File::open(path)?;
    let mmap = unsafe { Mmap::map(&file)? };
    let content = &mmap[..];

    let mut reports: Vec<String> = Vec::new();
    let mut new_content: Vec<u8> = Vec::with_capacity(content.len());
    let mut needs_fix = false;
    let mut fixed_lines = 0;

    let mut lines: Vec<&[u8]> = Vec::new();
    let mut line_s = 0;
    for pos in memchr_iter(b'\n', content) {
        lines.push(&content[line_s..pos]);
        line_s = pos + 1;
    }
    if line_s < content.len() {
        lines.push(&content[line_s..]);
    }

    let mut blank_count = 0usize;
    let mut prev_line: &[u8] = b"";

    for (idx, &line) in lines.iter().enumerate() {
        let line_num = idx + 1;
        let trimmed = trim(line);
        let is_blank = trimmed.is_empty();

        // E501 line too long
        if line.len() > max_line_length {
            reports.push(format!(
                "{} {}:{}:{} [{}] line too long ({} > {})",
                "✖".red(), path.display(), line_num, max_line_length + 1,
                "E501".cyan(), line.len(), max_line_length
            ));
        }

        // W291/W293 trailing whitespace
        let has_trailing = line.last().map(|&b| b == b' ' || b == b'\t').unwrap_or(false);
        if has_trailing {
            let code = if leading_spaces(line) > 0 { "W293" } else { "W291" };
            reports.push(format!(
                "{} {}:{} [{}] trailing whitespace",
                "⚠".yellow(), path.display(), line_num, code.cyan()
            ));
        }

        // W191 tab indentation
        if leading_tabs(line) > 0 && (is_py(path) || is_js_ts(path)) {
            reports.push(format!(
                "{} {}:{} [{}] indentation contains tabs",
                "⚠".yellow(), path.display(), line_num, "W191".cyan()
            ));
        }

        // E302/E303 blank lines (Python)
        if is_py(path) {
            if is_blank {
                blank_count += 1;
            } else {
                if blank_count > 2 {
                    reports.push(format!(
                        "{} {}:{} [{}] too many blank lines ({})",
                        "⚠".yellow(), path.display(), line_num, "E303".cyan(), blank_count
                    ));
                }
                if (trimmed.starts_with(b"def ") || trimmed.starts_with(b"class "))
                    && leading_spaces(line) == 0
                    && blank_count < 2
                    && line_num > 1
                {
                    reports.push(format!(
                        "{} {}:{} [{}] expected 2 blank lines before top-level definition",
                        "⚠".yellow(), path.display(), line_num, "E302".cyan()
                    ));
                }
                blank_count = 0;
            }
        }

        // Python rules
        if is_py(path) && !is_blank && !is_py_comment(line) {

            // E711 comparison to None
            if contains(trimmed, b"== None") {
                reports.push(format!("{} {}:{} [{}] comparison to None (use 'is None')",
                    "⚠".yellow(), path.display(), line_num, "E711".cyan()));
            }
            if contains(trimmed, b"!= None") {
                reports.push(format!("{} {}:{} [{}] comparison to None (use 'is not None')",
                    "⚠".yellow(), path.display(), line_num, "E711".cyan()));
            }

            // E712 comparison to True/False
            if contains(trimmed, b"== True") || contains(trimmed, b"== False") {
                reports.push(format!("{} {}:{} [{}] comparison to True/False (use 'if cond:')",
                    "⚠".yellow(), path.display(), line_num, "E712".cyan()));
            }

            // E721 type() comparison
            if contains(trimmed, b"type(") && contains(trimmed, b") ==") {
                reports.push(format!("{} {}:{} [{}] do not compare types, use isinstance()",
                    "✖".red(), path.display(), line_num, "E721".cyan()));
            }

            // E401 multiple imports
            if trimmed.starts_with(b"import ") && contains(trimmed, b", ") {
                reports.push(format!("{} {}:{} [{}] multiple imports on one line",
                    "⚠".yellow(), path.display(), line_num, "E401".cyan()));
            }

            // F403 wildcard import
            if trimmed.starts_with(b"from ") && contains(trimmed, b"import *") {
                reports.push(format!("{} {}:{} [{}] wildcard import",
                    "⚠".yellow(), path.display(), line_num, "F403".cyan()));
            }

            // E722 bare except
            if contains(trimmed, b"except:") {
                reports.push(format!("{} {}:{} [{}] do not use bare 'except'",
                    "✖".red(), path.display(), line_num, "E722".cyan()));
            }

            // E741 ambiguous variable names
            for amb in [b" l =".as_ref(), b" O =".as_ref(), b" I =".as_ref()] {
                if contains(trimmed, amb) {
                    reports.push(format!("{} {}:{} [{}] ambiguous variable name (l, O, or I)",
                        "⚠".yellow(), path.display(), line_num, "E741".cyan()));
                    break;
                }
            }

            // B006 mutable default argument
            if trimmed.starts_with(b"def ") {
                for mutable in [b"=[]".as_ref(), b"={}".as_ref(), b"=set()".as_ref()] {
                    if contains(trimmed, mutable) {
                        reports.push(format!("{} {}:{} [{}] mutable default argument",
                            "✖".red(), path.display(), line_num, "B006".cyan()));
                        break;
                    }
                }
            }

            // S105 hardcoded password
            let line_lower = line.to_ascii_lowercase();
            for kw in [b"password=".as_ref(), b"secret=".as_ref(), b"api_key=".as_ref(), b"passwd=".as_ref()] {
                if contains(&line_lower, kw) {
                    let col = find_col(&line_lower, kw);
                    if col < line.len() {
                        let after = &line[col + kw.len() - 1..];
                      let ta = trim(after);
                        if ta.starts_with(b"\"") || ta.starts_with(b"'") {
                            reports.push(format!("{} {}:{} [{}] hardcoded password/secret",
                                "✖".red(), path.display(), line_num, "S105".cyan()));
                            break;
                        }
                    }
                }
            }

            // S605 os.system
            if contains(trimmed, b"os.system(") {
                reports.push(format!("{} {}:{} [{}] os.system() — use subprocess instead",
                    "✖".red(), path.display(), line_num, "S605".cyan()));
            }

            // W605 invalid escape sequences
            for seq in [b"\\a".as_ref(), b"\\b".as_ref(), b"\\d".as_ref(), b"\\e".as_ref(),
                        b"\\g".as_ref(), b"\\h".as_ref(), b"\\j".as_ref(), b"\\k".as_ref(),
                        b"\\m".as_ref(), b"\\o".as_ref(), b"\\p".as_ref(), b"\\q".as_ref(),
                        b"\\w".as_ref(), b"\\y".as_ref(), b"\\z".as_ref()] {
                if contains(trimmed, seq) && !contains(trimmed, b"r\"") && !contains(trimmed, b"r'") {
                    reports.push(format!("{} {}:{} [{}] invalid escape sequence",
                        "⚠".yellow(), path.display(), line_num, "W605".cyan()));
                    break;
                }
            }

            // AIR001 deprecated OpenAI API
            if (contains(trimmed, b"openai.Completion") || contains(trimmed, b"openai.ChatCompletion")) {
                reports.push(format!("{} {}:{} [{}] deprecated OpenAI API (use openai>=1.0 style)",
                    "⚠".yellow(), path.display(), line_num, "AIR001".cyan()));
            }

            // S106 leaked API key
            if (contains(trimmed, b"sk-") || contains(trimmed, b"AIza") || contains(trimmed, b"AKIA"))
                && (contains(trimmed, b"\"") || contains(trimmed, b"'"))
            {
                reports.push(format!("{} {}:{} [{}] possible leaked API key",
                    "✖".red(), path.display(), line_num, "S106".cyan()));
            }
        }

        // Rust rules
        if is_rust(path) && !is_blank {
            if contains(trimmed, b".unwrap()") && !contains(prev_line, b"// SAFETY") {
                reports.push(format!("{} {}:{} [{}] .unwrap() — consider '?' or expect()",
                    "⚠".yellow(), path.display(), line_num, "R001".cyan()));
            }
            if contains(trimmed, b".clone()") {
                reports.push(format!("{} {}:{} [{}] .clone() — ensure this is necessary",
                    "⚠".yellow(), path.display(), line_num, "R002".cyan()));
            }
            if contains(trimmed, b"TODO") || contains(trimmed, b"FIXME") || contains(trimmed, b"HACK") {
                reports.push(format!("{} {}:{} [{}] unresolved TODO/FIXME/HACK",
                    "⚠".yellow(), path.display(), line_num, "R003".cyan()));
            }
            if contains(trimmed, b"panic!(") {
                reports.push(format!("{} {}:{} [{}] panic!() — prefer Result/Option",
                    "⚠".yellow(), path.display(), line_num, "R004".cyan()));
            }
            if (contains(trimmed, b"sk-") || contains(trimmed, b"AIza") || contains(trimmed, b"AKIA"))
                && (contains(trimmed, b"\"") || contains(trimmed, b"'"))
            {
                reports.push(format!("{} {}:{} [{}] possible leaked API key",
                    "✖".red(), path.display(), line_num, "S106".cyan()));
            }
        }

        // JS/TS rules
        if is_js_ts(path) && !is_blank {
            if trimmed.starts_with(b"var ") {
                reports.push(format!("{} {}:{} [{}] use 'const' or 'let' instead of 'var'",
                    "⚠".yellow(), path.display(), line_num, "NO-VAR".cyan()));
            }
            if contains(trimmed, b"console.log(") || contains(trimmed, b"console.error(") {
                reports.push(format!("{} {}:{} [{}] unexpected console statement",
                    "⚠".yellow(), path.display(), line_num, "NO-CONSOLE".cyan()));
            }
            if contains(trimmed, b" == ") && !contains(trimmed, b" === ") && !contains(trimmed, b"!==") {
                reports.push(format!("{} {}:{} [{}] use '===' instead of '=='",
                    "⚠".yellow(), path.display(), line_num, "EQEQEQ".cyan()));
            }
            if contains(trimmed, b"TODO") || contains(trimmed, b"FIXME") {
                reports.push(format!("{} {}:{} [{}] unresolved TODO/FIXME",
                    "⚠".yellow(), path.display(), line_num, "NO-TODO".cyan()));
            }
            if (contains(trimmed, b"sk-") || contains(trimmed, b"AIza") || contains(trimmed, b"AKIA"))
                && (contains(trimmed, b"\"") || contains(trimmed, b"'") || contains(trimmed, b"`"))
            {
                reports.push(format!("{} {}:{} [{}] possible leaked API key",
                    "✖".red(), path.display(), line_num, "S106".cyan()));
            }
        }

        // Fix phase
        if fix {
            let mut fixed_line: Vec<u8> = line.to_vec();
            let mut line_fixed = false;

            // Remove trailing whitespace
            if has_trailing {
                while fixed_line.last().map(|&b| b == b' ' || b == b'\t').unwrap_or(false) {
                    fixed_line.pop();
                }
                line_fixed = true;
            }

            // Python: == None -> is None
            if is_py(path) {
                if let Some(pos) = fixed_line.windows(7).position(|w| w == b"== None") {
                    fixed_line[pos..pos+7].copy_from_slice(b"is None");
                    line_fixed = true;
                }
                if let Some(pos) = fixed_line.windows(7).position(|w| w == b"!= None") {
                    let mut new_line = fixed_line[..pos].to_vec();
                    new_line.extend_from_slice(b"is not None");
                    new_line.extend_from_slice(&fixed_line[pos+7..]);
                    fixed_line = new_line;
                    line_fixed = true;
                }
            }

            // JS: var -> let
            if is_js_ts(path) && fixed_line.starts_with(b"var ") {
                fixed_line[0..4].copy_from_slice(b"let ");
                line_fixed = true;
            }

            if line_fixed {
                fixed_lines += 1;
                needs_fix = true;
            }
            new_content.extend_from_slice(&fixed_line);
        } else {
            new_content.extend_from_slice(line);
        }

        new_content.push(b'\n');
        prev_line = lines[idx];
    }

    if !content.ends_with(b"\n") && new_content.ends_with(b"\n") {
        new_content.pop();
    }

    if fix && needs_fix {
        let parent = path.parent().unwrap_or(Path::new("."));
        let mut tmp = NamedTempFile::new_in(parent)?;
        tmp.write_all(&new_content)?;
        tmp.flush()?;
        tmp.persist(path)?;
    }

    Ok((reports, fixed_lines))
}
