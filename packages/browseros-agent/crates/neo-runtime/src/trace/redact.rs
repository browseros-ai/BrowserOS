//! Neo: The one redaction policy for what leaves a run as a record. A record (`RunOutcome::calls`,
//! every [`RunEvent`](crate::RunEvent), the `tracing` line per call) can outlive the run in a
//! store, so nothing an agent typed into a page may reach one by accident. Two rules:
//!
//! - **Params, by kind.** [`summarize`] keeps of each param what its [`ParamKind`] allows. The
//!   spec generates the kinds from Playwright's types (`MethodMeta::params`), and a string at a
//!   path the shape does not list is a secret, so a forged or unknown message keeps no text.
//! - **Free text, by value.** Call logs, error messages, console lines, the return value and the
//!   script are scrubbed of every secret the run registered in its [`RunSecrets`]. The door
//!   registers each `Secret`-kind param and discarded URL part of [`MIN_SECRET_CHARS`] or more,
//!   so `fill`'s call-log
//!   line `fill("hunter22")` reads `fill("[redacted]")` in every record.
//!
//! ```text
//!  the door ── register_secrets ──▶ RunSecrets (record-only)     copy-on-write: a scrub never
//!  (later) neo-extensions ── register_protected ──▶ (protected)   waits on a registration
//!  records ── summarize, Scrubber::scrub ──▶ every tier
//!  what the script sees ── Scrubber::scrub_protected ──▶ protected only
//! ```
//!
//! What the script sees from a Playwright method is never redacted: it replies as Playwright
//! would, `fill`'s line included. Only a protected value, a host
//! secret neo-extensions writes for the agent, is scrubbed there too.
//!
//! What escapes, said plainly: secrets shorter than [`MIN_SECRET_CHARS`] in free text, a secret
//! the page transforms before it is echoed, and a selector built from a secret.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::{io, mem};

use neo_runtime_spec::{ParamKind, ParamShape};
use serde_json::{Map, Value, json};
use url::form_urlencoded;

use crate::quickjs_bridge::GetterField;

/// A secret shorter than this is not searched for in free text: four characters match far too
/// much text by chance. Its own param is still redacted by kind.
/// Deviation: `/packages/protocol/spec/frame.yml` `Frame.commands.fill.parameters.value`: `value: string` Neo scrubs record text only for secrets of at least four characters to avoid accidental matches.
pub(crate) const MIN_SECRET_CHARS: usize = 4;

/// The most secrets one run registers. Past it the run is saturated, and every free text
/// scrubbed after is replaced whole: a script filling thousands of values must not make each
/// scrub search thousands of patterns, and an unsearched secret must not slip through.
/// Deviation: `/packages/protocol/spec/frame.yml` `Frame.commands.fill.parameters.value`: `value: string` Neo replaces free text wholesale after saturation so an unregistered secret cannot escape.
const MAX_SECRETS: usize = 512;

/// A secret is searched for by its first this many characters. A match of a longer secret's
/// start takes the rest of its line too, where the rest of the secret would be.
/// Deviation: `/packages/protocol/spec/frame.yml` `Frame.commands.fill.parameters.value`: `value: string` Neo bounds pattern matching and removes the rest of a line when a cut secret matches.
const MATCH_CHARS: usize = 256;

/// What a match in free text becomes.
/// Deviation: `/packages/protocol/spec/frame.yml` `Frame.commands.fill.parameters.value`: `value: string` Neo blanks secrets in records; Playwright trace params keep the original input.
const REDACTED: &str = "[redacted]";

/// The most bytes a params summary takes as JSON.
/// Deviation: `/packages/protocol/spec/frame.yml` `Frame.commands.fill.parameters.value`: `value: string` Neo caps persisted param summaries; the script still receives the full protocol reply.
pub(crate) const MAX_SUMMARY_BYTES: usize = 4 * 1024;

// The caps on a run's own text in its `RunEnded`, Neo policy: the outcome keeps the script's
// output whole for its caller, while a trace keeps a bounded, scrubbed copy.

/// A return value larger than this as JSON is left out (`value_truncated`).
/// Deviation: `/packages/protocol/spec/tracing.yml` `Tracing.commands.tracingStopChunk` Neo caps the recorded return value to bound persistence; the caller keeps its full value.
pub(crate) const MAX_VALUE_BYTES: usize = 64 * 1024;
/// The script is kept up to this many bytes.
/// Deviation: `/packages/protocol/spec/tracing.yml` `Tracing.commands.tracingStopChunk` Neo keeps a bounded scrubbed script, independent of Playwright trace source resources.
pub(crate) const MAX_SCRIPT_BYTES: usize = 64 * 1024;
/// Each console line is kept up to this many bytes.
/// Deviation: `/packages/protocol/spec/tracing.yml` `Tracing.commands.tracingStopChunk` Neo bounds recorded console lines; the script output returned to the caller is unchanged.
pub(crate) const MAX_LINE_BYTES: usize = 8 * 1024;
/// The run's error message is kept up to this many bytes.
/// Deviation: `/packages/protocol/spec/tracing.yml` `Tracing.commands.tracingStopChunk` Neo bounds recorded errors while preserving the full Playwright error in the reply.
pub(crate) const MAX_ERROR_BYTES: usize = 4 * 1024;

/// How much of each string and collection a summary keeps, from roomy to tight. Params are
/// summarized at the first limits whose summary fits [`MAX_SUMMARY_BYTES`].
/// Deviation: `/packages/protocol/spec/frame.yml` `Frame.commands.fill.parameters.value`: `value: string` Neo progressively summarizes strings and collections instead of recording raw params.
const SUMMARY_LIMITS: [Limits; 4] = [
    Limits {
        text: 1024,
        items: 64,
    },
    Limits {
        text: 256,
        items: 16,
    },
    Limits { text: 64, items: 4 },
    Limits { text: 16, items: 1 },
];

