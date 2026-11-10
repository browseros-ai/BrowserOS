//! An aria snapshot as Playwright's YAML: `renderAriaSnapshotAsYaml` (`isomorphic/
//! ariaSnapshotRenderer.ts`) with no options, as Playwright's server calls it (`server/frames.ts`
//! `ariaSnapshot`), and the escaping of `isomorphic/yaml.ts`, ported to Rust. The page hands Neo
//! the snapshot as Playwright's `AriaSnapshotJSON` (the `ariaSnapshot` glue), so the YAML is
//! made here, at the last moment, exactly where Playwright's server makes it.
//!
//! The port keeps JavaScript's semantics wherever the output depends on them:
//! - a name's `length` (the 900 limit) counts UTF-16 code units, so an emoji counts twice;
//! - `\s` is JS's whitespace set (`progress::is_js_whitespace`: U+FEFF, U+00A0, U+2028, …, but
//!   not U+0085 or U+180E);
//! - `!isNaN(Number(str))` is JS's `StringToNumber` ([`parses_as_number`]): `''`, `0x1f`, `0b1`,
//!   `0o7`, `Infinity`, `1e3`, `.5` and `5.` are numbers; `1_0`, `+0x1` and `infinity` are not;
//! - `JSON.stringify(name)` escapes `"`, `\` and C0 controls only, lowercase hex
//!   ([`push_json_string`]);
//! - character classes like `[\x7f-\x9f]` test code points below U+00A0, so UTF-16 units and
//!   Rust `char`s agree on them.
//!
//! Pinned by `crates/neo-sdk/neo-playwright-injected/tests/aria-yaml-goldens.json`, which the vendored
//! renderer itself writes (`scripts/aria-yaml-goldens.ts`); the tests below decode each case
//! with the generated `AriaNode` types and compare the YAML byte for byte.

use std::borrow::Cow;

use neo_playwright_protocol::injected::{
    AriaChild, AriaInvalid, AriaInvalidKind, AriaNode, AriaNodeCursor, AriaRole, AriaTristate,
};
use serde_json::Value;

use crate::progress::is_js_whitespace;

/// The longest name a key prints, in UTF-16 code units: YAML allows 1024 characters in a key,
/// and Playwright leaves room for the role and the attributes.
const MAX_NAME_UNITS: usize = 900;

/// Strings YAML reads as a boolean, null or `~`, compared after `toLowerCase()`.
const YAML_WORDS: [&str; 10] = [
    "y", "n", "yes", "no", "true", "false", "on", "off", "null", "~",
];

/// Lowercasing never shortens a string by code points, so only a string of at most five code
/// points, at most 20 UTF-8 bytes, can lowercase to one of [`YAML_WORDS`].
const MAX_YAML_WORD_BYTES: usize = 20;

/// A snapshot that cannot be rendered: a role that does not serialize as its name, which only
/// a broken generated type could produce.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the aria role {0} has no name")]
pub(crate) struct RenderError(String);

