//! Playwright's `urlMatches` (`packages/isomorphic/urlMatch.ts` at 26137face) for the `url` a script
//! passes to `waitForURL`. neo-sdk sends it as Playwright's own `SerializedURLMatch` without
//! `urlPattern` (the realm has no `URLPattern`): a glob, where `''` matches every URL, or a
//! `RegExp`'s `source` and `flags`. Neo has no `baseURL` option, so a glob resolves against none.
//!
//! A glob becomes a regex source here, by a port of Playwright's `resolveGlobBase` and
//! `globToRegexPattern` over the `url` crate's WHATWG parser. `tests/url-match-goldens.mjs` runs
//! Playwright's own code over a table of globs and URLs into
//! `tests/fixtures/url-match-goldens.json`, and `tests_that_the_goldens_match` replays it.
//!
//! Every pattern, a glob's or a `RegExp`'s, is tested in a scratch QuickJS context of its own:
//! the engine the script's `RegExp` lives in, so a match here is what `new RegExp(…).test(url)`
//! answers in the realm. Neither alternative is exact: the realm has no `URL` global, so
//! Playwright's code would skip URL normalization there, and Rust's `regex` crate is not
//! JavaScript's dialect.
//!
//! A test runs synchronously on the caller's thread, a tokio worker during a wait. Its interrupt
//! handler stops any test that runs past [`TEST_BUDGET`] (QuickJS's regexp engine polls it every
//! 10,000 steps), so a catastrophic pattern fails the wait with an `Error` instead of pinning the
//! worker. Playwright's would block its event loop.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rquickjs::context::intrinsic;
use rquickjs::function::This;
use rquickjs::{
    CatchResultExt, CaughtError, Coerced, Constructor, Context, FromJs, Function, Object, Runtime,
};
use serde::Deserialize;
use url::Url;

use crate::progress::PlaywrightError;

/// How long one test of one URL may run. A pattern that needs more is catastrophic: a URL
/// pattern answers in microseconds.
const TEST_BUDGET: Duration = Duration::from_millis(100);
/// A compiled pattern, its backtracking stack and a URL (at most 2 MiB in Chromium) fit easily.
const MEMORY_LIMIT_BYTES: usize = 16 * 1024 * 1024;
/// As the realm's: well under a tokio worker's 2 MiB native stack.
const MAX_STACK_BYTES: usize = 512 * 1024;
/// The global the compiled `RegExp` lives under in its scratch context.
const REGEXP_GLOBAL: &str = "regexp";
/// How much of a pattern or URL an error message quotes: a reply and its trace record carry the
/// message, and a script may pass a 40 KB glob or wait on a 2 MiB `data:` URL.
const QUOTED_CHARS: usize = 200;

/// Playwright's `escapedChars` (`urlMatch.ts:28`).
const ESCAPED_CHARS: [char; 14] = [
    '$', '^', '+', '.', '*', '(', ')', '|', '\\', '?', '{', '}', '[', ']',
];
/// Globs `resolveGlobBase` leaves unparsed: these URLs are never relative to a base URL.
const UNRESOLVED_PREFIXES: [&str; 5] = ["about:", "data:", "chrome:", "edge:", "file:"];

/// Playwright's `SerializedURLMatch` without `urlPattern`: exactly one of `glob` or `regexSource`
/// (with `regexFlags`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SerializedUrlMatch {
    pub(crate) glob: Option<String>,
    pub(crate) regex_source: Option<String>,
    pub(crate) regex_flags: Option<String>,
}

/// One compiled `waitForURL` match. `Send + Sync`: a wait holds it across awaits.
pub(crate) struct UrlMatcher {
    glob: Option<String>,
    /// `None` for the `''` glob, which matches every URL.
    regexp: Option<Regexp>,
}