/// The longest form of a searched secret, in bytes: [`MATCH_CHARS`] characters, each at most
/// 12 bytes once escaped (`😀` or `%F0%9F%98%80`). Scanning this far past a cut point
/// finds every match that starts before it.
/// Platform: a Unicode scalar uses at most four UTF-8 bytes, each three bytes when percent encoded (`url::form_urlencoded`).
const MAX_PATTERN_BYTES: usize = MATCH_CHARS * 12;

/// One pass's summary budget. The same traversal tightens these limits until the JSON fits,
/// so every supported param shape follows one bounded redaction policy.
#[derive(Clone, Copy)]
struct Limits {
    /// Bytes per kept string or key.
    text: usize,
    /// Items per array, fields per object.
    items: usize,
}

/// What a trace keeps of one call's params.
pub(crate) struct Summary {
    /// Valid JSON of at most [`MAX_SUMMARY_BYTES`].
    pub(crate) params: Value,
    /// Something was cut to fit.
    pub(crate) truncated: bool,
}

/// What a trace keeps of `params`, given its command's `shapes` (`MethodMeta::params`; empty
/// for a method Neo does not know): each value as its kind allows, strings and collections cut
/// to fit [`MAX_SUMMARY_BYTES`].
pub(crate) fn summarize(params: &Value, shapes: &[ParamShape]) -> Summary {
    for limits in SUMMARY_LIMITS {
        let mut walk = Summarize {
            shapes,
            limits,
            truncated: false,
        };
        let summary = walk.node(params, &mut String::new(), None);
        if json_len(&summary) <= MAX_SUMMARY_BYTES {
            return Summary {
                params: summary,
                truncated: walk.truncated,
            };
        }
    }
    Summary {
        params: Value::Object(Map::new()),
        truncated: true,
    }
}

/// Registers every `Secret`-kind string in `params` as a record-only secret of the run, so the
/// call's log and error, and everything the run records after, are scrubbed of it. Walks every
/// value, also those a summary would cut. URL fields also register their discarded parts so
/// navigation logs and saved scripts cannot bypass the URL field policy.
pub(crate) fn register_secrets(params: &Value, shapes: &[ParamShape], secrets: &RunSecrets) {
    visit_strings(
        params,
        shapes,
        &mut String::new(),
        None,
        &mut |text, kind| match kind {
            ParamKind::Secret => secrets.register(text),
            ParamKind::Url => register_url_secrets(text, secrets),
            ParamKind::Key => {
                if redact_key(text) != text {
                    secrets.register(text);
                }
            }
            ParamKind::Plain | ParamKind::Selector | ParamKind::Binary => {}
        },
    );
}

/// Getter initializers carry live URLs even though their records keep only page ids. Their
/// discarded URL parts must also stay out of later free text, such as a returned initializer.
pub(crate) fn register_getter_secrets(field: GetterField, value: &Value, secrets: &RunSecrets) {
    fn urls(value: &Value, secrets: &RunSecrets) {
        match value {
            Value::Array(items) => {
                for item in items {
                    urls(item, secrets);
                }
            }
            Value::Object(fields) => {
                for (key, value) in fields {
                    if matches!(key.as_str(), "url" | "href")
                        && let Some(url) = value.as_str()
                    {
                        register_url_secrets(url, secrets);
                    } else {
                        urls(value, secrets);
                    }
                }
            }
            _ => {}
        }
    }
    match field {
        GetterField::Url => {
            if let Some(url) = value.as_str() {
                register_url_secrets(url, secrets);
            }
        }
        GetterField::Initializer
        | GetterField::Pages
        | GetterField::Frames
        | GetterField::ParentFrame
        | GetterField::ChildFrames
        | GetterField::Opener
        | GetterField::Chain
        | GetterField::ParseUrl
        | GetterField::SetUrl => urls(value, secrets),
        GetterField::Name | GetterField::Failure => {
            if let Some(text) = value.as_str() {
                secrets.register(text);
            }
        }
        _ => {}
    }
}

/// Bounded getter records keep identities and scalar state, never page text or handle
/// initializers. This is independent of the raw answer sent back into QuickJS.
pub(crate) fn getter_summary(field: GetterField, value: &Value) -> Value {
    fn identity(value: &Value) -> Value {
        match value {
            Value::Null => Value::Null,
            Value::String(id) => Value::String(id.clone()),
            Value::Array(items) => Value::Array(items.iter().map(identity).collect()),
            Value::Object(fields) => fields.get("guid").cloned().unwrap_or(Value::Null),
            Value::Bool(_) | Value::Number(_) => value.clone(),
        }
    }
    let summary = match field {
        GetterField::Url => value
            .as_str()
            .map_or(Value::Null, |url| json!(redact_url(url))),
        GetterField::IsClosed
        | GetterField::IsDetached
        | GetterField::IsConnected
        | GetterField::MatchesUrl
        | GetterField::Version
        | GetterField::Finished
        | GetterField::Timing => value.clone(),
        GetterField::Initializer
        | GetterField::Pages
        | GetterField::Frames
        | GetterField::ParentFrame
        | GetterField::ChildFrames
        | GetterField::RedirectedTo
        | GetterField::ExistingResponse
        | GetterField::Debugger
        | GetterField::Opener
        | GetterField::Args
        | GetterField::Element => identity(value),
        GetterField::Chain => json!({"count": value.as_array().map_or(0, Vec::len)}),
        GetterField::Devices => json!({"count": value.as_object().map_or(0, serde_json::Map::len)}),
        GetterField::PausedDetails => json!({"paused": !value.is_null()}),
        GetterField::RandomValues => {
            json!({"bytes": value.as_str().map_or(0, |s| s.len() * 3 / 4 - s.chars().rev().take_while(|ch| *ch == '=').count())})
        }
        GetterField::Name | GetterField::Failure | GetterField::ParseUrl | GetterField::SetUrl => {
            summarize(value, &[]).params
        }
    };
    if json_len(&summary) > MAX_SUMMARY_BYTES {
        Value::Null
    } else {
        summary
    }
}

