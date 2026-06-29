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

#[derive(Debug, Clone)]
pub struct LintReport {
    pub code: &'static str,
    pub message: String,
    pub line: usize,
    pub col: usize,
    pub fixable: bool,
}

impl LintReport {
    fn format(&self, path: &Path) -> String {
        format!(
            "{} {}:{}:{} [{}] {}",
            if self.fixable { "⚠".yellow() } else { "✖".red() },
            path.display(),
            self.line,
            self.col,
            self.code.cyan(),
            self.message
        )
    }
}

/// Быстрая проверка: есть ли подстрока в строке (без аллокаций)
#[inline(always)]
fn contains(line: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || line.len() < needle.len() { return false; }
    line.windows(needle.len()).any(|w| w == needle)
}

/// Индекс первого вхождения подстроки
#[inline(always)]
fn find_col(line: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() { return 0; }
    line.windows(needle.len())
        .position(|w| w == needle)
        .unwrap_or(0) + 1
}

/// Количество ведущих пробелов
#[inline(always)]
fn leading_spaces(line: &[u8]) -> usize {
    line.iter().take_while(|&&b| b == b' ').count()
}

/// Количество ведущих табов
#[inline(always)]
fn leading_tabs(line: &[u8]) -> usize {
    line.iter().take_while(|&&b| b == b'\t').count()
}

/// Строка без ведущих/хвостовых пробелов (без аллокаций — просто срез)
#[inline(always)]
fn trim(line: &[u8]) -> &[u8] {
    let start = line.iter().position(|&b| b != b' ' && b != b'\t').unwrap_or(line.len());
    let end = line.iter().rposition(|&b| b != b' ' && b != b'\t' && b != b'\r').map(|i| i + 1).unwrap_or(0);
    if start >= end { b"" } else { &line[start..end] }
}