/// One entry still to print, in document order.
enum Pending<'a> {
    Node(&'a AriaNode, usize),
    Text(&'a str, usize),
}

/// `renderAriaSnapshotAsYaml(snapshot)`: one line per node or text run, two spaces of indent
/// per level, joined with `\n` and no trailing newline (an empty snapshot is `""`). Walks the
/// tree with its own stack, in Playwright's order, so no snapshot depth can exhaust the thread's.
///
/// # Errors
///
/// [`RenderError`] for a role with no name.
pub(crate) fn render(snapshot: &[AriaNode]) -> Result<String, RenderError> {
    let mut lines: Vec<String> = Vec::new();
    let mut pending: Vec<Pending<'_>> = snapshot
        .iter()
        .rev()
        .map(|node| Pending::Node(node, 0))
        .collect();
    while let Some(next) = pending.pop() {
        let (node, depth) = match next {
            Pending::Text(text, depth) => {
                lines.push(text_line(text, depth));
                continue;
            }
            Pending::Node(node, depth) => (node, depth),
        };
        if node.role == AriaRole::Text {
            lines.push(text_line(node.text.as_deref().unwrap_or(""), depth));
            continue;
        }
        let key = format!(
            "{}- {}",
            indent(depth),
            yaml_escape_key_if_needed(&key(node)?)
        );
        let props: Vec<(&str, &str)> = [
            ("url", node.url.as_deref()),
            ("placeholder", node.placeholder.as_deref()),
        ]
        .into_iter()
        .filter_map(|(name, value)| Some((name, value?)))
        .collect();
        let has_children = node
            .children
            .as_ref()
            .is_some_and(|children| !children.is_empty());
        match (&node.text, props.is_empty()) {
            // A leaf with no text, props or children.
            (None, true) if !has_children => lines.push(key),
            // A leaf with only its text, which wins over any children.
            (Some(text), true) => {
                lines.push(format!("{key}: {}", yaml_escape_value_if_needed(text)));
            }
            _ => {
                lines.push(format!("{key}:"));
                for (name, value) in props {
                    lines.push(format!(
                        "{}- /{name}: {}",
                        indent(depth + 1),
                        yaml_escape_value_if_needed(value)
                    ));
                }
                if let Some(text) = &node.text {
                    lines.push(text_line(text, depth + 1));
                } else {
                    for child in node.children.iter().flatten().rev() {
                        pending.push(match child {
                            AriaChild::Text(text) => Pending::Text(text, depth + 1),
                            AriaChild::Node(child) => Pending::Node(child, depth + 1),
                        });
                    }
                }
            }
        }
    }
    Ok(lines.join("\n"))
}

/// `visitText`: `- text: <value>`. Playwright skips the line when the escaped value is empty,
/// which never happens: an empty text escapes to `""`.
fn text_line(text: &str, depth: usize) -> String {
    format!(
        "{}- text: {}",
        indent(depth),
        yaml_escape_value_if_needed(text)
    )
}

fn indent(depth: usize) -> String {
    "  ".repeat(depth)
}

/// `createKey`: the role, the name as JSON when it is non-empty and at most
/// [`MAX_NAME_UNITS`] long, then each attribute in Playwright's order. Every flag prints only
/// when truthy, so `false` and a `level` of 0 print nothing, and `cursor` prints only with a
/// `ref`.
fn key(node: &AriaNode) -> Result<String, RenderError> {
    let mut key = role_name(node.role)?;
    if let Some(name) = node
        .name
        .as_deref()
        .filter(|name| !name.is_empty() && name.encode_utf16().count() <= MAX_NAME_UNITS)
    {
        key.push(' ');
        push_json_string(&mut key, name);
    }
    match node.checked {
        Some(AriaTristate::Mixed(_)) => key.push_str(" [checked=mixed]"),
        Some(AriaTristate::Flag(true)) => key.push_str(" [checked]"),
        _ => {}
    }
    if node.disabled == Some(true) {
        key.push_str(" [disabled]");
    }
    if node.expanded == Some(true) {
        key.push_str(" [expanded]");
    }
    if node.active == Some(true) {
        key.push_str(" [active]");
    }
    match node.invalid {
        Some(AriaInvalid::Kind(AriaInvalidKind::Grammar)) => key.push_str(" [invalid=grammar]"),
        Some(AriaInvalid::Kind(AriaInvalidKind::Spelling)) => key.push_str(" [invalid=spelling]"),
        Some(AriaInvalid::Flag(true)) => key.push_str(" [invalid]"),
        _ => {}
    }
    if let Some(level) = node.level.filter(|level| *level != 0) {
        key.push_str(" [level=");
        key.push_str(&level.to_string());
        key.push(']');
    }
    match node.pressed {
        Some(AriaTristate::Mixed(_)) => key.push_str(" [pressed=mixed]"),
        Some(AriaTristate::Flag(true)) => key.push_str(" [pressed]"),
        _ => {}
    }
    if node.selected == Some(true) {
        key.push_str(" [selected]");
    }
    if node.aria_hidden == Some(true) {
        key.push_str(" [aria-hidden]");
    }
    if let Some(reference) = node
        .r#ref
        .as_deref()
        .filter(|reference| !reference.is_empty())
    {
        key.push_str(" [ref=");
        key.push_str(reference);
        key.push(']');
        if node.cursor == Some(AriaNodeCursor::Pointer) {
            key.push_str(" [cursor=pointer]");
        }
    }
    if let Some(rect) = &node.r#box {
        let sides = [rect.x, rect.y, rect.width, rect.height].map(|side| side.to_string());
        key.push_str(" [box=");
        key.push_str(&sides.join(","));
        key.push(']');
    }
    Ok(key)
}