/// URL-shaped params also appear verbatim in navigation logs and in the script. Register
/// the discarded parts as record-only secrets, plus the full URL, so the field policy cannot
/// be bypassed by those free-text copies. This never changes the URL sent to the browser.
fn register_url_secrets(text: &str, secrets: &RunSecrets) {
    if redact_url(text) == text {
        return;
    }
    secrets.register(text);
    let part = |value: &str| {
        secrets.register(value);
        // Include decoded query/fragment text too: a script may print the token separately.
        for (_, decoded) in form_urlencoded::parse(format!("v={value}").as_bytes()) {
            secrets.register(&decoded);
        }
    };
    let (url, fragment) = text
        .split_once('#')
        .map_or((text, None), |(url, fragment)| (url, Some(fragment)));
    if let Some(fragment) = fragment {
        part(fragment);
    }
    let (base, query) = url
        .split_once('?')
        .map_or((url, None), |(base, query)| (base, Some(query)));
    if let Some(query) = query {
        for pair in query.split('&') {
            if let Some((_, value)) = pair.split_once('=') {
                part(value);
            }
        }
    }
    if let Some((scheme, body)) = base.split_once(':')
        && is_scheme(scheme)
        && !body.starts_with('/')
        && !scheme.eq_ignore_ascii_case("about")
    {
        part(body);
    }
    let authority = base
        .split_once("://")
        .map(|(_, rest)| rest)
        .or_else(|| base.strip_prefix("//"));
    if let Some(authority) = authority
        && let Some((userinfo, _)) = authority
            .split('/')
            .next()
            .unwrap_or_default()
            .rsplit_once('@')
    {
        for value in userinfo.split(':') {
            part(value);
        }
    }
}

/// A URL with what may identify a person or grant access taken out: userinfo, the fragment and
/// query values go, while the scheme, host, path and query keys stay, so a trace still says
/// where a call went. An opaque URL (`data:`, `javascript:`) keeps only its scheme: its body is
/// content. Text that is not a URL, such as a glob, keeps the same parts by the same rules.
pub(crate) fn redact_url(url: &str) -> String {
    let url = url.split_once('#').map_or(url, |(before, _)| before);
    let (base, query) = match url.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (url, None),
    };
    if let Some((scheme, body)) = base.split_once(':')
        && is_scheme(scheme)
        && !body.starts_with('/')
        && !scheme.eq_ignore_ascii_case("about")
    {
        let body_chars = url.chars().count() - scheme.chars().count() - 1;
        return format!("{scheme}:[redacted {body_chars}]");
    }
    let mut redacted = without_userinfo(base);
    if let Some(query) = query {
        redacted.push('?');
        let pairs: Vec<String> = query
            .split('&')
            .map(|pair| match pair.split_once('=') {
                Some((key, _)) => format!("{key}={REDACTED}"),
                None if pair.is_empty() => String::new(),
                None => format!("{pair}={REDACTED}"),
            })
            .collect();
        redacted.push_str(&pairs.join("&"));
    }
    redacted
}

/// A key for `press`, as a trace keeps it: named keys stay (`Enter`, `Shift+ArrowLeft`), and so
/// does a shortcut, which types nothing (`Control+A`); a character it would type does not
/// (`Shift+[redacted 1]`).
pub(crate) fn redact_key(key: &str) -> String {
    let tokens = key_tokens(key);
    let shortcut = tokens
        .iter()
        .any(|token| matches!(token.as_str(), "Control" | "Alt" | "Meta" | "ControlOrMeta"));
    if shortcut {
        return key.to_owned();
    }
    let tokens: Vec<String> = tokens
        .into_iter()
        .map(|token| {
            if is_named_key(&token) {
                token
            } else {
                redacted(&token)
            }
        })
        .collect();
    tokens.join("+")
}

/// `text` cut to at most `max` bytes, on a character boundary.
pub(crate) fn cut(text: &str, max: usize) -> &str {
    &text[..text.floor_char_boundary(max)]
}

/// The run's secrets, in two tiers. **Record-only** values (a script's own `fill` value) are
/// scrubbed from every record and never from what the script sees: the script wrote them.
/// **Protected** values (a host secret neo-extensions writes for the agent) are scrubbed from
/// both. One per run, shared by everything that records.
///
/// Copy-on-write: a registration swaps in a new [`Scrubber`], and a scrub works on the one it
/// took, so scrubbing never holds the lock that the door, which registers on the realm's
/// thread, needs.
#[derive(Default)]
pub(crate) struct RunSecrets(Mutex<Scrubber>);

impl RunSecrets {
    /// Registers a record-only secret.
    pub(crate) fn register(&self, secret: &str) {
        self.add(secret, false);
    }

