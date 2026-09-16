//! Reduces a `run` script to its shape.
//!
//! Identifiers and call structure survive because the agent wrote them and they carry
//! the diagnosis. Every literal is replaced, because the user's data lives in literals:
//! URLs, search terms, credentials typed into a form. Comments go entirely, since agents
//! narrate intent in them.
//!
//! The lexer only ever *removes* information. An unterminated string or an unrecognised
//! construct collapses into a placeholder rather than passing through, so malformed input
//! cannot leak by falling off the end of a match arm.

/// Keeps a fingerprint small enough to stay a fingerprint. Longer scripts are cut here
/// and marked, which is itself a useful signal about the shape of the failure.
pub const FINGERPRINT_MAX_BYTES: usize = 4_000;

const TRUNCATION_MARKER: &str = "\n/* truncated */";

/// Single digits survive because they carry meaning in slicing and indexing and cannot
/// identify anyone. Anything wider becomes `<num>`: ports, ids, prices, counts.
fn is_safe_digit(raw: &str) -> bool {
    raw.len() == 1 && raw.as_bytes()[0].is_ascii_digit()
}

/// True when a `/` at this point starts a regex rather than a division. Standard
/// heuristic: regex can only follow a position where a value cannot.
fn slash_starts_regex(previous: Option<char>) -> bool {
    match previous {
        None => true,
        Some(c) => matches!(
            c,
            '(' | ','
                | '='
                | ':'
                | '['
                | '!'
                | '&'
                | '|'
                | '?'
                | '{'
                | '}'
                | ';'
                | '+'
                | '-'
                | '*'
                | '%'
                | '<'
                | '>'
                | '~'
                | '^'
                | '\n'
        ),
    }
}

/// Rewrites `source` into a structural fingerprint. Never returns any literal text from
/// the input.
#[must_use]
pub fn fingerprint(source: &str) -> String {
    let mut out = String::with_capacity(source.len().min(FINGERPRINT_MAX_BYTES));
    let mut chars = source.chars().peekable();
    // Tracks the last emitted non-whitespace character, for the regex/division decision.
    let mut previous: Option<char> = None;

    while let Some(c) = chars.next() {
        match c {
            // Strings and templates collapse whole. A template's `${...}` may hold an
            // identifier worth keeping, but it far more often holds an interpolated URL,
            // so the whole literal goes.
            '\'' | '"' | '`' => {
                consume_string(&mut chars, c);
                out.push_str("<str>");
                previous = Some('x');
            }
            '/' => match chars.peek() {
                Some('/') => {
                    for c in chars.by_ref() {
                        if c == '\n' {
                            out.push('\n');
                            previous = Some('\n');
                            break;
                        }
                    }
                }
                Some('*') => {
                    chars.next();
                    let mut last = '\0';
                    for c in chars.by_ref() {
                        if last == '*' && c == '/' {
                            break;
                        }
                        last = c;
                    }
                    out.push(' ');
                }
                _ => {
                    if slash_starts_regex(previous) {
                        consume_regex(&mut chars);
                        out.push_str("<re>");
                        previous = Some('x');
                    } else {
                        out.push('/');
                        previous = Some('/');
                    }
                }
            },
            '0'..='9' => {
                let raw = consume_number(c, &mut chars);
                if is_safe_digit(&raw) {
                    out.push_str(&raw);
                } else {
                    out.push_str("<num>");
                }
                previous = Some('x');
            }
            c if c.is_whitespace() => {
                out.push(c);
                if c == '\n' {
                    previous = Some('\n');
                }
            }
            c => {
                out.push(c);
                previous = Some(c);
            }
        }

        if out.len() >= FINGERPRINT_MAX_BYTES {
            truncate_on_char_boundary(&mut out, FINGERPRINT_MAX_BYTES);
            out.push_str(TRUNCATION_MARKER);
            return out;
        }
    }

    out
}

/// Walks to the closing quote, honouring backslash escapes. Runs to end of input when
/// the literal is unterminated, which is the safe direction: nothing is emitted either way.
fn consume_string(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, quote: char) {
    let mut escaped = false;
    for c in chars.by_ref() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if c == quote {
            return;
        }
    }
}

/// Walks a regex literal to its closing slash, skipping escapes and character classes,
/// then eats trailing flags.
fn consume_regex(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    let mut escaped = false;
    let mut in_class = false;
    while let Some(c) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '[' => in_class = true,
            ']' => in_class = false,
            '/' if !in_class => {
                while let Some(f) = chars.peek() {
                    if f.is_ascii_alphabetic() {
                        chars.next();
                    } else {
                        break;
                    }
                }
                return;
            }
            _ => {}
        }
    }
}

/// Collects a numeric literal, covering hex, binary, octal, floats, exponents, bigint
/// and separators, so no digit-adjacent text escapes as structure.
fn consume_number(first: char, chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut raw = String::from(first);
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
            raw.push(c);
            chars.next();
        } else if (c == '+' || c == '-') && matches!(raw.chars().last(), Some('e') | Some('E')) {
            raw.push(c);
            chars.next();
        } else {
            break;
        }
    }
    raw
}

