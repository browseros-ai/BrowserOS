//! Shared helpers of the filesystem tools (port of `tools/filesystem/utils.ts`): output
//! truncation, the visible-file walk, binary/image detection, line-ending handling, and the JS
//! string/number semantics the TS output texts depend on (UTF-16 lengths, `Number#toString`,
//! `String#replace` substitution patterns, `localeCompare` collation).

use super::paths::{is_path_inside, realpath};
use std::{
    cmp::Ordering,
    path::{Path, PathBuf},
};

const MAX_LINES: usize = 2000;
const MAX_BYTES: usize = 50 * 1024;
pub(super) const GREP_MAX_LINE_LENGTH: usize = 500;
pub(super) const DEFAULT_GREP_LIMIT: f64 = 100.0;
pub(super) const DEFAULT_FIND_LIMIT: f64 = 1000.0;
pub(super) const DEFAULT_LS_LIMIT: f64 = 500.0;
pub(super) const DEFAULT_BASH_TIMEOUT: f64 = 120.0;
pub(super) const MAX_GREP_FILE_SIZE: u64 = 2 * 1024 * 1024;
/// `TOOL_LIMITS.FILESYSTEM_READ_MAX_LINES` / `_MAX_CHARS` (`@browseros/shared/constants/limits`).
pub(super) const MAX_READ_LINES: usize = 500;
pub(super) const MAX_READ_CHARS: usize = 15_000;

pub(super) struct Truncation {
    pub content: String,
    pub truncated: bool,
    pub total_lines: usize,
    pub kept_lines: usize,
}

/// `truncateTail`: the last ≤2000 lines / ≤50 KiB (UTF-8 bytes + 1 per newline) of `content`.
pub(super) fn truncate_tail(content: &str) -> Truncation {
    let lines: Vec<&str> = content.split('\n').collect();
    let total_lines = lines.len();
    let mut kept: Vec<&str> = Vec::new();
    let mut bytes = 0;
    let mut truncated = false;
    for line in lines.iter().rev() {
        let line_bytes = line.len() + 1;
        if kept.len() >= MAX_LINES || bytes + line_bytes > MAX_BYTES {
            truncated = true;
            break;
        }
        kept.push(line);
        bytes += line_bytes;
    }
    kept.reverse();
    Truncation {
        kept_lines: kept.len(),
        content: kept.join("\n"),
        truncated,
        total_lines,
    }
}

/// `truncateLine`: cut at `max` UTF-16 units and mark it.
pub(super) fn truncate_line(line: &str, max: usize) -> String {
    if utf16_len(line) <= max {
        return line.to_owned();
    }
    format!("{} [truncated]", utf16_prefix(line, max))
}

pub(super) fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The first `max` UTF-16 units; a surrogate pair straddling the cut is dropped whole (JS would
/// keep a lone high surrogate, which a Rust string cannot hold).
fn utf16_prefix(text: &str, max: usize) -> &str {
    let mut units = 0;
    for (index, ch) in text.char_indices() {
        units += ch.len_utf16();
        if units > max {
            return &text[..index];
        }
    }
    text
}

/// Directories `walkFiles` never descends into.
const IGNORED_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "dist",
    "build",
    ".next",
    "coverage",
    "__pycache__",
    ".venv",
    "venv",
    ".tox",
    "target",
    ".gradle",
    ".idea",
    ".vscode",
    ".cache",
    ".turbo",
    ".output",
    ".nuxt",
    ".svelte-kit",
    ".parcel-cache",
    ".angular",
    ".expo",
    ".yarn",
    ".pnp",
];

const BINARY_EXTENSIONS: &[&str] = &[
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".ico", ".woff", ".woff2", ".ttf", ".eot",
    ".otf", ".pdf", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx", ".zip", ".tar", ".gz",
    ".bz2", ".rar", ".7z", ".xz", ".exe", ".dll", ".so", ".dylib", ".bin", ".o", ".a", ".mp3",
    ".mp4", ".avi", ".mov", ".wav", ".flac", ".ogg", ".sqlite", ".db", ".wasm", ".class", ".pyc",
];

/// `isBinaryPath`: by the text after the last `.` anywhere in the path (TS quirk: a dot in a
/// directory name counts when the file has none).
pub(super) fn is_binary_path(path: &str) -> bool {
    path.rfind('.')
        .is_some_and(|dot| BINARY_EXTENSIONS.contains(&path[dot..].to_lowercase().as_str()))
}