    /// Registers a protected secret, which [`Scrubber::scrub_protected`] also removes.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "neo-extensions registers host secrets here before it writes one"
        )
    )]
    pub(crate) fn register_protected(&self, secret: &str) {
        self.add(secret, true);
    }

    /// The secrets as registered now. A secret registered later is not in it.
    pub(crate) fn scrubber(&self) -> Scrubber {
        // A poisoned lock means a registration panicked; the set it held is still whole, since
        // the new one is swapped in last, but it cannot be read. Treat the run as saturated, so
        // nothing that might hold a secret is recorded.
        self.0
            .lock()
            .map_or_else(|_| Scrubber::saturated(), |current| current.clone())
    }

    fn add(&self, secret: &str, protected: bool) {
        let chars = secret.chars().count();
        if chars < MIN_SECRET_CHARS {
            return;
        }
        let start = cut_chars(secret, MATCH_CHARS);
        let is_cut = chars > MATCH_CHARS;
        let Ok(mut current) = self.0.lock() else {
            return;
        };
        if let Some(next) = current.0.with(start, is_cut, protected) {
            *current = Scrubber(Arc::new(next));
        }
    }
}

/// One snapshot of a run's secrets, which scrubs text of them. Cheap to clone.
#[derive(Clone, Default)]
pub(crate) struct Scrubber(Arc<SecretSet>);

/// Immutable patterns shared by scrubbers. Registration replaces the snapshot, leaving
/// existing readers free to scan without the run's registration lock.
#[derive(Clone, Default)]
struct SecretSet {
    /// Every form of every secret, by first byte, and longest first within one first byte, so
    /// [`SecretSet::match_end`] finds the longest match at a position in one short run.
    patterns: Vec<Pattern>,
    /// Raw prefixes identify registrations separately from their encoded forms. Treating an
    /// encoded form as an existing raw secret would miss its double-encoded form.
    originals: Vec<(Arc<str>, bool)>,
    /// Secrets registered.
    count: usize,
    /// A secret was refused at [`MAX_SECRETS`]: records are replaced whole from now on.
    saturated: bool,
    /// A protected secret was refused: so is what the script sees.
    protected_saturated: bool,
}

#[derive(Clone)]
struct Pattern {
    text: Arc<str>,
    /// Which registered secret this is a form of.
    secret: usize,
    /// The secret was longer than [`MATCH_CHARS`], so this is its start, and a match takes the
    /// rest of its line.
    cut: bool,
    protected: bool,
}

impl Scrubber {
    /// A scrubber that replaces every text whole, for a run whose secrets cannot be read.
    fn saturated() -> Self {
        Self(Arc::new(SecretSet {
            saturated: true,
            protected_saturated: true,
            ..SecretSet::default()
        }))
    }

    /// `text` for a record: every registered secret replaced, in each of the forms text can
    /// carry it in.
    pub(crate) fn scrub<'t>(&self, text: &'t str) -> Cow<'t, str> {
        self.0.scan(text, false)
    }

    /// `text` for the script: only protected secrets replaced.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "neo-extensions scrubs what crosses into a run that wrote a protected value"
        )
    )]
    pub(crate) fn scrub_protected<'t>(&self, text: &'t str) -> Cow<'t, str> {
        self.0.scan(text, true)
    }

    /// [`Self::scrub`] of `text`, cut to at most `max` bytes. Only as much of `text` is searched
    /// as a match starting before the cut can span, so a huge text costs no more than a short
    /// one.
    pub(crate) fn scrub_cut(&self, text: &str, max: usize) -> String {
        if self.0.saturated {
            return redacted(text);
        }
        let window = cut(text, max.saturating_add(MAX_PATTERN_BYTES));
        cut(&self.scrub(window), max).to_owned()
    }

    /// A JSON value for a record: every string and key scrubbed.
    pub(crate) fn scrub_value(&self, value: &Value) -> Value {
        match value {
            Value::String(text) => Value::String(self.scrub(text).into_owned()),
            Value::Array(items) => {
                Value::Array(items.iter().map(|item| self.scrub_value(item)).collect())
            }
            Value::Object(fields) => Value::Object(
                fields
                    .iter()
                    .map(|(key, field)| (self.scrub(key).into_owned(), self.scrub_value(field)))
                    .collect(),
            ),
            Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        }
    }
}

impl SecretSet {
    /// This set with `start` (a secret's first [`MATCH_CHARS`] characters) added in its three
    /// forms, or `None` when nothing changes.
    fn with(&self, start: &str, is_cut: bool, protected: bool) -> Option<SecretSet> {
        let known = self
            .originals
            .iter()
            .position(|(text, cut)| **text == *start && *cut == is_cut)
            .map(|secret| {
                (
                    secret,
                    self.patterns
                        .iter()
                        .any(|pattern| pattern.secret == secret && pattern.protected),
                )
            });
        match known {
            Some((_, was_protected)) if was_protected || !protected => None,
            // A record-only secret registered again as protected now protects its forms.
            Some((secret, _)) => {
                let mut next = self.clone();
                for pattern in &mut next.patterns {
                    pattern.protected |= pattern.secret == secret;
                }
                Some(next)
            }
            None if self.count >= MAX_SECRETS => {
                let refused = self.saturated && (self.protected_saturated || !protected);
                (!refused).then(|| SecretSet {
                    saturated: true,
                    protected_saturated: self.protected_saturated || protected,
                    ..self.clone()
                })
            }
            None => {
                let mut next = self.clone();
                let secret = next.count;
                next.count += 1;
                next.originals.push((Arc::from(start), is_cut));
                let mut forms = vec![start.to_owned(), json_escaped(start), url_encoded(start)];
                forms.sort_unstable();
                forms.dedup();
                next.patterns.extend(forms.into_iter().map(|form| Pattern {
                    text: Arc::from(form),
                    secret,
                    cut: is_cut,
                    protected,
                }));
                next.patterns.sort_by(|a, b| {
                    let first = |pattern: &Pattern| pattern.text.as_bytes().first().copied();
                    first(a)
                        .cmp(&first(b))
                        .then(b.text.len().cmp(&a.text.len()))
                });
                Some(next)
            }
        }
    }