impl UrlMatcher {
    /// Compiles the match the SDK sent.
    ///
    /// # Errors
    ///
    /// An invalid glob (Playwright's `Invalid glob pattern …`) or `RegExp`, or a match that is
    /// neither a glob nor a `RegExp`.
    pub(crate) fn new(wire: &SerializedUrlMatch) -> Result<Self, PlaywrightError> {
        match (&wire.glob, &wire.regex_source) {
            // `urlMatches` answers `true` for `''` before it builds any pattern.
            (Some(glob), None) if glob.is_empty() => Ok(Self {
                glob: Some(String::new()),
                regexp: None,
            }),
            (Some(glob), None) => Ok(Self {
                glob: Some(glob.clone()),
                regexp: Some(Regexp::new(
                    &resolve_glob_to_regex_pattern(glob)?,
                    "",
                    format!("glob {}", quoted(glob)),
                )?),
            }),
            (None, Some(source)) => {
                let flags = wire.regex_flags.as_deref().unwrap_or_default();
                Ok(Self {
                    glob: None,
                    regexp: Some(Regexp::new(
                        source,
                        flags,
                        format!("RegExp /{}/{flags}", shortened(source)),
                    )?),
                })
            }
            _ => Err(PlaywrightError::error(
                "url: expected exactly one of glob or regexSource",
            )),
        }
    }

    /// Whether `url` matches, as Playwright's `urlMatches(undefined, url, match)`.
    ///
    /// # Errors
    ///
    /// The `RegExp` engine failed or overran its budget on `url`.
    pub(crate) fn matches(&self, url: &str) -> Result<bool, PlaywrightError> {
        self.regexp
            .as_ref()
            .map_or(Ok(true), |regexp| regexp.test(url))
    }

    /// The glob as the script wrote it, for the `waiting for navigation to "<glob>"` log line;
    /// `None` for a `RegExp`, which Playwright does not print.
    pub(crate) fn glob(&self) -> Option<&str> {
        self.glob.as_deref()
    }
}

impl fmt::Debug for UrlMatcher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UrlMatcher")
            .field("glob", &self.glob)
            .field(
                "regexp",
                &self.regexp.as_ref().map(|regexp| &regexp.display),
            )
            .finish()
    }
}

/// One `RegExp`, compiled once in a QuickJS runtime of its own, which nothing else enters.
struct Regexp {
    /// What the script passed, for errors: `glob "<glob>"` or `RegExp /<source>/<flags>`,
    /// shortened.
    display: String,
    budget: Arc<Mutex<Budget>>,
    /// Holds the runtime alive; dropping it frees the whole runtime.
    context: Context,
}

/// The test in progress, shared with the runtime's interrupt handler.
#[derive(Debug, Default)]
struct Budget {
    /// When the running test must stop; `None` between tests.
    deadline: Option<Instant>,
    /// Whether the handler stopped the running test.
    overran: bool,
}

impl Regexp {
    /// Compiles `new RegExp(source, flags)`; `display` names what the script passed.
    fn new(source: &str, flags: &str, display: String) -> Result<Self, PlaywrightError> {
        let engine = |error: rquickjs::Error| {
            PlaywrightError::error(format!("url: QuickJS could not compile {display}: {error}"))
        };
        let runtime = Runtime::new().map_err(engine)?;
        runtime.set_memory_limit(MEMORY_LIMIT_BYTES);
        runtime.set_max_stack_size(MAX_STACK_BYTES);
        let budget = Arc::new(Mutex::new(Budget::default()));
        let interrupt = Arc::clone(&budget);
        runtime.set_interrupt_handler(Some(Box::new(move || {
            // A poisoned lock stops the test: without its deadline, nothing else could.
            interrupt.lock().map_or(true, |mut budget| {
                let stop = budget.deadline.is_some_and(|at| Instant::now() >= at);
                budget.overran |= stop;
                stop
            })
        })));
        let context = Context::custom::<(intrinsic::RegExpCompiler, intrinsic::RegExp)>(&runtime)
            .map_err(engine)?;
        context.with(|ctx| {
            let compiled = (|| {
                let constructor: Constructor<'_> = ctx.globals().get("RegExp")?;
                let regexp: Object<'_> = constructor.construct((source, flags))?;
                ctx.globals().set(REGEXP_GLOBAL, regexp)
            })();
            compiled.catch(&ctx).map_err(|error| {
                PlaywrightError::error(format!(
                    "url: {display} cannot be compiled: {}",
                    caught(error)
                ))
            })
        })?;
        Ok(Self {
            display,
            budget,
            context,
        })
    }