/// `IMAGE_MIME_TYPES` keyed by lowercase extension; membership doubles as `IMAGE_EXTENSIONS`.
pub(super) fn image_mime_type(extension: &str) -> Option<&'static str> {
    Some(match extension {
        ".png" => "image/png",
        ".jpg" | ".jpeg" => "image/jpeg",
        ".gif" => "image/gif",
        ".webp" => "image/webp",
        ".bmp" => "image/bmp",
        ".svg" => "image/svg+xml",
        ".ico" => "image/x-icon",
        _ => return None,
    })
}

/// Node's `path.extname`: from the last `.` of the final component, unless that dot leads it.
pub(super) fn extname(path: &Path) -> String {
    let Some(name) = path.file_name().map(|name| name.to_string_lossy().into_owned()) else {
        return String::new();
    };
    match name.rfind('.') {
        Some(0) | None => String::new(),
        Some(dot) => name[dot..].to_owned(),
    }
}

pub(super) struct WalkEntry {
    /// Relative to the walk's base, for display and glob matching.
    pub path: String,
    /// Canonical path, for reading.
    pub real_path: PathBuf,
}

/// `walkFiles`: depth-first in directory order, skipping `IGNORED_DIRS`, yielding regular files
/// and symlinks whose canonical target stays under the canonical base. Unreadable directories are
/// skipped silently. `visit` returns `false` to stop the walk early.
pub(super) fn walk_files(base: &Path, visit: &mut dyn FnMut(WalkEntry) -> bool) {
    let Ok(root) = realpath(base) else {
        return;
    };
    walk_dir(base, base, &root, visit);
}

fn walk_dir(
    dir: &Path,
    base: &Path,
    root: &Path,
    visit: &mut dyn FnMut(WalkEntry) -> bool,
) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return true;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let full = dir.join(&name);
        if file_type.is_dir() {
            if IGNORED_DIRS.contains(&name.as_str()) {
                continue;
            }
            if !walk_dir(&full, base, root, visit) {
                return false;
            }
        } else if file_type.is_file() || file_type.is_symlink() {
            let Ok(canonical) = std::fs::canonicalize(&full) else {
                continue;
            };
            if !is_path_inside(root, &canonical) {
                continue;
            }
            let path = full
                .strip_prefix(base)
                .map(|relative| relative.to_string_lossy().into_owned())
                .unwrap_or_else(|_| full.to_string_lossy().into_owned());
            if !visit(WalkEntry {
                path,
                real_path: canonical,
            }) {
                return false;
            }
        }
    }
    true
}

/// `Bun.Glob` matching on a relative path: `*` stays within one segment, `**/` spans zero or more
/// directories, braces expand. A pattern with neither `/` nor `**` is searched at any depth.
pub(super) fn glob_matcher(pattern: &str) -> Result<globset::GlobMatcher, String> {
    let effective = if pattern.contains('/') || pattern.contains("**") {
        pattern.to_owned()
    } else {
        format!("**/{pattern}")
    };
    globset::GlobBuilder::new(&effective)
        .literal_separator(true)
        .build()
        .map(|glob| glob.compile_matcher())
        .map_err(|error| error.to_string())
}

pub(super) fn strip_bom(content: &str) -> (&str, bool) {
    match content.strip_prefix('\u{feff}') {
        Some(rest) => (rest, true),
        None => (content, false),
    }
}

pub(super) fn detect_line_ending(content: &str) -> &'static str {
    if content.contains("\r\n") {
        "\r\n"
    } else if content.contains('\r') {
        "\r"
    } else {
        "\n"
    }
}

pub(super) fn normalize_to_lf(content: &str) -> String {
    content.replace("\r\n", "\n").replace('\r', "\n")
}

pub(super) fn restore_line_endings(content: &str, ending: &str) -> String {
    if ending == "\n" {
        content.to_owned()
    } else {
        content.replace('\n', ending)
    }
}

/// `String(n)` for the numbers these tools print (line numbers, limits, timeouts): integral values
/// without a fraction, everything else in Rust's shortest round-trip form, which agrees with JS
/// for the magnitudes that occur here.
pub(super) fn js_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e21 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

/// `params.x || fallback` for an optional number: `0` and absence both pick the fallback.
pub(super) fn or_default(value: Option<f64>, fallback: f64) -> f64 {
    match value {
        Some(value) if value != 0.0 && !value.is_nan() => value,
        _ => fallback,
    }
}