fn truncate_on_char_boundary(text: &mut String, limit: usize) {
    let mut end = limit.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything a real script might carry that must never reach us. Each entry is a
    /// script plus the substring that would be a leak if it survived.
    const LEAK_CASES: &[(&str, &str)] = &[
        (
            "await browser.input(3).fill(ref, 'dani@example.com');",
            "dani@example.com",
        ),
        (
            "await browser.input(3).fill(pw, \"hunter2-correct-horse\");",
            "hunter2-correct-horse",
        ),
        (
            "const key = 'sk-or-v1-abcdef0123456789';",
            "sk-or-v1-abcdef0123456789",
        ),
        (
            "await browser.upload(3, { path: '/Users/someone/Desktop/taxes.pdf' });",
            "/Users/someone/Desktop/taxes.pdf",
        ),
        (
            "await browser.nav(3).goto('https://shop.test/search?q=pregnancy+test');",
            "pregnancy+test",
        ),
        (
            "// looking for the cheapest flight to Lagos for Amara\nconst pid = 4;",
            "Amara",
        ),
        (
            "/* client is Northwind Bank, do not log */ const pid = 4;",
            "Northwind Bank",
        ),
        (
            "await browser.nav(3).goto(`https://mail.test/u/0/#inbox/${threadId}`);",
            "mail.test",
        ),
        ("const found = /Dr\\. Helena Vasquez/.test(text);", "Helena"),
        (
            "await browser.type('my social is 078-05-1120');",
            "078-05-1120",
        ),
    ];

    #[test]
    fn no_user_content_survives_the_fingerprint() {
        for (script, secret) in LEAK_CASES {
            let out = fingerprint(script);
            assert!(
                !out.contains(secret),
                "leaked {secret:?}\n  script: {script}\n  output: {out}"
            );
        }
    }

    #[test]
    fn structure_that_carries_the_diagnosis_survives() {
        let out = fingerprint(
            "const pid = 20;\nawait browser.nav(pid).goto('https://x.test/?q=secret');\nconst md = await browser.read(pid);",
        );
        for keep in ["const pid", "browser.nav", ".goto", "browser.read", "await"] {
            assert!(out.contains(keep), "lost {keep:?} from {out}");
        }
        assert!(out.contains("<str>"), "url not replaced: {out}");
        assert!(out.contains("<num>"), "page id not replaced: {out}");
    }

    #[test]
    fn the_wrong_call_shape_is_still_visible() {
        let out = fingerprint("await browser.wait(3).forText('Order placed');");
        assert!(out.contains("browser.wait"));
        assert!(out.contains(".forText"));
        assert!(!out.contains("Order placed"));
    }

    #[test]
    fn a_missing_global_is_still_visible() {
        let out = fingerprint("const r = await fetch('https://api.test/me?token=abc');");
        assert!(out.contains("fetch"));
        assert!(!out.contains("token=abc"));
    }

    #[test]
    fn single_digits_survive_because_they_are_structure() {
        let out = fingerprint("md.slice(0, 6000); arr[1];");
        assert!(out.contains("slice(0, <num>)"), "{out}");
        assert!(out.contains("arr[1]"), "{out}");
    }

    #[test]
    fn numeric_forms_never_leak_a_digit_sequence() {
        for script in [
            "const a = 0xDEADBEEF;",
            "const b = 1_234_567;",
            "const c = 1.5e10;",
            "const d = 0b1011;",
            "const e = 99999n;",
        ] {
            let out = fingerprint(script);
            assert!(
                !out.chars()
                    .any(|c| c.is_ascii_digit() && !out.contains("<num>")),
                "{script} -> {out}"
            );
            assert!(out.contains("<num>"), "{script} -> {out}");
        }
    }

    #[test]
    fn division_is_not_mistaken_for_a_regex() {
        let out = fingerprint("const ratio = total / count;");
        assert!(out.contains("total / count"), "{out}");
        assert!(!out.contains("<re>"), "{out}");
    }

    #[test]
    fn an_unterminated_string_cannot_leak_its_tail() {
        let out = fingerprint("const a = 'dani@example.com and the rest of the file");
        assert!(!out.contains("dani@example.com"), "{out}");
    }

    #[test]
    fn a_nested_quote_inside_a_string_does_not_reopen_structure() {
        let out = fingerprint(r#"const a = "she said \"my pin is 4021\" loudly";"#);
        assert!(!out.contains("4021"), "{out}");
    }

    #[test]
    fn long_scripts_are_truncated_and_marked() {
        let script = format!("const x = 1;\n{}", "await browser.read(pid);\n".repeat(400));
        let out = fingerprint(&script);
        assert!(out.len() <= FINGERPRINT_MAX_BYTES + TRUNCATION_MARKER.len());
        assert!(out.ends_with(TRUNCATION_MARKER), "{out}");
    }

    /// The broad guarantee, stated as a property: take every quoted run of text in the
    /// input and assert none of it appears in the output.
    #[test]
    fn no_quoted_span_from_any_input_survives() {
        let scripts = [
            "browser.nav(1).goto('https://a.test/x?y=z'); browser.type('hello world');",
            "const t = `token ${k} end`; const u = \"mixed 'quotes' here\";",
            "browser.grep(2, { pattern: 'Invoice #4471 for Acme' });",
        ];
        for script in scripts {
            let out = fingerprint(script);
            for span in script.split(['\'', '"', '`']).skip(1).step_by(2) {
                let span = span.trim();
                if span.len() < 4 {
                    continue;
                }
                assert!(
                    !out.contains(span),
                    "leaked {span:?} from {script} -> {out}"
                );
            }
        }
    }
}
