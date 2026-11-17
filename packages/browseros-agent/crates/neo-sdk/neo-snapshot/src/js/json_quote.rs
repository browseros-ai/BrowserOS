/// `JSON.stringify(s)` for a string, appended to `out`: a `"`, each character, a `"`. `"` and `\`
/// take a backslash, as do the five control characters with a short form (`\b`, `\t`, `\n`, `\f`,
/// `\r`); every other character below U+0020 is `\u00xx` in lower-case hex; everything else,
/// U+007F to U+009F, U+2028, U+2029 and `/` included, is written as it is. Written out by hand
/// so that no `serde_json` change can move a byte of a printed name.
pub(crate) fn json_quote(s: &str, out: &mut String) {
    out.push('"');
    for character in s.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            '\0'..='\u{1f}' => {
                let code = u32::from(character);
                out.push_str(if code < 0x10 { "\\u000" } else { "\\u001" });
                out.extend(char::from_digit(code % 16, 16));
            }
            _ => out.push(character),
        }
    }
    out.push('"');
}