/// `String.prototype.replace(string, string)`: replaces the first occurrence, honouring the `$$`,
/// `$&`, `` $` `` and `$'` substitution patterns (a string pattern has no capture groups, so `$1`
/// stays literal). `filesystem_edit` inherits this quirk from TS.
pub(super) fn js_replace_first(haystack: &str, needle: &str, replacement: &str) -> String {
    let Some(start) = haystack.find(needle) else {
        return haystack.to_owned();
    };
    let end = start + needle.len();
    let mut substituted = String::with_capacity(replacement.len());
    let mut chars = replacement.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            substituted.push(ch);
            continue;
        }
        match chars.peek() {
            Some('$') => {
                chars.next();
                substituted.push('$');
            }
            Some('&') => {
                chars.next();
                substituted.push_str(needle);
            }
            Some('`') => {
                chars.next();
                substituted.push_str(&haystack[..start]);
            }
            Some('\'') => {
                chars.next();
                substituted.push_str(&haystack[end..]);
            }
            _ => substituted.push('$'),
        }
    }
    format!("{}{substituted}{}", &haystack[..start], &haystack[end..])
}

/// `a.localeCompare(b, undefined, { sensitivity: 'base' })` for file names: ICU root collation's
/// primary order for ASCII (whitespace, then punctuation and symbols in DUCET order, then digits,
/// then letters ignoring case). Non-ASCII characters sort after ASCII by lowercase code point, an
/// approximation of the full collation. Equal names compare equal, so a stable sort keeps
/// directory order for them, as V8's does.
pub(super) fn locale_compare_base(a: &str, b: &str) -> Ordering {
    a.chars()
        .map(collation_weight)
        .cmp(b.chars().map(collation_weight))
}

fn collation_weight(ch: char) -> u32 {
    const PUNCTUATION: &str = " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";
    if let Some(index) = PUNCTUATION.find(ch) {
        return index as u32;
    }
    if ch.is_ascii_digit() {
        return 100 + (ch as u32 - '0' as u32);
    }
    if ch.is_ascii_alphabetic() {
        return 200 + (ch.to_ascii_lowercase() as u32 - 'a' as u32);
    }
    let lower = ch.to_lowercase().next().unwrap_or(ch);
    1_000 + lower as u32
}

/// `Array#sort()` default order: UTF-16 code units.
pub(super) fn utf16_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// `(bytes / divisor).toFixed(1)` for the exact quotients file sizes produce (powers of two), with
/// JS's round-half-up rather than Rust's round-half-even formatting.
pub(super) fn to_fixed_1(bytes: u64, divisor: u64) -> String {
    let tenths = (u128::from(bytes) * 10 + u128::from(divisor) / 2) / u128::from(divisor);
    format!("{}.{}", tenths / 10, tenths % 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_replace_honours_substitution_patterns() {
        assert_eq!(js_replace_first("a-b-a", "a", "[$&]"), "[a]-b-a");
        assert_eq!(js_replace_first("xay", "a", "$`|$'"), "xx|yy");
        assert_eq!(js_replace_first("a", "a", "$$1 $1"), "$1 $1");
    }

    #[test]
    fn collation_puts_punctuation_digits_letters_in_icu_order() {
        let mut names = vec!["b", "a.b", "_x", "a-b", "A", "1", "a_b", "Zed"];
        names.sort_by(|a, b| locale_compare_base(a, b));
        assert_eq!(names, vec!["_x", "1", "A", "a_b", "a-b", "a.b", "b", "Zed"]);
    }

    #[test]
    fn to_fixed_rounds_half_up() {
        assert_eq!(to_fixed_1(1024 + 256, 1024), "1.3");
        assert_eq!(to_fixed_1(1024, 1024), "1.0");
        assert_eq!(to_fixed_1(1536, 1024), "1.5");
    }

    #[test]
    fn glob_matches_like_bun() -> Result<(), String> {
        assert!(glob_matcher("*.ts")?.is_match("src/a.ts"));
        assert!(glob_matcher("*.ts")?.is_match("a.ts"));
        assert!(!glob_matcher("src/*.ts")?.is_match("src/deep/a.ts"));
        assert!(glob_matcher("**/*.{js,jsx}")?.is_match("a/b.jsx"));
        Ok(())
    }
}
