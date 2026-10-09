//! Reduces a `run` script to its shape.
//!
//! # Allowlist, not blocklist
//!
//! The output is built from tokens positively recognised as safe. Identifiers, keywords
//! and punctuation are emitted because the agent wrote them and they carry the
//! diagnosis. Literals become placeholders because the user's data lives in literals: the
//! URL, the search term inside it, a password typed into a form. Anything the scanner
//! cannot classify becomes a placeholder too.
//!
//! That direction matters. The first version of this module copied the source and removed
//! what it recognised as a literal, which is a blocklist, and it leaked. It decided
//! regex-versus-division from the preceding character, so `return /Amara Okafor/` looked
//! like division and the name was copied out verbatim. Any keyword before a regex did it:
//! `return`, `typeof`, `case`, `in`, `of`, `delete`, `void`, `new`, `instanceof`.
//!
//! Regex-versus-division is genuinely context dependent, which is why real scanners track
//! the preceding *token* rather than the preceding character. Rather than reimplement
//! that, this uses `ress`, a JavaScript scanner. A scanner and not a parser: a fingerprint
//! needs token kinds, not structure, and a scanner degrades far better on input that does
//! not parse.
//!
//! When scanning fails outright, `sdk_calls_only` takes over and emits nothing but names
//! drawn from our own SDK surface, so an unscannable script still says which API it was
//! using and still cannot carry user text.

use ress::prelude::*;

/// Keeps a fingerprint small enough to stay a fingerprint.
pub const FINGERPRINT_MAX_BYTES: usize = 4_000;

const TRUNCATION_MARKER: &str = "\n/* truncated */";

/// The `browser` SDK surface. The fallback path emits only these, so it cannot carry a
/// name the user chose.
const SDK_SURFACE: &[&str] = &[
    "browser",
    "pages",
    "observe",
    "input",
    "nav",
    "cdp",
    "cdpJsonForPage",
    "read",
    "grep",
    "wait",
    "screenshot",
    "evaluate",
    "download",
    "pdf",
    "upload",
    "tabGroups",
    "windows",
    "saveHelper",
    "listHelpers",
    "readHelper",
    // the page handle, and the browser members that hand one out
    "page",
    "open",
    "history",
    "helpers",
    "id",
    "info",
    "check",
    "uncheck",
    "focus",
    "drag",
    "insertText",
    "clickAt",
    "typeAt",
    "hoverAt",
    "dragAt",
    "dialogAccept",
    "dialogDismiss",
    "waitForSelector",
    "waitForText",
    "waitForTime",
    "save",
    "snapshot",
    "diff",
    "resolveRef",
    "click",
    "fill",
    "type",
    "press",
    "hover",
    "selectOption",
    "scroll",
    "goto",
    "back",
    "forward",
    "reload",
    "newPage",
    "close",
    "list",
    "getInfo",
    "console",
    "sleep",
    "setTimeout",
];

/// Rewrites `source` into a structural fingerprint. Never returns literal text from the
/// input.
#[must_use]
pub fn fingerprint(source: &str) -> String {
    let rendered = match ress::tokenize(source) {
        Ok(tokens) => from_tokens(&tokens),
        // Unscannable. Fall back to the one thing that is safe by construction.
        Err(_) => sdk_calls_only(source),
    };
    cap(rendered)
}

/// Emits one token per recognised kind, and a placeholder for everything else.
fn from_tokens(tokens: &[Token<&str>]) -> String {
    let mut out = String::new();
    let mut previous_was_word = false;
    // Nesting depth inside a substituting template literal. See `Template::Head` below.
    let mut template_depth = 0_u32;
    for token in tokens {
        // A template with `${}` is a string the script is building, so everything
        // interpolated into it is string data by construction: `` `Hello ${user.name}` ``
        // carries the name as surely as a quoted literal would. Emitting `<str>` for the
        // quoted chunks while passing the substitutions through would redact the wrapper
        // and keep the payload, so the whole template collapses to one placeholder.
        // Templates nest, hence the depth count rather than a flag.
        if let Token::Template(template) = token {
            match template {
                Template::Head(_) => {
                    template_depth += 1;
                    if template_depth > 1 {
                        continue;
                    }
                }
                Template::Middle(_) => continue,
                Template::Tail(_) => {
                    template_depth = template_depth.saturating_sub(1);
                    continue;
                }
                Template::NoSub(_) => {
                    if template_depth > 0 {
                        continue;
                    }
                }
            }
        } else if template_depth > 0 {
            continue;
        }
        let (text, is_word) = match token {
            Token::Ident(ident) => (ident.to_string(), true),
            Token::Keyword(keyword) => (keyword.to_string(), true),
            Token::Punct(punct) => (punct.to_string(), false),
            Token::Boolean(value) => (value.to_string(), true),
            Token::Null => ("null".to_string(), true),
            Token::Number(_) => ("<num>".to_string(), true),
            Token::String(_) | Token::Template(_) => ("<str>".to_string(), true),
            Token::RegEx(_) => ("<re>".to_string(), true),
            // Agents narrate intent in comments, so they go entirely.
            Token::Comment(_) => continue,
            Token::EoF => break,
        };
        // Two adjacent words need a separator; punctuation reads better without one.
        if is_word && previous_was_word {
            out.push(' ');
        }
        out.push_str(&text);
        previous_was_word = is_word;
        if out.len() >= FINGERPRINT_MAX_BYTES {
            break;
        }
    }
    out
}

