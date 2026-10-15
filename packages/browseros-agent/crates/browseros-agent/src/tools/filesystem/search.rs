//! `filesystem_grep`, `filesystem_find` and `filesystem_ls` (ports of `grep.ts`, `find.ts`,
//! `ls.ts`): read-only views of the workspace. All three only ever surface entries whose canonical
//! path stays inside the workspace, so a symlink cannot be used to list or read outside it.
//!
//! Limits are JS numbers in TS (`params.limit || DEFAULT`), compared without integer coercion; they
//! stay `f64` here so the "Showing first N" texts and cut-offs match for any value a model sends.
//!
//! DECIDED (deviation): patterns are Rust `regex` syntax, which lacks JS lookaround and
//! backreferences; such a pattern reports `Invalid regex pattern` instead of matching.

use super::{
    FsResult, Input,
    paths::{FsError, node_error, resolve_existing, resolve_existing_from_root, workspace_root},
    util::{
        DEFAULT_FIND_LIMIT, DEFAULT_GREP_LIMIT, DEFAULT_LS_LIMIT, GREP_MAX_LINE_LENGTH,
        MAX_GREP_FILE_SIZE, glob_matcher, is_binary_path, js_number, locale_compare_base,
        or_default, to_fixed_1, truncate_line, utf16_cmp, walk_files,
    },
};
use std::{collections::BTreeSet, path::Path};

struct GrepMatch {
    file: String,
    line_number: usize,
    line: String,
    is_match: bool,
}

pub(super) fn grep(cwd: &Path, input: &Input) -> Result<FsResult, FsError> {
    let path_input = input.str("path").filter(|path| !path.is_empty());
    let search_path = resolve_existing(cwd, path_input.unwrap_or("."))?;
    let limit = or_default(input.number("limit"), DEFAULT_GREP_LIMIT);
    // A fractional context makes TS index lines fractionally and throw; whole lines are used here.
    let context = or_default(input.number("context"), 0.0).max(0.0).trunc() as usize;
    let pattern = input.str("pattern").unwrap_or_default();

    let source = if input.flag("literal") {
        regex::escape(pattern)
    } else {
        pattern.to_owned()
    };
    let Ok(regex) = regex::RegexBuilder::new(&source)
        .case_insensitive(input.flag("ignore_case"))
        .build()
    else {
        return Ok(FsResult::error(format!("Invalid regex pattern: {pattern}")));
    };
    let glob = match input.str("glob").filter(|glob| !glob.is_empty()) {
        Some(glob) => Some(glob_matcher(glob)?),
        None => None,
    };

    let Ok(meta) = std::fs::metadata(&search_path) else {
        return Ok(FsResult::error(format!(
            "Path not found: {}",
            path_input.unwrap_or(".")
        )));
    };

    let mut all_matches = Vec::new();
    let mut total = 0.0;
    if meta.is_file() {
        let display =
            path_input.map_or_else(|| search_path.to_string_lossy().into_owned(), str::to_owned);
        let Some(content) = read_grep_file(&search_path) else {
            return Ok(FsResult::error(format!("Unable to read file: {display}")));
        };
        let lines: Vec<&str> = content.split('\n').collect();
        let found = search_lines(&display, &lines, &regex, context, limit);
        total = found.iter().filter(|found| found.is_match).count() as f64;
        all_matches = found;
    } else {
        walk_files(&search_path, &mut |file| {
            if is_binary_path(&file.path) {
                return true;
            }
            if glob.as_ref().is_some_and(|glob| !glob.is_match(&file.path)) {
                return true;
            }
            let Some(content) = read_grep_file(&file.real_path) else {
                return true;
            };
            let remaining = limit - total;
            if remaining <= 0.0 {
                return false;
            }
            let lines: Vec<&str> = content.split('\n').collect();
            let found = search_lines(&file.path, &lines, &regex, context, remaining);
            total += found.iter().filter(|found| found.is_match).count() as f64;
            all_matches.extend(found);
            total < limit
        });
    }

    if all_matches.is_empty() {
        return Ok(FsResult::text("No matches found."));
    }
    let mut result = format_matches(&all_matches, context);
    if total >= limit {
        result.push_str(&format!(
            "\n\n(Showing first {} matches. Use limit={} to see more.)",
            js_number(limit),
            js_number(limit * 2.0)
        ));
    }
    Ok(FsResult::text(result))
}