/// Проверяем что строка — комментарий Python
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

    let mut reports: Vec<LintReport> = Vec::new();
    let mut new_content: Vec<u8> = Vec::with_capacity(content.len());
    let mut needs_fix = false;
    let mut fixed_lines = 0;

    // Состояние для многострочных проверок
    let mut prev_line: &[u8] = b"";
    let mut prev_blank = false;
    let mut blank_count = 0usize;
    let mut line_num = 0usize;
    let mut line_start = 0usize;

    // Первый проход — собираем строки
    let mut lines: Vec<&[u8]> = Vec::new();
    let mut line_s = 0;
    for pos in memchr_iter(b'\n', content) {
        lines.push(&content[line_s..pos]);
        line_s = pos + 1;
    }
    if line_s < content.len() {
        lines.push(&content[line_s..]);
    }

    let total_lines = lines.len();

    for (idx, &line) in lines.iter().enumerate() {
        line_num = idx + 1;
        let trimmed = trim(line);
        let is_blank = trimmed.is_empty();

        // ── E501 Длина строки ──────────────────────────────────────────
        if line.len() > max_line_length {
            reports.push(LintReport {
                code: "E501",
                message: format!("line too long ({} > {})", line.len(), max_line_length),
                line: line_num, col: max_line_length + 1, fixable: false,
            });
        }

        // ── W291/W293 Trailing whitespace ─────────────────────────────
        let has_trailing = line.last().map(|&b| b == b' ' || b == b'\t').unwrap_or(false)
            && !line.ends_with(b"\r");
        if has_trailing {
            let code = if leading_spaces(line) > 0 || leading_tabs(line) > 0 {
                "W293"
            } else {
                "W291"
            };
            reports.push(LintReport {
                code,
                message: "trailing whitespace".to_string(),
                line: line_num,
                col: line.iter().rposition(|&b| b != b' ' && b != b'\t').map(|i| i + 2).unwrap_or(1),
                fixable: true,
            });
        }

        // ── W191 Tab indentation ───────────────────────────────────────
        if leading_tabs(line) > 0 && (is_py(path) || is_js_ts(path)) {
            reports.push(LintReport {
                code: "W191",
                message: "indentation contains tabs".to_string(),
                line: line_num, col: 1, fixable: false,
            });
        }

        // ── E302/E303 Blank lines (Python) ─────────────────────────────
        if is_py(path) {
            if is_blank {
                blank_count += 1;
            } else {
                if blank_count > 2 {
                    reports.push(LintReport {
                        code: "E303",
                        message: format!("too many blank lines ({})", blank_count),
                        line: line_num, col: 1, fixable: false,
                    });
                }
                // E302: два пустых перед def/class на верхнем уровне
                if (trimmed.starts_with(b"def ") || trimmed.starts_with(b"class "))
                    && leading_spaces(line) == 0
                    && blank_count < 2
                    && line_num > 1
                {
                    reports.push(LintReport {
                        code: "E302",
                        message: "expected 2 blank lines before top-level definition".to_string(),
                        line: line_num, col: 1, fixable: false,
                    });
                }
                blank_count = 0;
            }
        }

        // ──────────────── Python-специфичные правила ───────────────────
        if is_py(path) && !is_blank && !is_py_comment(line) {

            // E711 Comparison to None (== None / != None)
            if contains(trimmed, b"== None") {
                reports.push(LintReport {
                    code: "E711",
                    message: "comparison to None (use 'is None')".to_string(),
                    line: line_num, col: find_col(line, b"== None"), fixable: true,
                });
            }
            if contains(trimmed, b"!= None") {
                reports.push(LintReport {
                    code: "E711",
                    message: "comparison to None (use 'is not None')".to_string(),
                    line: line_num, col: find_col(line, b"!= None"), fixable: true,
                });
            }

            // E712 Comparison to True/False
            if contains(trimmed, b"== True") || contains(trimmed, b"== False") {
                reports.push(LintReport {
                    code: "E712",
                    message: "comparison to True/False (use 'if cond:' or 'if not cond:')".to_string(),
                    line: line_num, col: 1, fixable: false,
                });
            }

            // E721 type() comparison (use isinstance)
            if contains(trimmed, b"type(") && contains(trimmed, b") ==") {
                reports.push(LintReport {
                    code: "E721",
                    message: "do not compare types, use isinstance()".to_string(),
                    line: line_num, col: find_col(line, b"type("), fixable: false,
                });
            }

            // W605 Invalid escape sequence
            for seq in [b"\\a", b"\\b", b"\\c", b"\\d", b"\\e", b"\\g",
                        b"\\h", b"\\i", b"\\j", b"\\k", b"\\l", b"\\m",
                        b"\\o", b"\\p", b"\\q", b"\\w", b"\\y", b"\\z"].iter() {
                if contains(trimmed, seq) && !contains(trimmed, b"r\"") && !contains(trimmed, b"r'") {
                    reports.push(LintReport {
                        code: "W605",
                        message: format!("invalid escape sequence '{}'", std::str::from_utf8(seq).unwrap_or("?")),
                        line: line_num, col: find_col(line, seq), fixable: false,
                    });
                    break;
                }
            }

            // E401 Multiple imports on one line
            if trimmed.starts_with(b"import ") && contains(trimmed, b", ") && !contains(trimmed, b"from ") {
                reports.push(LintReport {
                    code: "E401",
                    message: "multiple imports on one line".to_string(),
                    line: line_num, col: 1, fixable: false,
                });
            }

            // F401 Unused import placeholder (wildcard import)
            if trimmed.starts_with(b"from ") && contains(trimmed, b"import *") {
                reports.push(LintReport {
                    code: "F403",
                    message: "wildcard import — unable to detect undefined names".to_string(),
                    line: line_num, col: 1, fixable: false,
                });
            }

            // E722 Bare except
            if contains(trimmed, b"except:") {
                reports.push(LintReport {
                    code: "E722",
                    message: "do not use bare 'except'".to_string(),
                    line: line_num, col: find_col(line, b"except:"), fixable: false,
                });
            }

            // B006 Mutable default argument
            for mutable in [b"=[]".as_ref(), b"={}".as_ref(), b"=set()".as_ref()].iter() {
                if contains(trimmed, mutable) && trimmed.starts_with(b"def ") {
                    reports.push(LintReport {
                        code: "B006",
                        message: "mutable default argument".to_string(),
                        line: line_num, col: 1, fixable: false,
                    });
                    break;
                }
            }

            // S105 Hardcoded password/secret
            for kw in [b"password=".as_ref(), b"secret=".as_ref(), b"api_key=".as_ref(), b"passwd=".as_ref()].iter() {
                if contains(&line.to_ascii_lowercase(), kw) {
                    let after = &line[find_col(line, kw) + kw.len() - 1..];
                    let trimmed_after = trim(after);
                    if trimmed_after.starts_with(b"\"") || trimmed_after.starts_with(b"'") {
                        reports.push(LintReport {
                            code: "S105",
                            message: "hardcoded password/secret detected".to_string(),
                            line: line_num, col: find_col(line, kw), fixable: false,
                        });
                        break;
                    }
                }
            }

            // S106 os.system() call
            if contains(trimmed, b"os.system(") {
                reports.push(LintReport {
                    code: "S605",
                    message: "os.system() call — use subprocess instead".to_string(),
                    line: line_num, col: find_col(line, b"os.system("), fixable: false,
                });
            }

            // E711-подобное: print statement (Python 2 style)
            if trimmed.starts_with(b"print ") && !contains(trimmed, b"print(") {
                reports.push(LintReport {
                    code: "UP001",
                    message: "print statement (Python 2 style), use print()".to_string(),
                    line: line_num, col: 1, fixable: true,
                });
            }

            // E741 Ambiguous variable names
            for amb in [b" l =", b" O =", b" I =", b"(l,", b"(O,", b"(I,"].iter() {
                if contains(trimmed, amb) {
                    reports.push(LintReport {
                        code: "E741",
                        message: "ambiguous variable name (l, O, or I)".to_string(),
                        line: line_num, col: 1, fixable: false,
                    });
                    break;
                }
            }

            // AIR001 Deprecated OpenAI import pattern
            if contains(trimmed, b"import openai") || contains(trimmed, b"from openai") {
                // Проверяем старый API стиль
                if contains(trimmed, b"openai.Completion") || contains(trimmed, b"openai.ChatCompletion") {
                    reports.push(LintReport {
                        code: "AIR001",
                        message: "deprecated OpenAI API call (use openai>=1.0 client style)".to_string(),
                        line: line_num, col: 1, fixable: false,
                    });
                }
            }

            // S106 Leaked API key pattern (sk-...)
            if (contains(trimmed, b"sk-") || contains(trimmed, b"AIza") || contains(trimmed, b"AKIA"))
                && (contains(trimmed, b"\"") || contains(trimmed, b"'"))
            {
                reports.push(LintReport {
                    code: "S106",
                    message: "possible leaked API key in source code".to_string(),
                    line: line_num, col: 1, fixable: false,
                });
            }
        }

        // ──────────────── Rust-специфичные правила ─────────────────────
        if is_rust(path) && !is_blank {
            let t = trimmed;

            // R001 unwrap() без комментария
            if contains(t, b".unwrap()") && !contains(prev_line, b"// SAFETY") && !contains(prev_line, b"// unwrap") {
                reports.push(LintReport {
                    code: "R001",
                    message: ".unwrap() — consider using '?' or expect()".to_string(),
                    line: line_num, col: find_col(line, b".unwrap()"), fixable: false,
                });
            }

            // R002 clone() на больших типах — просто предупреждаем о частом clone
            if contains(t, b".clone()") {
                reports.push(LintReport {
                    code: "R002",
                    message: ".clone() — ensure this is necessary".to_string(),
                    line: line_num, col: find_col(line, b".clone()"), fixable: false,
                });
            }

            // R003 TODO/FIXME/HACK комментарии
            if contains(t, b"TODO") || contains(t, b"FIXME") || contains(t, b"HACK") {
                reports.push(LintReport {
                    code: "R003",
                    message: "unresolved TODO/FIXME/HACK comment".to_string(),
                    line: line_num, col: 1, fixable: false,
                });
            }

            // R004 panic!() вне тестов
            if contains(t, b"panic!(") && !contains(t, b"#[test]") {
                reports.push(LintReport {
                    code: "R004",
                    message: "panic!() call — prefer returning Result/Option".to_string(),
                    line: line_num, col: find_col(line, b"panic!("), fixable: false,
                });
            }

            // R005 println! в библиотечном коде (не в main/test)
            if contains(t, b"println!(") {
                reports.push(LintReport {
                    code: "R005",
                    message: "println!() — consider using a proper logger".to_string(),
                    line: line_num, col: find_col(line, b"println!("), fixable: false,
                });
            }

            // S106 Leaked key
            if (contains(t, b"sk-") || contains(t, b"AIza") || contains(t, b"AKIA"))
                && (contains(t, b"\"") || contains(t, b"'"))
            {
                reports.push(LintReport {
                    code: "S106",
                    message: "possible leaked API key in source code".to_string(),
                    line: line_num, col: 1, fixable: false,
                });
            }
        }

        // ──────────────── JS/TS правила ────────────────────────────────
        if is_js_ts(path) && !is_blank {
            let t = trimmed;

            // no-var
            if t.starts_with(b"var ") {
                reports.push(LintReport {
                    code: "NO-VAR",
                    message: "use 'const' or 'let' instead of 'var'".to_string(),
                    line: line_num, col: 1, fixable: true,
                });
            }

            // no-console
            if contains(t, b"console.log(") || contains(t, b"console.error(") {
                reports.push(LintReport {
                    code: "NO-CONSOLE",
                    message: "unexpected console statement".to_string(),
                    line: line_num, col: find_col(line, b"console."), fixable: false,
                });
            }

            // eqeqeq (== вместо ===)
            if contains(t, b" == ") && !contains(t, b" === ") && !contains(t, b" !== ") {
                reports.push(LintReport {
                    code: "EQEQEQ",
                    message: "use '===' instead of '=='".to_string(),
                    line: line_num, col: find_col(line, b" == "), fixable: true,
                });
            }

            // TODO/FIXME
            if contains(t, b"TODO") || contains(t, b"FIXME") {
                reports.push(LintReport {
                    code: "NO-TODO",
                    message: "unresolved TODO/FIXME comment".to_string(),
                    line: line_num, col: 1, fixable: false,
                });
            }

            // S106 Leaked key
            if (contains(t, b"sk-") || contains(t, b"AIza") || contains(t, b"AKIA"))
                && (contains(t, b"\"") || contains(t, b"'") || contains(t, b"`"))
            {
                reports.push(LintReport {
                    code: "S106",
                    message: "possible leaked API key in source code".to_string(),
                    line: line_num, col: 1, fixable: false,
                });
            }
        }

        // ── Фиксы ──────────────────────────────────────────────────────
        if fix {
            let mut fixed_line: Vec<u8> = line.to_vec();
            let mut line_fixed = false;

            // Убираем trailing whitespace
            if has_trailing {
                while fixed_line.last().map(|&b| b == b' ' || b == b'\t').unwrap_or(false) {
                    fixed_line.pop();
                }
                line_fixed = true;
            }

            // Python: == None → is None
            if is_py(path) {
                if let Some(pos) = fixed_line.windows(7).position(|w| w == b"== None") {
                    fixed_line[pos..pos+7].copy_from_slice(b"is None");
                    line_fixed = true;
                }
                if let Some(pos) = fixed_line.windows(7).position(|w| w == b"!= None") {
                    let replacement = b"is not None";
                    let mut new_line = fixed_line[..pos].to_vec();
                    new_line.extend_from_slice(replacement);
                    new_line.extend_from_slice(&fixed_line[pos+7..]);
                    fixed_line = new_line;
                    line_fixed = true;
                }
            }

            // JS: var → let
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
        prev_blank = is_blank;
        line_start = line_start; // unused but kept for clarity
    }

    // Убираем лишний \n в конце если его не было
    if !content.ends_with(b"\n") && new_content.ends_with(b"\n") {
        new_content.pop();
    }

    // Атомарная перезапись
    if fix && needs_fix {
        let parent = path.parent().unwrap_or(Path::new("."));
        let mut tmp = NamedTempFile::new_in(parent)?;
        tmp.write_all(&new_content)?;
        tmp.flush()?;
        tmp.persist(path)?;
    }

    let formatted: Vec<String> = reports.iter().map(|r| r.format(path)).collect();
    Ok((formatted, fixed_lines))
}