/// The role's name as the spec spells it, which is how its generated enum serializes.
fn role_name(role: AriaRole) -> Result<String, RenderError> {
    match serde_json::to_value(role) {
        Ok(Value::String(name)) => Ok(name),
        _ => Err(RenderError(format!("{role:?}"))),
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Appends `\x` or `\u00` and the two lowercase hex digits of a code point below U+0100.
fn push_hex_escape(out: &mut String, prefix: &str, c: char) {
    let code = u32::from(c);
    out.push_str(prefix);
    for nibble in [code >> 4, code & 0xf] {
        out.push(char::from(HEX[(nibble & 0xf) as usize]));
    }
}

/// `JSON.stringify(text)` for a well-formed string: quotes, `\"`, `\\`, the five short escapes,
/// `\u00XX` for every other C0 control, and everything else as it is (DEL, C1 controls, U+2028).
fn push_json_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0}'..='\u{1f}' => push_hex_escape(out, "\\u00", c),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `yamlEscapeKeyIfNeeded`: single quotes, with `'` doubled, when YAML would misread `text`.
pub(crate) fn yaml_escape_key_if_needed(text: &str) -> Cow<'_, str> {
    if !needs_quotes(text) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(format!("'{}'", text.replace('\'', "''")))
}

/// `yamlEscapeValueIfNeeded`: double quotes when YAML would misread `text`, escaping `\`, `"`,
/// the five short escapes, and `\xXX` (lowercase) for the rest of `[\x00-\x1f\x7f-\x9f]`.
pub(crate) fn yaml_escape_value_if_needed(text: &str) -> Cow<'_, str> {
    if !needs_quotes(text) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}' => push_hex_escape(&mut out, "\\x", c),
            c => out.push(c),
        }
    }
    out.push('"');
    Cow::Owned(out)
}

/// `yamlStringNeedsQuotes`, rule for rule.
fn needs_quotes(text: &str) -> bool {
    let (Some(first), Some(last)) = (text.chars().next(), text.chars().next_back()) else {
        return true;
    };
    // /^\s|\s$/
    if is_js_whitespace(first) || is_js_whitespace(last) {
        return true;
    }
    // /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f-\x9f]/
    if text.chars().any(|c| {
        matches!(c, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{7f}'..='\u{9f}')
    }) {
        return true;
    }
    // /^-/
    if first == '-' {
        return true;
    }
    // /[\n:](\s|$)/
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if matches!(c, '\n' | ':') && chars.peek().is_none_or(|next| is_js_whitespace(*next)) {
            return true;
        }
    }
    // /\s#/
    if text
        .chars()
        .zip(text.chars().skip(1))
        .any(|(c, next)| is_js_whitespace(c) && next == '#')
    {
        return true;
    }
    // /[\n\r]/
    if text.contains(['\n', '\r']) {
        return true;
    }
    // /^[&*\],?!>|@"'#%]/
    if matches!(
        first,
        '&' | '*' | ']' | ',' | '?' | '!' | '>' | '|' | '@' | '"' | '\'' | '#' | '%'
    ) {
        return true;
    }
    // /[{}`]/ and /^\[/
    if text.contains(['{', '}', '`']) || first == '[' {
        return true;
    }
    // !isNaN(Number(str)) || [...].includes(str.toLowerCase())
    if parses_as_number(text)
        || (text.len() <= MAX_YAML_WORD_BYTES && YAML_WORDS.contains(&text.to_lowercase().as_str()))
    {
        return true;
    }
    // /^[-+]?\.(inf|nan)$/i, whose `i` without `u` folds ASCII letters only.
    let unsigned = text.strip_prefix(['-', '+']).unwrap_or(text);
    unsigned
        .strip_prefix('.')
        .is_some_and(|rest| rest.eq_ignore_ascii_case("inf") || rest.eq_ignore_ascii_case("nan"))
}

