//! `filesystem_write` and `filesystem_edit` (ports of `write.ts` / `edit.ts`): the two tools that
//! modify the workspace. Edit is exact-match first, then a whitespace-tolerant line match, and
//! preserves the file's BOM and line endings.

use super::{
    FsResult, Input,
    paths::{FsError, node_error, resolve_existing, resolve_write_target},
    read::read_utf8_lossy,
    util::{detect_line_ending, js_replace_first, normalize_to_lf, restore_line_endings, strip_bom},
};
use std::path::Path;

pub(super) fn write(cwd: &Path, input: &Input) -> Result<FsResult, FsError> {
    let path = input.str("path").unwrap_or_default();
    let content = input.str("content").unwrap_or_default();
    let resolved = resolve_write_target(cwd, path)?;
    if let Some(parent) = resolved.parent() {
        std::fs::create_dir_all(parent).map_err(|error| node_error(&error, "mkdir", Some(parent)))?;
    }
    std::fs::write(&resolved, content).map_err(|error| node_error(&error, "open", Some(&resolved)))?;
    Ok(FsResult::text(format!(
        "Wrote {} bytes to {path}",
        content.len()
    )))
}

pub(super) fn edit(cwd: &Path, input: &Input) -> Result<FsResult, FsError> {
    let path = input.str("path").unwrap_or_default();
    let old_string = input.str("old_string").unwrap_or_default();
    let new_string = input.str("new_string").unwrap_or_default();
    let resolved = resolve_existing(cwd, path)?;
    let raw = read_utf8_lossy(&resolved)?;

    let (without_bom, has_bom) = strip_bom(&raw);
    let line_ending = detect_line_ending(without_bom);
    let content = normalize_to_lf(without_bom);
    let old_norm = normalize_to_lf(old_string);
    let new_norm = normalize_to_lf(new_string);

    if old_norm == new_norm {
        return Ok(FsResult::error(
            "old_string and new_string are identical — no change needed.",
        ));
    }

    let exact = count_occurrences(&content, &old_norm);
    let updated = if exact == 1 {
        js_replace_first(&content, &old_norm, &new_norm)
    } else if exact > 1 {
        return Ok(FsResult::error(format!(
            "Found {exact} exact occurrences of old_string. Add more surrounding context to make the match unique."
        )));
    } else {
        match fuzzy_replace(&content, &old_norm, &new_norm)? {
            Some(updated) => updated,
            None => {
                return Ok(FsResult::error(
                    "old_string not found in file (exact and whitespace-tolerant match both failed).",
                ));
            }
        }
    };

    let mut final_content = restore_line_endings(&updated, line_ending);
    if has_bom {
        final_content.insert(0, '\u{feff}');
    }
    std::fs::write(&resolved, final_content)
        .map_err(|error| node_error(&error, "open", Some(&resolved)))?;

    let diff = old_string
        .split('\n')
        .map(|line| format!("- {line}"))
        .chain(new_string.split('\n').map(|line| format!("+ {line}")))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(FsResult::text(format!("Applied edit to {path}\n\n{diff}")))
}

/// Non-overlapping occurrences. DECIDED (deviation): an empty needle counts every position, so
/// the edit reports a non-unique match; TS's `indexOf` loop never advances and hangs the tool.
fn count_occurrences(content: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return content.chars().count() + 1;
    }
    content.matches(needle).count()
}

/// Line-wise match with each line trimmed; must be unique.
fn fuzzy_replace(content: &str, old: &str, new: &str) -> Result<Option<String>, FsError> {
    let content_lines: Vec<&str> = content.split('\n').collect();
    let search: Vec<&str> = old.split('\n').map(str::trim).collect();
    if search.len() > content_lines.len() {
        return Ok(None);
    }
    let mut matches = 0;
    let mut first = None;
    for start in 0..=content_lines.len() - search.len() {
        let all = search
            .iter()
            .enumerate()
            .all(|(offset, line)| content_lines[start + offset].trim() == *line);
        if all {
            matches += 1;
            first.get_or_insert(start);
        }
    }
    let Some(start) = first else {
        return Ok(None);
    };
    if matches > 1 {
        return Err(format!(
            "Whitespace-tolerant match found {matches} occurrences. Add more context to make the match unique."
        ));
    }
    let mut lines: Vec<&str> = content_lines[..start].to_vec();
    lines.extend(new.split('\n'));
    lines.extend(&content_lines[start + search.len()..]);
    Ok(Some(lines.join("\n")))
}