    /// Playwright's `match.lastIndex = 0; return match.test(urlString)`, under the budget.
    fn test(&self, url: &str) -> Result<bool, PlaywrightError> {
        self.context.with(|ctx| {
            self.arm(Some(Instant::now() + TEST_BUDGET));
            let tested = (|| {
                let regexp: Object<'_> = ctx.globals().get(REGEXP_GLOBAL)?;
                regexp.set("lastIndex", 0)?;
                let test: Function<'_> = regexp.get("test")?;
                test.call::<_, bool>((This(regexp), url))
            })()
            .catch(&ctx);
            let overran = self.arm(None);
            tested.map_err(|error| {
                if overran {
                    PlaywrightError::error(format!(
                        "url: {} did not finish testing {} within {}ms",
                        self.display,
                        quoted(url),
                        TEST_BUDGET.as_millis(),
                    ))
                } else {
                    PlaywrightError::error(format!(
                        "url: {} failed on {}: {}",
                        self.display,
                        quoted(url),
                        caught(error),
                    ))
                }
            })
        })
    }

    /// Starts (`Some`) or ends (`None`) a test's budget; returns whether the one ending overran.
    fn arm(&self, deadline: Option<Instant>) -> bool {
        self.budget.lock().map_or(true, |mut budget| {
            let overran = budget.overran;
            *budget = Budget {
                deadline,
                overran: false,
            };
            overran
        })
    }
}

/// `text` cut to [`QUOTED_CHARS`] characters, with `…` when it was longer.
fn shortened(text: &str) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(QUOTED_CHARS).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// [`shortened`] in double quotes, for a glob or a URL.
fn quoted(text: &str) -> String {
    format!("\"{}\"", shortened(text))
}

/// The message of a JS error caught at the context boundary.
fn caught(error: CaughtError<'_>) -> String {
    match error {
        CaughtError::Exception(exception) => exception.message().unwrap_or_default(),
        CaughtError::Value(value) => {
            let ctx = value.ctx().clone();
            Coerced::<String>::from_js(&ctx, value)
                .map_or_else(|error| error.to_string(), |text| text.0)
        }
        CaughtError::Error(error) => error.to_string(),
    }
}

/// Playwright's `resolveGlobToRegexPattern(undefined, glob)` (`urlMatch.ts:199`).
fn resolve_glob_to_regex_pattern(glob: &str) -> Result<String, PlaywrightError> {
    glob_to_regex_pattern(&resolve_glob_base(glob))
}

/// Playwright's `resolveGlobBase(undefined, glob)` (`urlMatch.ts:214`): the literal parts of a
/// URL glob normalized as `new URL()` normalizes them (a trailing `/` after the host, default
/// ports dropped, percent-encoding, a lowercase scheme and host), glob tokens kept as written.
fn resolve_glob_base(glob: &str) -> String {
    if glob.starts_with('*') {
        return glob.to_owned();
    }
    // Escaped `\\?` behaves the same as `?` in Playwright's globs.
    let glob = glob.replace("\\\\?", "?");
    if UNRESOLVED_PREFIXES
        .iter()
        .any(|prefix| glob.starts_with(prefix))
    {
        return glob;
    }
    // Glob symbols may be escaped in the URL and some of them, such as `?`, affect resolution,
    // so tokens that hold them parse as placeholders and are put back afterwards.
    let mut tokens = TokenMap::default();
    let relative_path = glob
        .split('/')
        .enumerate()
        .map(|(index, token)| tokens.substitute(index, token))
        .collect::<Vec<_>>()
        .join("/");
    // Playwright's `resolveBaseURL`: the URL's `href` and `origin`, or the path unchanged when it
    // does not parse. Without a base URL a relative glob never parses.
    let (resolved, case_insensitive) = match Url::parse(&relative_path) {
        Ok(url) => (
            url.as_str().to_owned(),
            Some(url.origin().ascii_serialization()),
        ),
        Err(_) => (relative_path, None),
    };
    tokens
        .0
        .into_iter()
        .fold(resolved, |resolved, (placeholder, original)| {
            // Schema and domain are case-insensitive.
            let lowercase = case_insensitive
                .as_deref()
                .is_some_and(|origin| origin.contains(&placeholder));
            let replacement = if lowercase {
                original.to_lowercase()
            } else {
                original
            };
            // JS `String.prototype.replace` with a string pattern: the first occurrence only.
            resolved.replacen(&placeholder, &replacement, 1)
        })
}