    /// `text` with every applicable match replaced by [`REDACTED`]. Overlapping and adjacent
    /// matches merge into one, so two secrets that share text leave none of either behind.
    fn scan<'t>(&self, text: &'t str, protected_only: bool) -> Cow<'t, str> {
        let saturated = if protected_only {
            self.protected_saturated
        } else {
            self.saturated
        };
        if saturated {
            return Cow::Owned(redacted(text));
        }
        if self.patterns.is_empty() {
            return Cow::Borrowed(text);
        }
        let mut scrubbed = String::new();
        let mut copied = 0;
        let mut run: Option<(usize, usize)> = None;
        for at in 0..text.len() {
            if let Some((start, end)) = run
                && at > end
            {
                scrubbed.push_str(&text[copied..start]);
                scrubbed.push_str(REDACTED);
                copied = end;
                run = None;
            }
            if let Some(end) = self.match_end(text, at, protected_only) {
                run = Some(match run {
                    Some((start, until)) => (start, until.max(end)),
                    None => (at, end),
                });
            }
        }
        if let Some((start, end)) = run {
            scrubbed.push_str(&text[copied..start]);
            scrubbed.push_str(REDACTED);
            copied = end;
        }
        if copied == 0 && scrubbed.is_empty() {
            return Cow::Borrowed(text);
        }
        scrubbed.push_str(&text[copied..]);
        Cow::Owned(scrubbed)
    }

    /// Where the furthest-reaching match starting at byte `at` ends, if any. A pattern starts
    /// with a UTF-8 lead byte, so a match can only start on a character boundary.
    fn match_end(&self, text: &str, at: usize, protected_only: bool) -> Option<usize> {
        let first = *text.as_bytes().get(at)?;
        let begin = self
            .patterns
            .partition_point(|pattern| pattern.text.as_bytes()[0] < first);
        let rest = &text.as_bytes()[at..];
        self.patterns[begin..]
            .iter()
            .take_while(|pattern| pattern.text.as_bytes()[0] == first)
            .filter(|pattern| !protected_only || pattern.protected)
            .filter(|pattern| rest.starts_with(pattern.text.as_bytes()))
            .map(|pattern| {
                let end = at + pattern.text.len();
                if pattern.cut {
                    text[end..].find('\n').map_or(text.len(), |line| end + line)
                } else {
                    end
                }
            })
            .max()
    }
}

/// The summary walk at one set of limits.
struct Summarize<'s> {
    shapes: &'s [ParamShape],
    limits: Limits,
    truncated: bool,
}

impl Summarize<'_> {
    fn node(&mut self, value: &Value, path: &mut String, inherited: Option<ParamKind>) -> Value {
        let kind = kind_at(self.shapes, path, inherited);
        match value {
            Value::String(text) => {
                Value::String(self.text(text, kind.unwrap_or(ParamKind::Secret)))
            }
            Value::Array(items) => {
                let inner = inner_kind(kind);
                let mut kept = Vec::new();
                for item in items {
                    if kept.len() == self.limits.items {
                        self.truncated = true;
                        break;
                    }
                    let len = path.len();
                    path.push_str("[]");
                    kept.push(self.node(item, path, inner));
                    path.truncate(len);
                }
                Value::Array(kept)
            }
            Value::Object(fields) => {
                let inner = inner_kind(kind);
                let mut kept = Map::new();
                for (key, field) in fields {
                    if kept.len() == self.limits.items {
                        self.truncated = true;
                        break;
                    }
                    let len = path.len();
                    if !path.is_empty() {
                        path.push('.');
                    }
                    path.push_str(key);
                    let field = self.node(field, path, inner);
                    path.truncate(len);
                    kept.insert(self.keep(key), field);
                }
                Value::Object(kept)
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        }
    }

    fn text(&mut self, text: &str, kind: ParamKind) -> String {
        match kind {
            ParamKind::Secret => redacted(text),
            ParamKind::Binary => format!("[binary {} bytes]", base64_bytes(text)),
            ParamKind::Plain | ParamKind::Selector => self.keep(text),
            ParamKind::Url => self.keep(&redact_url(text)),
            ParamKind::Key => self.keep(&redact_key(text)),
        }
    }

    fn keep(&mut self, text: &str) -> String {
        let kept = cut(text, self.limits.text);
        self.truncated |= kept.len() < text.len();
        kept.to_owned()
    }
}