/// `readGrepFileContent`: regular files only, final-component symlinks refused (TS opens with
/// `O_NOFOLLOW`), at most 2 MiB; any failure skips the file.
fn read_grep_file(path: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_GREP_FILE_SIZE {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// `searchFileLines`: up to `max_matches` matching lines plus `context` lines around each.
fn search_lines(
    file: &str,
    lines: &[&str],
    regex: &regex::Regex,
    context: usize,
    max_matches: f64,
) -> Vec<GrepMatch> {
    let mut matched = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if (matched.len() as f64) >= max_matches {
            break;
        }
        if regex.is_match(line) {
            matched.push(index);
        }
    }
    let mut included = BTreeSet::new();
    for &index in &matched {
        let from = index.saturating_sub(context);
        let to = (index + context).min(lines.len().saturating_sub(1));
        included.extend(from..=to);
    }
    included
        .into_iter()
        .map(|index| GrepMatch {
            file: file.to_owned(),
            line_number: index + 1,
            line: lines[index].to_owned(),
            is_match: matched.contains(&index),
        })
        .collect()
}

/// `path:line:text` for matches, `path-line-text` for context, `--` between non-adjacent context
/// groups, a blank line between files.
fn format_matches(matches: &[GrepMatch], context: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut previous_file: Option<&str> = None;
    let mut previous_line: Option<usize> = None;
    for found in matches {
        if previous_file != Some(found.file.as_str()) {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            previous_file = Some(&found.file);
            previous_line = None;
        }
        if context > 0 && previous_line.is_some_and(|previous| found.line_number > previous + 1) {
            lines.push("--".to_owned());
        }
        let separator = if found.is_match { ':' } else { '-' };
        lines.push(format!(
            "{}{separator}{}{separator}{}",
            found.file,
            found.line_number,
            truncate_line(&found.line, GREP_MAX_LINE_LENGTH)
        ));
        previous_line = Some(found.line_number);
    }
    lines.join("\n")
}

pub(super) fn find(cwd: &Path, input: &Input) -> Result<FsResult, FsError> {
    let path_input = input.str("path").filter(|path| !path.is_empty());
    let search_path = resolve_existing(cwd, path_input.unwrap_or("."))?;
    let limit = or_default(input.number("limit"), DEFAULT_FIND_LIMIT);
    let pattern = input.str("pattern").unwrap_or_default();
    let glob = glob_matcher(pattern)?;

    let mut matches: Vec<String> = Vec::new();
    walk_files(&search_path, &mut |file| {
        if glob.is_match(&file.path) {
            matches.push(file.path);
            if matches.len() as f64 >= limit {
                return false;
            }
        }
        true
    });

    if matches.is_empty() {
        return Ok(FsResult::text(format!(
            "No files matching \"{pattern}\" found."
        )));
    }
    matches.sort_by(|a, b| utf16_cmp(a, b));
    let mut result = matches.join("\n");
    if matches.len() as f64 >= limit {
        result.push_str(&format!(
            "\n\n(Showing first {} results. Use limit={} to see more.)",
            js_number(limit),
            js_number(limit * 2.0)
        ));
    }
    Ok(FsResult::text(result))
}

pub(super) fn ls(cwd: &Path, input: &Input) -> Result<FsResult, FsError> {
    let input_path = input
        .str("path")
        .filter(|path| !path.is_empty())
        .unwrap_or(".");
    let root = workspace_root(cwd)?;
    let resolved = resolve_existing_from_root(&root, input_path)?;
    let limit = or_default(input.number("limit"), DEFAULT_LS_LIMIT);
    let entries = std::fs::read_dir(&resolved)
        .map_err(|error| node_error(&error, "scandir", Some(&resolved)))?;

    let mut dirs: Vec<String> = Vec::new();
    let mut files: Vec<(String, u64)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // Hide entries that resolve outside the workspace (escaping or dangling symlinks).
        let child = join_display(input_path, &name);
        if resolve_existing_from_root(&root, &child).is_err() {
            continue;
        }
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            dirs.push(name);
            continue;
        }
        let size = std::fs::metadata(resolved.join(&name)).map_or(0, |meta| meta.len());
        files.push((name, size));
    }

    dirs.sort_by(|a, b| locale_compare_base(a, b));
    files.sort_by(|a, b| locale_compare_base(&a.0, &b.0));
    let mut lines: Vec<String> = Vec::new();
    for dir in &dirs {
        if lines.len() as f64 >= limit {
            break;
        }
        lines.push(format!("{dir}/"));
    }
    for (name, size) in &files {
        if lines.len() as f64 >= limit {
            break;
        }
        lines.push(format!("{name} ({})", format_size(*size)));
    }
    if lines.is_empty() {
        return Ok(FsResult::text("(empty directory)"));
    }
    let mut result = lines.join("\n");
    let total = dirs.len() + files.len();
    if total as f64 > limit {
        result.push_str(&format!(
            "\n\n(Showing {} of {total} entries. Use limit={} to see more.)",
            js_number(limit),
            js_number(limit * 2.0)
        ));
    }
    Ok(FsResult::text(result))
}

/// `path.join(inputPath, name)`, for re-validating each listed child through the boundary (which
/// folds `.`/`..` itself).
fn join_display(input_path: &str, name: &str) -> String {
    Path::new(input_path)
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{}KB", to_fixed_1(bytes, 1024))
    } else {
        format!("{}MB", to_fixed_1(bytes, 1024 * 1024))
    }
}
