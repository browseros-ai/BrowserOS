//! Whether CDP method names stay behind the door.
//!
//! `src/chromium_bridge/cdp.rs` and `cdp/<domain>.rs` declare every CDP command and event
//! neo-runtime names, each under its method's name (`page::Navigate` is `Page.navigate`).
//! Everything else sends and reads them by type, and an error that names a method reads the
//! declared command's `METHOD` (`cdp::page::Navigate::METHOD`), so each method is spelled once.
//! This test fails when a string literal in non-test code under `src/`, outside the door, quotes
//! a protocol method: a domain of browseros-cdp's pinned `protocol/protocol.json`, a dot and a
//! lowercase method name (`"Protocol error (Page.navigate): …"`).
//!
//! A file's test module, from the `#[cfg(test)]` that opens it on, may quote methods: its
//! fixtures stand for what the wire carries. So may comments. A test-only item above the test
//! module (`#[cfg(test)] fn glue_key`) does not end the scan, so the production code after it is
//! still checked.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::Value;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every domain of browseros-cdp's pinned protocol (`DOM`, `DOMStorage`, `Page`, …).
fn protocol_domains() -> Vec<String> {
    let path = crate_dir().join("../../browseros-cdp/protocol/protocol.json");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let protocol: Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()));
    let domains = protocol["domains"]
        .as_array()
        .unwrap_or_else(|| panic!("{} lists no domains", path.display()));
    domains
        .iter()
        .map(|domain| {
            domain["domain"]
                .as_str()
                .unwrap_or_else(|| panic!("a domain of {} has no name", path.display()))
                .to_string()
        })
        .collect()
}

/// A protocol method inside a string literal: anchored on a quote, then, before the next quote,
/// a protocol domain, a dot and a lowercase method name. Matched against one whole literal at a
/// time ([`tokens`]), so the quote is always the literal's own, never one closing another.
fn quoted_method(domains: &[String]) -> Regex {
    let alternation = domains
        .iter()
        .map(|domain| regex::escape(domain))
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!(r#""[^"]*?\b(?:{alternation})\.[a-z][A-Za-z]*\b"#))
        .unwrap_or_else(|error| panic!("the method pattern: {error}"))
}

/// Rust's comments, raw strings, strings and char literals, found left to right as the compiler
/// reads them: a quote inside a comment or a char literal (`'"'`) never opens a string, and a
/// `//` inside a string (`"https://…"`) never opens a comment.
fn tokens() -> Regex {
    Regex::new(
        r###"(?s)//[^\n]*|/\*.*?\*/|r##".*?"##|r#".*?"#|r".*?"|"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)'"###,
    )
    .unwrap_or_else(|error| panic!("the token pattern: {error}"))
}

/// `source` up to the `#[cfg(test)]` that opens a test module (the attribute, then a `mod`
/// line): the rest is the file's test code. A `#[cfg(test)]` on any other item is skipped.
fn non_test_code(source: &str) -> &str {
    let mut offset = 0;
    let mut lines = source.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        let opens_test_module = line.trim_start().starts_with("#[cfg(test)]")
            && lines.peek().is_some_and(|next| {
                let next = next.trim_start();
                next.starts_with("mod ") || next.starts_with("pub(crate) mod ")
            });
        if opens_test_module {
            return &source[..offset];
        }
        offset += line.len();
    }
    source
}

/// Each string literal of `source`'s non-test code that quotes a protocol method: its line and
/// the literal.
fn quoted_methods(source: &str, method: &Regex, tokens: &Regex) -> Vec<(usize, String)> {
    let code = non_test_code(source);
    tokens
        .find_iter(code)
        .filter(|token| {
            let text = token.as_str();
            (text.starts_with('"') || text.starts_with('r')) && method.is_match(text)
        })
        .map(|token| {
            let line = code[..token.start()].matches('\n').count() + 1;
            (line, token.as_str().to_string())
        })
        .collect()
}

/// Every `.rs` file under `folder`, the door's own files left out.
fn sources_outside_the_door(folder: &Path, found: &mut Vec<PathBuf>) {
    let door = crate_dir().join("src/chromium_bridge");
    let entries =
        fs::read_dir(folder).unwrap_or_else(|error| panic!("read {}: {error}", folder.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|error| panic!("list {}: {error}", folder.display()))
            .path();
        let in_the_door = path.parent() == Some(door.as_path())
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("cdp"));
        if in_the_door {
            continue;
        }
        if path.is_dir() {
            sources_outside_the_door(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

#[test]
fn no_source_outside_the_door_quotes_a_cdp_method() {
    let method = quoted_method(&protocol_domains());
    let tokens = tokens();
    let mut sources = Vec::new();
    sources_outside_the_door(&crate_dir().join("src"), &mut sources);
    sources.sort();
    assert!(
        sources.len() > 50,
        "the walk found only {} sources",
        sources.len()
    );
    let mut quoted = Vec::new();
    for path in &sources {
        let source = fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let relative = path.strip_prefix(crate_dir()).unwrap_or(path);
        for (line, literal) in quoted_methods(&source, &method, &tokens) {
            quoted.push(format!("{}:{line}: {literal}", relative.display()));
        }
    }
    assert!(
        quoted.is_empty(),
        "name a CDP method with its declared command's METHOD (src/chromium_bridge/cdp.rs), not a string:\n{}",
        quoted.join("\n")
    );
}

#[test]
fn the_scan_finds_a_quoted_method_and_nothing_else() {
    let method = quoted_method(&["DOM".to_string(), "Page".to_string()]);
    let source = r##"// "Page.navigate" in a comment
const QUOTE: char = '"';
const URL: &str = "https://a.test/page.html"; // "DOM.getBoxModel"
fn named() -> String { format!("{}: x", NAVIGATE) }
fn error(message: &str) -> String { format!("Protocol error (Page.navigate): {message}") }
/* "DOM.resolveNode" */
const RAW: &str = r#"{"method": "DOM.describeNode"}"#;
const WORD: &str = "Pages.list and Page.Navigate are no methods";
#[cfg(test)]
fn test_only() -> &'static str { "Page.reload" }
const AFTER: &str = "DOM.focus";
#[cfg(test)]
mod tests {
    const FIXTURE: &str = "Page.frameNavigated";
}
"##;
    let found: Vec<usize> = quoted_methods(source, &method, &tokens())
        .into_iter()
        .map(|(line, _)| line)
        .collect();
    assert_eq!(
        found,
        [5, 7, 10, 11],
        "a method in a string or raw string counts, test-only items above the test module included; comments, char literals, other words and the test module do not"
    );
}