/// Calls `visit` with every string in `value` and the kind it is read as.
fn visit_strings(
    value: &Value,
    shapes: &[ParamShape],
    path: &mut String,
    inherited: Option<ParamKind>,
    visit: &mut impl FnMut(&str, ParamKind),
) {
    let kind = kind_at(shapes, path, inherited);
    match value {
        Value::String(text) => visit(text, kind.unwrap_or(ParamKind::Secret)),
        Value::Array(items) => {
            let len = path.len();
            path.push_str("[]");
            for item in items {
                visit_strings(item, shapes, path, inner_kind(kind), visit);
            }
            path.truncate(len);
        }
        Value::Object(fields) => {
            for (key, field) in fields {
                let len = path.len();
                if !path.is_empty() {
                    path.push('.');
                }
                path.push_str(key);
                visit_strings(field, shapes, path, inner_kind(kind), visit);
                path.truncate(len);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// The kind of the value at `path`: its shape's, or else what its container passes down.
fn kind_at(shapes: &[ParamShape], path: &str, inherited: Option<ParamKind>) -> Option<ParamKind> {
    shapes
        .iter()
        .find(|shape| shape.path == path)
        .map(|shape| shape.kind)
        .or(inherited)
}

/// What a container of `kind` passes down to its children: a plain one is kept whole, one of
/// any other kind (a type that contains itself, or a value of the wrong type) keeps none of its
/// strings, and one of no kind lets each child's own path decide.
fn inner_kind(kind: Option<ParamKind>) -> Option<ParamKind> {
    kind.map(|kind| match kind {
        ParamKind::Plain => ParamKind::Plain,
        ParamKind::Selector
        | ParamKind::Url
        | ParamKind::Key
        | ParamKind::Secret
        | ParamKind::Binary => ParamKind::Secret,
    })
}

/// A secret as a record shows it: only its length, in characters.
pub(crate) fn redacted(text: &str) -> String {
    format!("[redacted {}]", text.chars().count())
}

/// The bytes base64 `text` decodes to.
fn base64_bytes(text: &str) -> usize {
    text.trim_end_matches('=').len() * 3 / 4
}

/// `text`'s first `max` characters.
fn cut_chars(text: &str, max: usize) -> &str {
    text.char_indices()
        .nth(max)
        .map_or(text, |(index, _)| &text[..index])
}

/// `text` as it reads inside a JSON string, without the quotes.
fn json_escaped(text: &str) -> String {
    let quoted = Value::String(text.to_owned()).to_string();
    quoted[1..quoted.len() - 1].to_owned()
}

/// `text` as JavaScript's `encodeURIComponent` writes it, the form a script builds URLs with.
fn url_encoded(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            let _written = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

/// A URL scheme: a letter, then letters, digits, `+`, `-` and `.`.
fn is_scheme(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// `base` without the `user:password@` of its authority.
fn without_userinfo(base: &str) -> String {
    let authority = if base.starts_with("//") {
        Some(2)
    } else {
        base.find("://")
            .filter(|scheme_end| is_scheme(&base[..*scheme_end]))
            .map(|scheme_end| scheme_end + 3)
    };
    let Some(start) = authority else {
        return base.to_owned();
    };
    let end = base[start..]
        .find('/')
        .map_or(base.len(), |slash| start + slash);
    match base[start..end].rfind('@') {
        Some(at) => format!("{}{}", &base[..start], &base[start + at + 1..]),
        None => base.to_owned(),
    }
}

/// Playwright's `Keyboard.press` split: a `+` ends a token only when the token so far is not
/// empty, so `+` and `Shift++` name the `+` key.
fn key_tokens(key: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    for c in key.chars() {
        if c == '+' && !token.is_empty() {
            tokens.push(mem::take(&mut token));
        } else {
            token.push(c);
        }
    }
    tokens.push(token);
    tokens
}

/// A key name from Playwright's layout, such as `Enter`, `ArrowLeft`, `F1` or `Shift`: more than
/// one character, capitalized, letters and digits only.
fn is_named_key(token: &str) -> bool {
    token.len() > 1
        && token.starts_with(|c: char| c.is_ascii_uppercase())
        && token.chars().all(|c| c.is_ascii_alphanumeric())
}

/// `value`'s length as JSON text, counted without building the text.
pub(crate) fn json_len(value: &Value) -> usize {
    struct Count(usize);
    impl io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    serde_json::to_writer(&mut count, value).map_or(usize::MAX, |()| count.0)
}

#[cfg(test)]
mod tests {
    use crate::playwright_methods::evaluate::FRAME_EVAL_ON_SELECTOR_ALL_META;
    use neo_runtime_spec::Command;
    use neo_runtime_spec::serve::{
        FrameClickParams, FrameFillParams, FrameGetAttributeParams, FrameGotoParams,
        FramePressParams, FrameSelectOptionParams, FrameTextContentParams,
    };
    use serde_json::json;

    use super::*;

    fn summary<C: Command>(params: &Value) -> Value {
        summarize(params, C::META.params).params
    }

    fn scrubber(secrets: &[&str]) -> Scrubber {
        let run = RunSecrets::default();
        for secret in secrets {
            run.register(secret);
        }
        run.scrubber()
    }

    #[test]
    fn expanded_getter_summaries_keep_state_and_ids_without_page_data() {
        let secret = "getter-page-data-do-not-store";
        let record =
            json!({"guid":"handle@run.1", "initializer":{"preview":secret,"value":secret}});
        for field in [
            GetterField::Initializer,
            GetterField::Opener,
            GetterField::ParentFrame,
            GetterField::Element,
        ] {
            assert_eq!(getter_summary(field, &record), json!("handle@run.1"));
        }
        for field in [
            GetterField::Pages,
            GetterField::Frames,
            GetterField::ChildFrames,
            GetterField::Args,
        ] {
            assert_eq!(
                getter_summary(field, &json!([record])),
                json!(["handle@run.1"])
            );
        }
        for field in [
            GetterField::Name,
            GetterField::Failure,
            GetterField::ParseUrl,
            GetterField::SetUrl,
        ] {
            assert!(
                !getter_summary(field, &json!(secret))
                    .to_string()
                    .contains(secret)
            );
        }
        assert_eq!(
            getter_summary(
                GetterField::Chain,
                &json!([{"headers":secret,"url":secret}])
            ),
            json!({"count":1})
        );
        assert_eq!(
            getter_summary(GetterField::PausedDetails, &json!({"params":secret})),
            json!({"paused":true})
        );
        for field in [
            GetterField::IsClosed,
            GetterField::IsDetached,
            GetterField::IsConnected,
            GetterField::MatchesUrl,
        ] {
            assert_eq!(getter_summary(field, &json!(false)), json!(false));
        }
        assert_eq!(
            getter_summary(GetterField::Timing, &json!({"startTime":123})),
            json!({"startTime":123})
        );
        assert_eq!(
            getter_summary(GetterField::RandomValues, &json!("YQ==")),
            json!({"bytes":1})
        );
        let huge = json!([{"guid":"x".repeat(MAX_SUMMARY_BYTES)}]);
        assert_eq!(getter_summary(GetterField::Args, &huge), Value::Null);
    }

    #[test]
    fn a_click_and_a_read_keep_their_params() {
        let click = json!({
            "selector": "internal:role=button[name=\"Go\"i]", "strict": true,
            "position": { "x": 1, "y": 2.5 }, "modifiers": ["Shift"], "button": "left",
            "clickCount": 2, "delay": 10, "trial": true,
        });
        assert_eq!(
            summary::<FrameClickParams>(&click),
            click,
            "a click hides nothing"
        );
        let read = json!({ "selector": "#a", "strict": true });
        assert_eq!(
            summary::<FrameTextContentParams>(&read),
            read,
            "a read hides nothing"
        );
        assert!(
            !summarize(&click, FrameClickParams::META.params).truncated,
            "nothing was cut"
        );
    }

    #[test]
    fn fill_keeps_its_selector_and_only_the_length_of_its_value() {
        let fill =
            json!({ "selector": "#password", "strict": true, "value": "hunter22", "force": true });
        assert_eq!(
            summary::<FrameFillParams>(&fill),
            json!({ "selector": "#password", "strict": true, "value": "[redacted 8]", "force": true })
        );
        let short = json!({ "selector": "#pin", "value": "123" });
        assert_eq!(
            summary::<FrameFillParams>(&short)["value"],
            "[redacted 3]",
            "a value too short to search for in text is still redacted as a param"
        );
    }

    #[test]
    fn urls_keep_where_they_go_and_nothing_that_grants_access() {
        let cases = [
            (
                "https://a.test/path/x?q=1&token=abc#frag",
                "https://a.test/path/x?q=[redacted]&token=[redacted]",
            ),
            ("https://user:pw@a.test:8080/p", "https://a.test:8080/p"),
            ("//user@a.test/p?x", "//a.test/p?x=[redacted]"),
            ("about:blank", "about:blank"),
            ("data:text/html,<p>secret</p>", "data:[redacted 23]"),
            ("javascript:alert(1)", "javascript:[redacted 8]"),
            ("**/login?next=/home", "**/login?next=[redacted]"),
            ("/relative/path", "/relative/path"),
        ];
        for (url, kept) in cases {
            assert_eq!(redact_url(url), kept, "{url}");
        }
        let goto = json!({
            "url": "https://a.test/?code=1", "waitUntil": "load", "referer": "https://b.test/#t",
        });
        assert_eq!(
            summary::<FrameGotoParams>(&goto),
            json!({ "url": "https://a.test/?code=[redacted]", "waitUntil": "load", "referer": "https://b.test/" })
        );
    }

    #[test]
    fn urls_cannot_leak_discarded_parts_through_free_text() {
        let run = RunSecrets::default();
        let url = "https://username:password@a.test/path?token=private%20value#fragment";
        register_secrets(&json!({"url": url}), FrameGotoParams::META.params, &run);
        let text = format!("navigating to {url}; token private value; password; fragment");
        let clean = run.scrubber().scrub(&text).into_owned();
        for secret in [url, "private value", "password", "fragment"] {
            assert!(!clean.contains(secret), "{clean}");
        }
        assert_eq!(
            run.scrubber().scrub_protected(url),
            url,
            "a record-only URL leaves replies untouched"
        );
    }

    #[test]
    fn press_keeps_named_keys_and_shortcuts_but_not_typed_characters() {
        let cases = [
            ("Enter", "Enter"),
            ("Shift+ArrowLeft", "Shift+ArrowLeft"),
            ("Control+A", "Control+A"),
            ("ControlOrMeta+v", "ControlOrMeta+v"),
            ("a", "[redacted 1]"),
            ("Shift+P", "Shift+[redacted 1]"),
            ("Shift++", "Shift+[redacted 1]"),
            ("password", "[redacted 8]"),
        ];
        for (key, kept) in cases {
            let press = json!({ "selector": "#a", "key": key });
            assert_eq!(summary::<FramePressParams>(&press)["key"], kept, "{key}");
        }
        let secrets = RunSecrets::default();
        register_secrets(
            &json!({"key":"private-key-text"}),
            FramePressParams::META.params,
            &secrets,
        );
        assert!(
            !secrets
                .scrubber()
                .scrub("Unknown key private-key-text")
                .contains("private-key-text")
        );
        assert_eq!(
            secrets.scrubber().scrub_protected("private-key-text"),
            "private-key-text"
        );
    }

    #[test]
    fn reads_and_choices_keep_what_they_name_and_arguments_keep_no_text() {
        let attribute = json!({ "selector": "#a", "name": "aria-label" });
        assert_eq!(summary::<FrameGetAttributeParams>(&attribute), attribute);
        let select = json!({
            "selector": "select", "options": [{ "valueOrLabel": "blue" }, { "value": "r", "label": "Red", "index": 2 }],
        });
        assert_eq!(summary::<FrameSelectOptionParams>(&select), select);
        let eval = json!({
            "selector": "li", "expression": "(ss) => ss.map(e => e.textContent)", "isFunction": true,
            "arg": { "value": { "a": [{ "s": "typed" }, { "n": 1 }], "ta": { "b": "AAAAAA==", "k": "ui8" } }, "handles": [] },
        });
        assert_eq!(
            summarize(&eval, FRAME_EVAL_ON_SELECTOR_ALL_META.params).params,
            json!({
                "selector": "li", "expression": "(ss) => ss.map(e => e.textContent)", "isFunction": true,
                "arg": { "value": { "a": [{ "s": "[redacted 5]" }, { "n": 1 }], "ta": { "b": "[binary 4 bytes]", "k": "ui8" } }, "handles": [] },
            })
        );
    }

    #[test]
    fn a_message_naming_no_known_method_keeps_no_text() {
        let forged = json!({ "selector": "#a", "n": 1, "nested": { "s": "a value", "b": true, "list": ["x"] } });
        assert_eq!(
            summarize(&forged, &[]).params,
            json!({ "selector": "[redacted 2]", "n": 1, "nested": { "s": "[redacted 7]", "b": true, "list": ["[redacted 1]"] } })
        );
        let unexpected = json!({ "selector": "#a", "extra": "a value" });
        assert_eq!(
            summary::<FrameTextContentParams>(&unexpected)["extra"],
            "[redacted 7]",
            "a field the command does not have is read as a secret"
        );
    }

    #[test]
    fn a_summary_stays_valid_json_within_its_cap() {
        let options: Vec<Value> = (0..1000)
            .map(|index| json!({ "value": format!("option {index}") }))
            .collect();
        let huge = json!({ "selector": "x".repeat(100_000), "options": options });
        let summary = summarize(&huge, FrameSelectOptionParams::META.params);
        let text = summary.params.to_string();
        assert!(text.len() <= MAX_SUMMARY_BYTES, "{} bytes", text.len());
        assert!(summary.truncated, "it says it was cut");
        assert!(
            summary.params["selector"]
                .as_str()
                .is_some_and(|kept| kept.starts_with("xxx"))
        );
    }

    #[test]
    fn registered_secrets_leave_free_text_in_every_form_text_carries_them() {
        let secret = "pa ss\"w/rd";
        let run = RunSecrets::default();
        register_secrets(
            &json!({ "selector": "#a", "value": secret }),
            FrameFillParams::META.params,
            &run,
        );
        let text = r#"fill("pa ss"w/rd") failed; sent {"value":"pa ss\"w/rd"} to /login?p=pa%20ss%22w%2Frd"#;
        assert_eq!(
            run.scrubber().scrub(text),
            r#"fill("[redacted]") failed; sent {"value":"[redacted]"} to /login?p=[redacted]"#
        );
        assert_eq!(
            run.scrubber().scrub("#a stays"),
            "#a stays",
            "a selector is not a secret"
        );
    }

    #[test]
    fn longer_and_overlapping_secrets_leave_nothing_of_either() {
        let nested = scrubber(&["abcdef", "abcdefgh"]);
        assert_eq!(nested.scrub("xx abcdefgh yy"), "xx [redacted] yy");
        let overlapping = scrubber(&["abcdef", "defghi"]);
        assert_eq!(overlapping.scrub("abcdefghi!"), "[redacted]!");
    }

    #[test]
    fn encoded_forms_do_not_hide_distinct_raw_registrations() {
        let secrets = scrubber(&["pa ss", "pa%20ss"]);
        assert_eq!(secrets.scrub("pa%2520ss"), "[redacted]");
    }

    #[test]
    fn short_secrets_are_not_searched_for_in_text() {
        assert_eq!(scrubber(&["abc"]).scrub("abc abc"), "abc abc");
    }

    #[test]
    fn a_long_secret_matched_by_its_start_takes_the_rest_of_its_line() {
        let secret = "s".repeat(300);
        let scrubber = scrubber(&[&secret]);
        let text = format!("token {secret} trailing\nnext line");
        assert_eq!(scrubber.scrub(&text), "token [redacted]\nnext line");
        let start_only = format!("x {} and more", &secret[..260]);
        assert_eq!(scrubber.scrub(&start_only), "x [redacted]");
    }

    #[test]
    fn past_512_secrets_free_text_is_replaced_whole() {
        let run = RunSecrets::default();
        for index in 0..MAX_SECRETS {
            run.register(&format!("secret-{index:04}"));
        }
        assert_eq!(run.scrubber().scrub("a secret-0007 b"), "a [redacted] b");
        run.register("one too many");
        assert_eq!(run.scrubber().scrub("anything"), "[redacted 8]");
        assert_eq!(run.scrubber().scrub_cut("anything", 4), "[redacted 8]");
    }

    #[test]
    fn record_only_secrets_stay_in_what_the_script_sees_and_protected_ones_do_not() {
        let run = RunSecrets::default();
        run.register("mine-1234");
        run.register_protected("host-5678");
        let text = "typed mine-1234, wrote host-5678";
        let scrubber = run.scrubber();
        assert_eq!(
            scrubber.scrub(text),
            "typed [redacted], wrote [redacted]",
            "a record"
        );
        assert_eq!(
            scrubber.scrub_protected(text),
            "typed mine-1234, wrote [redacted]",
            "what the script sees"
        );
        run.register_protected("mine-1234");
        assert_eq!(
            run.scrubber().scrub_protected(text),
            "typed [redacted], wrote [redacted]",
            "registered again as protected"
        );
    }

    #[test]
    fn a_cut_text_never_ends_in_part_of_a_secret() {
        let scrubber = scrubber(&["hunter22"]);
        let text = format!("{}hunter22 and more", "a".repeat(10));
        let kept = scrubber.scrub_cut(&text, 12);
        assert_eq!(kept.len(), 12, "{kept}");
        assert!(!kept.contains("hu"), "{kept}");
    }

    #[test]
    fn a_value_is_scrubbed_in_its_strings_and_keys() {
        let scrubber = scrubber(&["hunter22"]);
        assert_eq!(
            scrubber.scrub_value(&json!({ "hunter22": ["x hunter22 y", 1, null] })),
            json!({ "[redacted]": ["x [redacted] y", 1, null] })
        );
    }
}
