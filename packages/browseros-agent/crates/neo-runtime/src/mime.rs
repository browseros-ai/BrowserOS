//! Neo: `mime.getType` of the `mime` package (4.1.0) that Playwright's server types upload and
//! drop files with, over that package's own tables (`vendor/mime/types/`, unchanged), read once
//! in the order its default export defines them: `new Mime(standardTypes, otherTypes)`.

use std::collections::HashMap;
use std::sync::LazyLock;

/// Neo: mime 4.1.0's `types/standard.ts`.
const STANDARD: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/vendor/mime/types/standard.ts"
));
/// Neo: mime 4.1.0's `types/other.ts`.
const OTHER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/vendor/mime/types/other.ts"
));

/// Neo: every extension the two tables map, to its type.
static EXTENSION_TO_TYPE: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    for table in [STANDARD, OTHER] {
        define(&mut map, table);
    }
    map
});

/// Neo: mime's `Mime.define` for one table (`{ 'type': ['ext', '*ext'], … }`): each type's
/// extensions, lowercased; a starred one is never mapped, so another table's type keeps it.
fn define(map: &mut HashMap<String, String>, table: &str) {
    let mut current: Option<String> = None;
    let mut in_list = false;
    let mut chars = table.chars();
    while let Some(c) = chars.next() {
        match c {
            '[' => in_list = true,
            ']' => in_list = false,
            '\'' => {
                let text: String = chars.by_ref().take_while(|&c| c != '\'').collect();
                let text = text.to_lowercase();
                if !in_list {
                    current = Some(text);
                } else if let Some(kind) = &current
                    && !text.starts_with('*')
                {
                    map.insert(text, kind.clone());
                }
            }
            _ => {}
        }
    }
}

/// Playwright: `server/fileUploadUtils.ts` `prepareFilesForUpload`: `mimeType: payload.mimeType || mime.getType(payload.name) || 'application/octet-stream',`
///
/// mime's `Mime.getType`: the type of `path`'s extension (what follows its last `.` after its
/// last `/` or `\`), lowercased; `None` for an unknown extension and for a file with a path but
/// no extension. A bare name is its own extension (`getType('txt')` is `text/plain`).
pub(crate) fn get_type(path: &str) -> Option<&'static str> {
    let slash = path.rfind(['/', '\\']);
    let has_path = slash.is_some();
    let last = slash
        .map_or(path, |slash| &path[slash + 1..])
        .to_lowercase();
    let ext = last
        .rfind('.')
        .map_or(last.as_str(), |dot| &last[dot + 1..]);
    // `ext.length < last.length - 1`: something precedes the dot.
    let has_dot = ext.len() + 1 < last.len();
    if !has_dot && has_path {
        return None;
    }
    EXTENSION_TO_TYPE.get(ext).map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Common names, an office document from the `other` table, a starred extension left to the
    /// standard table, and mime's own edge rules.
    #[test]
    fn tests_that_get_type_reads_extensions_as_mime_does() {
        for (path, expected) in [
            ("file.txt", Some("text/plain")),
            ("PHOTO.PNG", Some("image/png")),
            ("/tmp/data.json", Some("application/json")),
            ("dir\\page.html", Some("text/html")),
            (
                "report.docx",
                Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
            ),
            ("song.wav", Some("audio/wav")),
            ("archive.tar.gz", Some("application/gzip")),
            ("txt", Some("text/plain")),
            (".txt", Some("text/plain")),
            ("dir/.txt", None),
            ("dir/README", None),
            ("file.unknownextension", None),
            ("", None),
        ] {
            assert_eq!(get_type(path), expected, "{path:?}");
        }
    }
}