/// Whether JS's `Number(text)` is not `NaN` (`StringToNumber`): after trimming JS whitespace,
/// empty, or a `0x`/`0o`/`0b` integer with at least one digit and no sign, or a decimal with an
/// optional sign, digits on at least one side of an optional `.`, and an optional exponent, or
/// `Infinity` with an optional sign. No numeric separators, no `n` suffix.
fn parses_as_number(text: &str) -> bool {
    let text = text.trim_matches(is_js_whitespace);
    if text.is_empty() {
        return true;
    }
    let radix = match text.get(..2) {
        Some("0x" | "0X") => Some(16),
        Some("0o" | "0O") => Some(8),
        Some("0b" | "0B") => Some(2),
        _ => None,
    };
    if let Some(radix) = radix
        && text.len() > 2
    {
        return text[2..].chars().all(|c| c.is_digit(radix));
    }
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    if unsigned == "Infinity" {
        return true;
    }
    let mut chars = unsigned.chars().peekable();
    let mut digits = 0;
    while chars.next_if(char::is_ascii_digit).is_some() {
        digits += 1;
    }
    if chars.next_if_eq(&'.').is_some() {
        while chars.next_if(char::is_ascii_digit).is_some() {
            digits += 1;
        }
    }
    if digits == 0 {
        return false;
    }
    if chars.next_if(|c| matches!(c, 'e' | 'E')).is_some() {
        let _sign = chars.next_if(|c| matches!(c, '+' | '-'));
        let mut exponent_digits = 0;
        while chars.next_if(char::is_ascii_digit).is_some() {
            exponent_digits += 1;
        }
        if exponent_digits == 0 {
            return false;
        }
    }
    chars.next().is_none()
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::slice;

    use serde::Deserialize;

    use super::*;

    #[derive(Deserialize)]
    struct Goldens {
        snapshots: Vec<SnapshotCase>,
        strings: Vec<StringCase>,
    }

    #[derive(Deserialize)]
    struct SnapshotCase {
        name: String,
        nodes: Vec<AriaNode>,
        yaml: String,
    }

    #[derive(Deserialize)]
    struct StringCase {
        input: String,
        key: String,
        value: String,
    }

    /// Read at run time through `CARGO_MANIFEST_DIR`, so a test binary from a shared build dir
    /// reads this checkout's corpus.
    fn goldens() -> Goldens {
        let crate_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
        let path = crate_dir.join("../neo-playwright-injected/tests/aria-yaml-goldens.json");
        let text = fs::read_to_string(&path).expect("the aria YAML goldens are committed");
        serde_json::from_str(&text).expect("every golden decodes with the generated types")
    }

    #[test]
    fn every_golden_snapshot_renders_as_playwrights_renderer_does() {
        let goldens = goldens();
        assert!(goldens.snapshots.len() >= 40, "the corpus is whole");
        for case in &goldens.snapshots {
            let rendered = render(&case.nodes).expect("every role has a name");
            assert!(
                rendered == case.yaml,
                "{}:\n--- Playwright\n{}\n--- Neo\n{rendered}",
                case.name,
                case.yaml
            );
        }
    }

    #[test]
    fn every_golden_string_escapes_as_playwrights_yaml_does() {
        let goldens = goldens();
        assert!(goldens.strings.len() >= 200, "the corpus is whole");
        for case in &goldens.strings {
            assert_eq!(
                yaml_escape_key_if_needed(&case.input),
                case.key,
                "key {:?}",
                case.input
            );
            assert_eq!(
                yaml_escape_value_if_needed(&case.input),
                case.value,
                "value {:?}",
                case.input
            );
        }
    }

    #[test]
    fn number_strings_follow_javascripts_string_to_number() {
        for number in [
            "",
            "0",
            "+1",
            "-1",
            ".5",
            "5.",
            "1e3",
            "1E-3",
            "1.e3",
            "0x1f",
            "0B1",
            "0o7",
            "Infinity",
            "-Infinity",
            "1e1000",
            " 12\u{a0}",
        ] {
            assert!(parses_as_number(number), "{number:?} is a number");
        }
        for not_a_number in [
            ".", "+", "e3", ".e3", "1e", "1e+", "0x", "0xg", "+0x1", "-0x1", "0b2", "00x1", "1_0",
            "12n", "NaN", "infinity", "\u{661}", "1 2",
        ] {
            assert!(!parses_as_number(not_a_number), "{not_a_number:?} is NaN");
        }
    }

    /// A tree deeper than a recursive walk could rely on still renders, line for line.
    #[test]
    fn a_deep_tree_renders_without_recursing() {
        let depth = 5_000;
        let mut node = AriaNode {
            role: AriaRole::Button,
            name: Some("deepest".to_string()),
            ..node_template()
        };
        for _ in 0..depth {
            node = AriaNode {
                role: AriaRole::Group,
                children: Some(vec![AriaChild::Node(Box::new(node))]),
                ..node_template()
            };
        }
        let rendered = render(slice::from_ref(&node)).expect("renders");
        assert_eq!(rendered.lines().count(), depth + 1);
        assert_eq!(
            rendered.lines().last(),
            Some(format!("{}- button \"deepest\"", "  ".repeat(depth)).as_str())
        );
        // Dropping the tree would recurse through every level; take it apart level by level.
        let mut next = Some(node);
        while let Some(mut current) = next.take() {
            next = current
                .children
                .take()
                .and_then(|children| children.into_iter().next())
                .and_then(|child| match child {
                    AriaChild::Node(child) => Some(*child),
                    AriaChild::Text(_) => None,
                });
        }
    }

    fn node_template() -> AriaNode {
        AriaNode {
            role: AriaRole::Generic,
            name: None,
            checked: None,
            disabled: None,
            expanded: None,
            active: None,
            invalid: None,
            level: None,
            pressed: None,
            selected: None,
            aria_hidden: None,
            r#ref: None,
            cursor: None,
            r#box: None,
            url: None,
            placeholder: None,
            text: None,
            children: None,
        }
    }
}