/// Playwright's `tokenMap` (a JS `Map`, so insertion-ordered): placeholder to original token.
#[derive(Default)]
struct TokenMap(Vec<(String, String)>);

impl TokenMap {
    /// One `/`-separated token of the glob, as Playwright's `relativePath` mapper rewrites it.
    fn substitute(&mut self, index: usize, token: &str) -> String {
        if token == "." || token == ".." || token.is_empty() {
            return token.to_owned();
        }
        if index == 0 && token.ends_with(':') {
            // `http*://`: the stand-in scheme has to be a web scheme, so that slashes are
            // inserted after the domain. An explicit scheme stays: it decides that slash.
            if token.contains(['*', '{']) {
                return self.map(token, "http:".to_owned());
            }
            return token.to_owned();
        }
        // Components without glob metacharacters are literal, so they round-trip through the URL
        // parser and are normalized by it.
        if !token.contains(['*', '?', '{', '}', '\\']) {
            return token.to_owned();
        }
        match token.split_once('?') {
            None => self.map(token, format!("$_{index}_$")),
            Some((prefix, query)) => {
                let prefix = self.map(prefix, format!("$_{index}_$"));
                let query = self.map(&format!("?{query}"), format!("?$_{index}_$"));
                format!("{prefix}{query}")
            }
        }
    }

    /// Playwright's `mapToken`: an empty token stays empty and is not recorded.
    fn map(&mut self, original: &str, placeholder: String) -> String {
        if original.is_empty() {
            return String::new();
        }
        match self.0.iter_mut().find(|(key, _)| *key == placeholder) {
            Some((_, value)) => original.clone_into(value),
            None => self.0.push((placeholder.clone(), original.to_owned())),
        }
        placeholder
    }
}

/// Playwright's `globToRegexPattern` (`urlMatch.ts:30`): `*` matches within one path segment,
/// `**` across segments, `{a,b}` either alternative, `\` escapes the next character, and every
/// other character, `?` included, matches itself.
fn glob_to_regex_pattern(glob: &str) -> Result<String, PlaywrightError> {
    let mut tokens = String::from("^");
    let mut in_group = false;
    // The glob character before the current one, as `glob[i - 1]`.
    let mut previous: Option<char> = None;
    let mut chars = glob.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(escaped) = chars.next()
        {
            push_literal(&mut tokens, escaped);
            previous = Some(escaped);
            continue;
        }
        if c == '*' {
            let before = previous;
            let mut star_count = 1;
            while chars.next_if_eq(&'*').is_some() {
                star_count += 1;
            }
            previous = Some('*');
            if star_count == 1 {
                tokens.push_str("([^/]*)");
            } else if chars.next_if_eq(&'/').is_some() {
                // Match either `/…something…/` or `/`.
                tokens.push_str(if before == Some('/') {
                    "((.+/)|)"
                } else {
                    "(.*/)"
                });
                previous = Some('/');
            } else {
                tokens.push_str("(.*)");
            }
            continue;
        }
        match c {
            '{' if in_group => return Err(invalid_glob(glob, "nested '{' is not supported")),
            '{' => {
                in_group = true;
                tokens.push('(');
            }
            '}' if !in_group => return Err(invalid_glob(glob, "unmatched '}'")),
            '}' => {
                in_group = false;
                tokens.push(')');
            }
            ',' if in_group => tokens.push('|'),
            ',' => tokens.push_str("\\,"),
            _ => push_literal(&mut tokens, c),
        }
        previous = Some(c);
    }
    if in_group {
        return Err(invalid_glob(glob, "unmatched '{'"));
    }
    tokens.push('$');
    Ok(tokens)
}

/// `c`, backslash-escaped when it is one of Playwright's `escapedChars`.
fn push_literal(tokens: &mut String, c: char) {
    if ESCAPED_CHARS.contains(&c) {
        tokens.push('\\');
    }
    tokens.push(c);
}

