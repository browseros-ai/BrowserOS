use std::error::Error;
use std::fmt;

use regress::{Flags, Regex};

/// Failure compiling a JavaScript regular expression or validating its flag set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegexError {
    message: String,
}

impl fmt::Display for RegexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for RegexError {}

/// A compiled ECMAScript `RegExp` behind this crate's own seam so downstream crates never depend
/// on `regress` directly.
///
/// Unflagged patterns compile and match over UTF-16 code units (UCS-2), where supplementary
/// characters such as emoji occupy two surrogate code units. Patterns with `u` or `v` compile and
/// match over Unicode scalar values (code points). Note: `regress 0.12.0` applies case folding
/// after class-set negation/subtraction under `/iv`, so patterns like `/\P{Lowercase_Letter}/iv`
/// or `/[[A-Z]--[a]]/iv` diverge from V8's ES2024 operand-level folding.
#[derive(Clone, Debug)]
pub struct JsRegex {
    pattern: String,
    flags: String,
    sticky: bool,
    unicode_mode: bool,
    inner: Regex,
}

impl PartialEq for JsRegex {
    fn eq(&self, other: &Self) -> bool {
        self.pattern == other.pattern && self.flags == other.flags
    }
}

impl Eq for JsRegex {}

struct ParsedFlags {
    canonical: String,
    sticky: bool,
    unicode_mode: bool,
    regress_flags: Flags,
}

impl JsRegex {
    /// Compiles `pattern` with default unflagged ECMAScript UCS-2 semantics (`flags = ""`).
    pub fn compile(pattern: &str) -> Result<Self, RegexError> {
        Self::compile_with_flags(pattern, "")
    }

    /// Compiles `pattern` with ECMAScript `RegExp` `flags` (any unique subset of `dgimsuvy`,
    /// rejecting unknown characters, duplicates, and `u` combined with `v`).
    pub fn compile_with_flags(pattern: &str, flags: &str) -> Result<Self, RegexError> {
        let parsed = parse_flags(flags)?;
        let inner = if parsed.unicode_mode {
            Regex::with_flags(pattern, parsed.regress_flags)
        } else {
            Regex::from_unicode(pattern.encode_utf16().map(u32::from), parsed.regress_flags)
        }
        .map_err(|error| RegexError {
            message: error.to_string(),
        })?;
        Ok(Self {
            pattern: pattern.to_owned(),
            flags: parsed.canonical,
            sticky: parsed.sticky,
            unicode_mode: parsed.unicode_mode,
            inner,
        })
    }

    /// The pattern source passed to compilation.
    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// The canonical sorted flags string (matching `RegExp.prototype.flags`).
    #[must_use]
    pub fn flags(&self) -> &str {
        &self.flags
    }

    /// Reports whether this regular expression matches `text`, reproducing Playwright's
    /// `re.lastIndex = 0; Boolean(text.match(re))`.
    ///
    /// Sticky (`y`) patterns require the first match to begin at offset 0.
    #[must_use]
    pub fn is_match(&self, text: &str) -> bool {
        let first = if self.unicode_mode {
            self.inner.find(text)
        } else {
            let units: Vec<u16> = text.encode_utf16().collect();
            self.inner.find_from_ucs2(&units, 0).next()
        };
        if self.sticky {
            first.is_some_and(|matched| matched.start() == 0)
        } else {
            first.is_some()
        }
    }
}