/// Last resort for a script the scanner cannot read.
///
/// Emits only names from `SDK_SURFACE`, in the order they appear. Every emitted byte comes
/// from our own constant rather than from the input, so this cannot leak whatever made the
/// script unscannable.
fn sdk_calls_only(source: &str) -> String {
    let mut found = Vec::new();
    let mut token = String::new();
    for c in source.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
            token.push(c);
            continue;
        }
        if !token.is_empty() {
            if let Some(name) = SDK_SURFACE.iter().find(|name| **name == token) {
                found.push(*name);
            }
            token.clear();
        }
    }
    if found.is_empty() {
        return "/* unscannable script, no SDK calls recognised */".to_string();
    }
    format!(
        "/* unscannable script; SDK calls seen: {} */",
        found.join(", ")
    )
}

fn cap(mut text: String) -> String {
    if text.len() <= FINGERPRINT_MAX_BYTES {
        return text;
    }
    let mut end = FINGERPRINT_MAX_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push_str(TRUNCATION_MARKER);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything a real script might carry that must never reach us. Each entry is a
    /// script plus the substring that would be a leak if it survived.
    const LEAK_CASES: &[(&str, &str)] = &[
        (
            "await browser.input(3).fill(ref, 'person@example.com');",
            "person@example.com",
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

    /// No literal survives, including a single digit.
    ///
    /// The previous version kept `0` through `9` on the grounds that a single digit
    /// cannot identify anyone, which is true, and which made the rule "no literals,
    /// except sometimes". Every leak in this module has come from an exception in the
    /// redaction path, so the exception is gone. `slice(<num>, <num>)` is marginally less
    /// readable than `slice(0, <num>)` and the rule is now one sentence with no caveat.
    #[test]
    fn no_number_survives_however_small() {
        let out = fingerprint("md.slice(0, 6000); arr[1];");
        assert!(!out.contains('0'), "{out}");
        assert!(!out.contains('1'), "{out}");
        assert!(out.contains("slice(<num>,<num>)"), "{out}");
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
        assert!(out.contains("total/count"), "{out}");
        assert!(!out.contains("<re>"), "{out}");
    }

    /// The leak this module was rewritten for. A regex after a keyword read as division
    /// under the old character-based heuristic, and its body was copied out verbatim.
    #[test]
    fn a_regex_after_a_keyword_is_redacted() {
        const CASES: &[&str] = &[
            "return /Amara Okafor/;",
            "const a = typeof /Amara Okafor/;",
            "if (x) return /Amara Okafor/.test(s);",
            "for (const k in /Amara Okafor/.source) {}",
            "const r = new RegExp(/Amara Okafor/);",
            "const y = x instanceof /Amara Okafor/;",
            "switch (v) { case /Amara Okafor/.source: break; }",
            "void /Amara Okafor/;",
            "delete o[/Amara Okafor/.source];",
        ];
        for script in CASES {
            let out = fingerprint(script);
            assert!(!out.contains("Amara"), "leaked from {script}\n  -> {out}");
            assert!(
                out.contains("<re>"),
                "regex not marked in {script}\n  -> {out}"
            );
        }
    }

    /// Identifiers carrying digits used to be split by the hand-rolled number lexer, so
    /// `p1.filter(...)` became `p<num>(...)` and the property access vanished with it.
    /// A substituting template is a string the script is building. The wrapper alone is
    /// not enough: `` `Hi ${name}` `` puts the name in the string just as a quote would,
    /// and a nested template must not reopen a path out of the placeholder.
    #[test]
    fn nothing_interpolated_into_a_template_survives() {
        let cases = [
            "const a = `Hello ${userName} from ${cityName}`;",
            "const a = `outer ${`inner ${userName}`} tail`;",
            "await browser.nav(1).goto(`https://h.test/?q=${userName}`);",
            "const a = `${'AmaraOkafor'}`;",
            "const a = `${/AmaraOkafor/.source}`;",
        ];
        for case in cases {
            let out = fingerprint(case);
            for leaked in ["userName", "cityName", "AmaraOkafor", "inner", "outer"] {
                assert!(
                    !out.contains(leaked),
                    "{leaked} leaked\n  in:  {case}\n  out: {out}"
                );
            }
        }
        // Structure outside the template is untouched.
        assert_eq!(
            fingerprint("await browser.nav(1).goto(`https://h.test/?q=${q}`);"),
            "await browser.nav(<num>).goto(<str>);"
        );
    }

    #[test]
    fn identifiers_containing_digits_survive_intact() {
        let out = fingerprint("const p1 = x; return p1.filter(r => r.ok);");
        assert!(out.contains("p1.filter"), "{out}");
        assert!(!out.contains("p<num>"), "{out}");
    }

    /// A script the scanner cannot read still reports which API it was using, and every
    /// byte of that comes from our own constant rather than from the input.
    #[test]
    fn an_unscannable_script_reports_sdk_calls_and_nothing_else() {
        let out = fingerprint("const a = 'unterminated; await browser.read(2); @@@ !!! }{");
        assert!(out.contains("unscannable"), "{out}");
        assert!(!out.contains("unterminated"), "{out}");
        assert!(!out.contains("@@@"), "{out}");
    }

    #[test]
    fn an_unterminated_string_cannot_leak_its_tail() {
        let out = fingerprint("const a = 'person@example.com and the rest of the file");
        assert!(!out.contains("person@example.com"), "{out}");
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