/// Playwright's `Invalid glob pattern ${JSON.stringify(glob)}: …` error.
fn invalid_glob(glob: &str, reason: &str) -> PlaywrightError {
    let quoted = serde_json::to_string(glob).unwrap_or_else(|_| format!("{glob:?}"));
    PlaywrightError::error(format!("Invalid glob pattern {quoted}: {reason}"))
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Instant;

    use serde::Deserialize;

    use super::{SerializedUrlMatch, TEST_BUDGET, UrlMatcher, resolve_glob_to_regex_pattern};
    use crate::progress::ErrorName;

    /// `tests/fixtures/url-match-goldens.json`, written by `tests/url-match-goldens.mjs`.
    #[derive(Deserialize)]
    struct Goldens {
        globs: Vec<GlobGolden>,
        regexes: Vec<RegexGolden>,
    }

    #[derive(Deserialize)]
    struct GlobGolden {
        glob: String,
        /// `resolveGlobToRegexPattern`'s result, absent when it threw.
        pattern: Option<String>,
        /// Its error's message, absent when it returned.
        error: Option<String>,
        #[serde(default)]
        urls: Vec<(String, bool)>,
    }

    #[derive(Deserialize)]
    struct RegexGolden {
        source: String,
        flags: String,
        urls: Vec<(String, bool)>,
    }

    fn goldens() -> Goldens {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/url-match-goldens.json"
        )))
        .unwrap()
    }

    fn glob(glob: &str) -> SerializedUrlMatch {
        SerializedUrlMatch {
            glob: Some(glob.to_owned()),
            ..SerializedUrlMatch::default()
        }
    }

    fn regex(source: &str, flags: &str) -> SerializedUrlMatch {
        SerializedUrlMatch {
            regex_source: Some(source.to_owned()),
            regex_flags: Some(flags.to_owned()),
            ..SerializedUrlMatch::default()
        }
    }

    #[test]
    fn tests_that_the_goldens_match() {
        let goldens = goldens();
        assert!(goldens.globs.len() > 50, "the glob table was not read");
        for golden in &goldens.globs {
            let pattern = resolve_glob_to_regex_pattern(&golden.glob);
            match (&golden.pattern, &golden.error) {
                (Some(expected), None) => {
                    assert_eq!(
                        pattern.as_ref().ok(),
                        Some(expected),
                        "pattern of {:?}",
                        golden.glob
                    );
                }
                (None, Some(expected)) => {
                    let error = pattern.unwrap_err();
                    assert_eq!(&error.message, expected, "error of {:?}", golden.glob);
                    let error = UrlMatcher::new(&glob(&golden.glob)).unwrap_err();
                    assert_eq!(&error.message, expected, "matcher of {:?}", golden.glob);
                    assert_eq!(error.name, ErrorName::Error, "{:?}", golden.glob);
                }
                _ => panic!("{:?}: a golden has a pattern or an error", golden.glob),
            }
            if golden.error.is_none() {
                let matcher = UrlMatcher::new(&glob(&golden.glob)).unwrap();
                assert_eq!(matcher.glob(), Some(golden.glob.as_str()), "glob");
                for (url, expected) in &golden.urls {
                    assert_eq!(
                        matcher.matches(url).unwrap(),
                        *expected,
                        "{:?} against {url:?}",
                        golden.glob
                    );
                }
            }
        }
        assert!(goldens.regexes.len() > 10, "the regex table was not read");
        for golden in &goldens.regexes {
            let matcher = UrlMatcher::new(&regex(&golden.source, &golden.flags)).unwrap();
            assert_eq!(matcher.glob(), None, "a RegExp has no glob");
            // In order, on one matcher: each test starts at `lastIndex` 0.
            for (url, expected) in &golden.urls {
                assert_eq!(
                    matcher.matches(url).unwrap(),
                    *expected,
                    "/{}/{} against {url:?}",
                    golden.source,
                    golden.flags
                );
            }
        }
    }

    #[test]
    fn tests_that_a_matcher_crosses_threads() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<UrlMatcher>();
        let matcher = UrlMatcher::new(&glob("**/done.html")).unwrap();
        let on_another_thread = thread::spawn(move || {
            matcher.matches("http://a.test/done.html").unwrap() && {
                !matcher.matches("http://a.test/x").unwrap()
            }
        });
        assert!(on_another_thread.join().unwrap(), "a moved matcher matches");
    }

    #[test]
    fn tests_that_a_catastrophic_regexp_fails_within_its_budget() {
        let matcher = UrlMatcher::new(&regex("(a+)+$", "")).unwrap();
        let url = format!("http://a.test/{}!", "a".repeat(64));
        let started = Instant::now();
        let error = matcher.matches(&url).unwrap_err();
        assert!(
            started.elapsed() < TEST_BUDGET * 20,
            "stopped after {:?}",
            started.elapsed()
        );
        assert_eq!(error.name, ErrorName::Error, "{}", error.message);
        assert!(
            error.message.contains("did not finish"),
            "{}",
            error.message
        );
        // The budget is per test: the same matcher answers the next URL.
        assert!(!matcher.matches("http://a.test/").unwrap(), "next test");
    }

    #[test]
    fn tests_that_an_invalid_regexp_is_an_error() {
        let error = UrlMatcher::new(&regex("(", "")).unwrap_err();
        assert_eq!(error.name, ErrorName::Error, "{}", error.message);
        assert!(
            error.message.starts_with("url: RegExp /(/ "),
            "{}",
            error.message
        );
        let error = UrlMatcher::new(&regex("a", "gg")).unwrap_err();
        assert_eq!(error.name, ErrorName::Error, "{}", error.message);
    }

    /// QuickJS caps a pattern at 255 capture groups and a glob's every `*` is one, so this valid
    /// glob cannot compile here (V8 takes it). The error names the script's glob, not the regex
    /// built from it, and quotes only its start.
    #[test]
    fn tests_that_a_glob_error_names_the_glob_and_stays_short() {
        let glob = "x*".repeat(300);
        let error = UrlMatcher::new(&super::tests::glob(&glob)).unwrap_err();
        assert_eq!(error.name, ErrorName::Error, "{}", error.message);
        assert!(
            error
                .message
                .starts_with(&format!("url: glob \"{}…\" ", &glob[..200])),
            "{}",
            error.message
        );
        assert!(!error.message.contains("RegExp"), "{}", error.message);
        assert!(error.message.len() < 400, "{}", error.message);
        let source = "(".repeat(1000);
        let error = UrlMatcher::new(&regex(&source, "")).unwrap_err();
        assert!(error.message.len() < 400, "{}", error.message);
    }

    #[test]
    fn tests_that_a_failed_test_quotes_only_the_start_of_its_url() {
        let matcher = UrlMatcher::new(&regex("(a+)+$", "")).unwrap();
        let url = format!("data:text/plain,{}!", "a".repeat(10_000));
        let error = matcher.matches(&url).unwrap_err();
        assert!(
            error.message.contains("did not finish"),
            "{}",
            error.message
        );
        assert!(error.message.len() < 600, "{}", error.message);
        assert!(error.message.contains("aaa…\""), "{}", error.message);
    }

    #[test]
    fn tests_that_a_match_is_exactly_one_of_glob_or_regexp() {
        for wire in [
            SerializedUrlMatch::default(),
            SerializedUrlMatch {
                glob: Some("**".to_owned()),
                regex_source: Some("a".to_owned()),
                regex_flags: Some(String::new()),
            },
        ] {
            let error = UrlMatcher::new(&wire).unwrap_err();
            assert_eq!(error.name, ErrorName::Error, "{wire:?}");
        }
        // `regexFlags` may be absent: `new RegExp(source, undefined)`.
        let wire = SerializedUrlMatch {
            regex_source: Some("done".to_owned()),
            ..SerializedUrlMatch::default()
        };
        assert!(
            UrlMatcher::new(&wire)
                .unwrap()
                .matches("http://a.test/done")
                .unwrap(),
            "no flags"
        );
    }

    #[test]
    fn tests_that_the_wire_shape_decodes() {
        let decoded: SerializedUrlMatch =
            serde_json::from_str(r#"{"regexSource":"done\\.html$","regexFlags":"i"}"#).unwrap();
        assert_eq!(decoded, regex("done\\.html$", "i"), "a RegExp");
        let decoded: SerializedUrlMatch =
            serde_json::from_str(r#"{"glob":"**/done.html"}"#).unwrap();
        assert_eq!(decoded, glob("**/done.html"), "a glob");
    }
}