// In `regress`, `Flags::unicode` enables Unicode mode (`\u{...}`, strict escapes, Unicode case
// folding) while `Flags::unicode_sets` enables `/v` class-set syntax. Because ES2024's `/v` flag
// implies full Unicode mode (and is mutually exclusive with `/u`), we set both `unicode` and
// `unicode_sets` on `regress::Flags` when `v` is present.
fn parse_flags(flags: &str) -> Result<ParsedFlags, RegexError> {
    let mut has_indices = false;
    let mut global = false;
    let mut icase = false;
    let mut multiline = false;
    let mut dot_all = false;
    let mut unicode = false;
    let mut unicode_sets = false;
    let mut sticky = false;

    for flag in flags.chars() {
        let slot = match flag {
            'd' => &mut has_indices,
            'g' => &mut global,
            'i' => &mut icase,
            'm' => &mut multiline,
            's' => &mut dot_all,
            'u' => &mut unicode,
            'v' => &mut unicode_sets,
            'y' => &mut sticky,
            _ => {
                return Err(RegexError {
                    message: format!("Invalid flags supplied to RegExp constructor '{flags}'"),
                });
            }
        };
        if *slot {
            return Err(RegexError {
                message: format!("Invalid flags supplied to RegExp constructor '{flags}'"),
            });
        }
        *slot = true;
    }

    if unicode && unicode_sets {
        return Err(RegexError {
            message: format!("Invalid flags supplied to RegExp constructor '{flags}'"),
        });
    }

    let mut canonical = String::with_capacity(flags.len());
    for (enabled, ch) in [
        (has_indices, 'd'),
        (global, 'g'),
        (icase, 'i'),
        (multiline, 'm'),
        (dot_all, 's'),
        (unicode, 'u'),
        (unicode_sets, 'v'),
        (sticky, 'y'),
    ] {
        if enabled {
            canonical.push(ch);
        }
    }

    let unicode_mode = unicode || unicode_sets;
    let regress_flags = Flags {
        icase,
        multiline,
        dot_all,
        no_opt: false,
        unicode: unicode_mode,
        unicode_sets,
    };

    Ok(ParsedFlags {
        canonical,
        sticky,
        unicode_mode,
        regress_flags,
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use rquickjs::{CatchResultExt, Context, Runtime};

    use super::JsRegex;

    #[test]
    fn canonicalizes_and_validates_flags() -> Result<(), Box<dyn Error>> {
        let re = JsRegex::compile_with_flags("abc", "yigmsd")?;
        assert_eq!(re.pattern(), "abc", "pattern preserves source verbatim");
        assert_eq!(
            re.flags(),
            "dgimsy",
            "flags sort alphabetically like RegExp.prototype.flags"
        );

        let same = JsRegex::compile_with_flags("abc", "dgimsy")?;
        assert_eq!(
            re, same,
            "equal patterns with equivalent flag sets compare equal"
        );

        for invalid in ["ii", "gg", "uv", "vu", "z", "I", " "] {
            assert!(
                JsRegex::compile_with_flags("abc", invalid).is_err(),
                "invalid flag string {invalid:?} must be rejected"
            );
        }
        Ok(())
    }

    #[test]
    fn matches_ucs2_unicode_unicode_sets_and_sticky_like_quickjs() -> Result<(), Box<dyn Error>> {
        let cases: &[(&str, &str, &str)] = &[
            ("", "", ""),
            ("", "", "abc"),
            ("^.$", "", "😀"),
            ("^..$", "", "😀"),
            ("^.$", "u", "😀"),
            ("^..$", "u", "😀"),
            ("Click me", "v", "Click me"),
            ("click me", "iv", "Click me"),
            ("foo", "y", "foo bar"),
            ("foo", "y", "bar foo"),
            ("foo", "gy", "foo bar"),
            ("foo", "gy", "bar foo"),
            ("foo", "my", "bar\nfoo"),
            ("^foo", "m", "bar\nfoo"),
            ("^foo", "my", "bar\nfoo"),
            ("^foo", "my", "foo\nbar"),
            ("a.b", "", "a\nb"),
            ("a.b", "s", "a\nb"),
            ("a|b", "y", "ba"),
            ("hello", "dgimsuy", "HELLO world"),
            ("hello", "dgimsvy", "HELLO world"),
        ];

        let runtime = Runtime::new()?;
        let context = Context::full(&runtime)?;
        let input = serde_json::to_string(&cases)?;
        let expected_json: String = context.with(|ctx| -> Result<String, String> {
            ctx.globals()
                .set("input", input)
                .catch(&ctx)
                .map_err(|caught| caught.to_string())?;
            ctx.eval::<String, _>(
                r"JSON.stringify(JSON.parse(input).map(([pattern, flags, text]) => {
                    const re = new RegExp(pattern, flags);
                    re.lastIndex = 0;
                    return Boolean(text.match(re));
                }))",
            )
            .catch(&ctx)
            .map_err(|caught| caught.to_string())
        })?;
        let expected: Vec<bool> = serde_json::from_str(&expected_json)?;
        assert_eq!(expected.len(), cases.len(), "one QuickJS answer per case");

        for (&(pattern, flags, text), &js_match) in cases.iter().zip(&expected) {
            let compiled = JsRegex::compile_with_flags(pattern, flags)?;
            assert_eq!(
                compiled.is_match(text),
                js_match,
                "/{pattern}/{flags} against {text:?} must match QuickJS"
            );
        }
        Ok(())
    }

    #[test]
    fn enforces_es2024_unicode_mode_under_v_flag() -> Result<(), Box<dyn Error>> {
        // ES2024 `/v` implies full Unicode mode (code-point matching, `\u{...}` escapes, class-set
        // operations, and strict identity-escape rejection), matching V8/Chromium. Bundled
        // QuickJS-NG 0.12.1 implements `LRE_FLAG_UNICODE_SETS` only as a partial stub in
        // `libregexp.c` without `is_unicode` or set operators, so these assertions check the
        // ES2024/V8 behavior directly.
        assert!(
            JsRegex::compile_with_flags("^.$", "v")?.is_match("😀"),
            "/^.$/v must treat a supplementary emoji as one Unicode code point"
        );
        assert!(
            JsRegex::compile_with_flags(r"\u{1F600}", "v")?.is_match("😀"),
            "/v must support \\u{{...}} code-point escapes"
        );
        assert!(
            JsRegex::compile_with_flags(r"[[a-z]--[aeiou]]", "v")?.is_match("b"),
            "/v class-set subtraction must match consonant 'b'"
        );
        assert!(
            !JsRegex::compile_with_flags(r"[[a-z]--[aeiou]]", "v")?.is_match("e"),
            "/v class-set subtraction must exclude vowel 'e'"
        );
        assert!(
            JsRegex::compile_with_flags(r"\a", "v").is_err(),
            "/v implies Unicode mode so identity escape \\a must be rejected"
        );
        assert!(
            JsRegex::compile_with_flags(r"\a", "").is_ok(),
            "unflagged Annex B allows identity escape \\a"
        );
        Ok(())
    }
}
