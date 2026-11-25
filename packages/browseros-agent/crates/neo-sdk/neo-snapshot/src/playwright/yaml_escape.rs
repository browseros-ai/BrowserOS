use crate::js;

/// How upstream's YAML writer (vendored `isomorphic/yaml.ts`) writes a string into a line: as it
/// is, or quoted when YAML would not read it back bare as that same string. The two places in a
/// line quote differently.
#[derive(Clone, Copy, Debug)]
pub(crate) enum YamlEscape {
    /// `yamlEscapeKeyIfNeeded`, for a key line: single quotes, each `'` doubled, every other
    /// character as it is, a line break and the C1 controls included.
    Key,
    /// `yamlEscapeValueIfNeeded`, for a text, a url or a placeholder: double quotes, with
    /// backslash escapes for `\` and `"` and every control character. The empty string needs
    /// quotes, so an empty text prints `""`.
    Value,
}

impl YamlEscape {
    /// Appends `text` to `out` as upstream writes it in this place.
    pub(crate) fn write(self, text: &str, out: &mut String) {
        if !needs_quotes(text) {
            out.push_str(text);
            return;
        }
        match self {
            Self::Key => {
                out.push('\'');
                for character in text.chars() {
                    if character == '\'' {
                        out.push_str("''");
                    } else {
                        out.push(character);
                    }
                }
                out.push('\'');
            }
            Self::Value => {
                out.push('"');
                // The characters of `/[\\"\x00-\x1f\x7f-\x9f]/g`, each replaced as upstream's
                // switch does.
                for character in text.chars() {
                    match character {
                        '\\' => out.push_str("\\\\"),
                        '"' => out.push_str("\\\""),
                        '\u{8}' => out.push_str("\\b"),
                        '\u{c}' => out.push_str("\\f"),
                        '\n' => out.push_str("\\n"),
                        '\r' => out.push_str("\\r"),
                        '\t' => out.push_str("\\t"),
                        // `'\\x' + code.toString(16).padStart(2, '0')`: two lower-case digits,
                        // since no code here passes 0x9f.
                        '\0'..='\u{1f}' | '\u{7f}'..='\u{9f}' => {
                            let code = u32::from(character);
                            out.push_str("\\x");
                            out.extend(char::from_digit(code >> 4, 16));
                            out.extend(char::from_digit(code & 0xf, 16));
                        }
                        _ => out.push(character),
                    }
                }
                out.push('"');
            }
        }
    }
}

/// The words YAML reads as a boolean or null, which upstream looks up after `toLowerCase()`.
const YAML_WORDS: [&str; 9] = ["y", "n", "yes", "no", "true", "false", "on", "off", "null"];

/// `yamlStringNeedsQuotes`: whether YAML would read `text` bare as something else. Upstream's
/// tests are regular expressions; each is written here as explicit character tests with the
/// expression beside it. `\s` is JavaScript's whitespace set, never Rust's.
fn needs_quotes(text: &str) -> bool {
    let (Some(first), Some(last)) = (text.chars().next(), text.chars().next_back()) else {
        // `str.length === 0`
        return true;
    };
    // `/^\s|\s$/`
    js::is_space(first)
        || js::is_space(last)
        // `/^-/`, `/^[&*\],?!>|@"'#%]/` and `/^\[/`
        || matches!(
            first,
            '-' | '&' | '*' | ']' | ',' | '?' | '!' | '>' | '|' | '@' | '"' | '\'' | '#' | '%' | '['
        )
        // `/[\n:]$/`, the end-of-string half of `/[\n:](\s|$)/`
        || matches!(last, '\n' | ':')
        || has_quoted_character_or_pair(text)
        // `!isNaN(Number(str))`
        || js::is_number(text)
        || is_yaml_word(text)
}

/// The tests of `yamlStringNeedsQuotes` that look anywhere in the string, in one pass.
fn has_quoted_character_or_pair(text: &str) -> bool {
    let mut previous: Option<char> = None;
    for character in text.chars() {
        // `/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f-\x9f]/` and `/[\n\r]/`: together, every control
        // character up to U+009F except the tab.
        let control = matches!(character, '\0'..='\u{8}' | '\n'..='\u{1f}' | '\u{7f}'..='\u{9f}');
        // `/[{}`]/`
        let flow = matches!(character, '{' | '}' | '`');
        // `/[\n:]\s/`, the other half of `/[\n:](\s|$)/`
        let mapping = matches!(previous, Some('\n' | ':')) && js::is_space(character);
        // `/\s#/`
        let comment = previous.is_some_and(js::is_space) && character == '#';
        if control || flow || mapping || comment {
            return true;
        }
        previous = Some(character);
    }
    false
}

/// `['y', 'n', 'yes', 'no', 'true', 'false', 'on', 'off', 'null'].includes(str.toLowerCase())`,
/// compared without ASCII case, which is exactly upstream's test: do NOT replace it with a Unicode
/// case fold. Outside ASCII, `toLowerCase()` turns only two characters into something with an
/// ASCII letter: the Kelvin sign U+212A into `k`, and U+0130 into `i` and U+0307. No word has a
/// `k` or an `i`, so no string outside ASCII ever lowers into one.
fn is_yaml_word(text: &str) -> bool {
    YAML_WORDS
        .iter()
        .any(|word| text.eq_ignore_ascii_case(word))
}
