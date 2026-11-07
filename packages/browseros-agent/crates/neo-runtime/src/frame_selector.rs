//! Splits Playwright selectors at frame boundaries, preserving quoted and composite selectors.

use std::future::Future;
use std::sync::Arc;

use neo_runtime_spec::injected::{InjectedFrameOwnerByRefParams, InjectedFrameOwnerParams};
use neo_runtime_spec::{Element, Handle};
use serde_json::{Value, json};

use crate::click::PageError;
use crate::frames::{FrameContext, FrameScope};
use crate::page::{FrameTab, GroupGuard};

use crate::progress;

/// Playwright's selector engines whose body is a nested selector, as a JSON string
/// (`kNestedSelectorNames` in `isomorphic/selectorParser.ts`).
const NESTED_SELECTORS: [&str; 10] = [
    "internal:has",
    "internal:has-not",
    "internal:and",
    "internal:or",
    "internal:chain",
    "left-of",
    "right-of",
    "above",
    "below",
    "near",
];

/// Whether a selector enters a frame: it has an `internal:control=enter-frame` or
/// `internal:control=any-frame` part, at the top or inside a nested selector such as
/// `internal:has="iframe >> internal:control=enter-frame >> b"`. Parts are found as
/// Playwright's `parseSelector` finds them, so text that only mentions the marker
/// (`getByText('internal:control=enter-frame')` is `internal:text="internal:control=enter-frame"i`)
/// is never mistaken for one. A nested body that is not Playwright's JSON enters no frame here;
/// the page's parser then reports it with Playwright's own message.
///
/// The recursion is as deep as the nesting, which stays shallow: each level must escape every
/// quote and backslash of the level inside it, so depth grows with the log of the length.
fn has_frame_controls(selector: &str) -> bool {
    selector_parts(selector).into_iter().any(|part| {
        let (name, body) = selector_part(part);
        match name {
            "internal:control" => matches!(body, "enter-frame" | "any-frame"),
            name if NESTED_SELECTORS.contains(&name) => {
                nested_selector(name, body).is_some_and(|nested| has_frame_controls(&nested))
            }
            _ => false,
        }
    })
}

/// Playwright's `parseSelectorString`, reduced to each part's engine name and body: split on
/// `>>` outside quotes, then read `name=body` when the name is an engine name. Every other part
/// (CSS, `//xpath`, a quoted text shorthand) gets an empty name. A backslash escapes the next
/// character, and a quote opens nothing inside an unquoted `text=` body
/// (`shouldIgnoreTextSelectorQuote`: the part so far matches `/^\s*text\s*=(.*)$/` with some
/// text, which JS's `.` keeps free of line breaks).
///
/// One pass, linear in the selector: Playwright re-matches the part so far at every quote, which
/// on a long selector full of quotes would hold the realm's thread for hours. Here each part's
/// `text=` body start is looked up once, and the latest line break is tracked as the scan goes.
fn selector_parts(selector: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    if selector.contains(">>") {
        let mut quote = None;
        let mut text_body: Option<Option<usize>> = None;
        let mut last_break = None;
        let mut chars = selector.char_indices().peekable();
        while let Some((index, c)) = chars.next() {
            if is_line_break(c) {
                last_break = Some(index);
            }
            if c == '\\' {
                if let Some((skipped, escaped)) = chars.next()
                    && is_line_break(escaped)
                {
                    last_break = Some(skipped);
                }
            } else if Some(c) == quote {
                quote = None;
            } else if quote.is_none() && matches!(c, '"' | '\'' | '`') {
                let body = *text_body.get_or_insert_with(|| text_body_start(selector, start));
                let in_text = body.is_some_and(|body| {
                    index > body && last_break.is_none_or(|line_break| line_break < body)
                });
                if !in_text {
                    quote = Some(c);
                }
            } else if quote.is_none() && c == '>' && chars.peek().is_some_and(|(_, c)| *c == '>') {
                parts.push(&selector[start..index]);
                chars.next();
                start = index + 2;
                text_body = None;
            }
        }
    }
    parts.push(&selector[start..]);
    parts
}

/// Where a `text=` part's body starts, for the part beginning at `start`: just past the `=` of
/// `\s*text\s*=`, or `None` for any other part.
fn text_body_start(selector: &str, start: usize) -> Option<usize> {
    let body = selector[start..]
        .trim_start_matches(progress::is_js_whitespace)
        .strip_prefix("text")?
        .trim_start_matches(progress::is_js_whitespace)
        .strip_prefix('=')?;
    Some(selector.len() - body.len())
}

/// A JS line terminator, which a regex `.` does not match.
fn is_line_break(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// One part's `(name, body)`: the name before the first `=` when it is a valid engine name
/// (`[a-zA-Z_0-9-+:*]+`, a leading `*` being the capture mark), else an empty name.
fn selector_part(part: &str) -> (&str, &str) {
    let part = part.trim_matches(progress::is_js_whitespace);
    let Some((name, body)) = part.split_once('=') else {
        return ("", part);
    };
    let name = name.trim_matches(progress::is_js_whitespace);
    let engine = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | ':' | '*'));
    if engine {
        (name.strip_prefix('*').unwrap_or(name), body)
    } else {
        ("", part)
    }
}

/// The selector nested in engine `name`'s part: Playwright's `JSON.parse('[' + body + ']')`, a
/// string, followed by a number for the engines that take a distance. `None` where Playwright
/// throws `Malformed selector`.
fn nested_selector(name: &str, body: &str) -> Option<String> {
    nested_body(name, body).map(|(selector, _)| selector)
}

/// Playwright: `isomorphic/selectorParser.ts` `parseSelector`: `const unescaped = JSON.parse('[' + part.body + ']');`
fn nested_body(name: &str, body: &str) -> Option<(String, Option<Value>)> {
    if !NESTED_SELECTORS.contains(&name) {
        return None;
    }
    let parsed: Vec<Value> = serde_json::from_str(&format!("[{body}]")).ok()?;
    match parsed.as_slice() {
        [Value::String(selector)] => Some((selector.clone(), None)),
        [Value::String(selector), distance @ Value::Number(_)]
            if matches!(name, "left-of" | "right-of" | "above" | "below" | "near") =>
        {
            Some((selector.clone(), Some(distance.clone())))
        }
        _ => None,
    }
}

/// Neo: Retains each original prefix spelling for comparison while nested bodies are
/// rewritten. Capture stays attached to its part, so removing a prefix also shifts it.
struct SelectorPart {
    text: String,
    comparable: String,
    capture: bool,
}

/// Playwright: `isomorphic/selectorParser.ts` `selectorPartsEqual`: `return stringifySelector({ parts: list1 }) === stringifySelector({ parts: list2 });`
fn comparable_part(part: &str) -> (String, bool) {
    let (name, body) = selector_part(part);
    let capture = !name.is_empty()
        && part.split_once('=').is_some_and(|(name, _)| {
            name.trim_matches(progress::is_js_whitespace)
                .starts_with('*')
        });
    let name = if name.is_empty() {
        if body.len() > 1
            && ((body.starts_with('"') && body.ends_with('"'))
                || (body.starts_with('\'') && body.ends_with('\'')))
        {
            "text"
        } else if body.trim_start_matches('(').starts_with("//") || body.starts_with("..") {
            "xpath"
        } else {
            "css"
        }
    } else {
        name
    };
    let comparable = match name {
        "css" => body.to_string(),
        "css:light" => format!(":light({body})"),
        "xpath" if body.starts_with("//") || body.starts_with("..") => body.to_string(),
        _ => format!("{name}={body}"),
    };
    (comparable, capture)
}

/// Playwright: `isomorphic/selectorParser.ts` `parseSelector`: `nested.body.parsed.capture -= lastFrameIndex + 1;`
fn normalize_composites(selector: &str) -> Result<Vec<SelectorPart>, String> {
    let mut parts: Vec<SelectorPart> = Vec::new();
    for part in selector_parts(selector) {
        let part = part.trim_matches(progress::is_js_whitespace);
        let (comparable, capture) = comparable_part(part);
        let mut text = part.to_string();
        let (name, body) = selector_part(part);
        if let Some((nested, distance)) = nested_body(name, body) {
            let mut nested = normalize_composites(&nested)?;
            let outer = if parts
                .first()
                .is_some_and(|part| selector_part(&part.text) == ("internal:control", "any-frame"))
            {
                &parts[1..]
            } else {
                &parts[..]
            };
            if let Some(last_frame) = nested
                .iter()
                .rposition(|part| selector_part(&part.text) == ("internal:control", "enter-frame"))
                && last_frame < outer.len()
                && nested[..=last_frame]
                    .iter()
                    .zip(outer)
                    .all(|(a, b)| a.comparable == b.comparable)
            {
                if nested[..=last_frame].iter().any(|part| part.capture) {
                    return Err(capture_before_frame());
                }
                nested.drain(..=last_frame);
            }
            let local = nested
                .iter()
                .map(|part| part.text.as_str())
                .collect::<Vec<_>>()
                .join(" >> ");
            let encoded = Value::String(local).to_string();
            let distance = distance.map_or_else(String::new, |value| format!(",{value}"));
            let capture = if capture { "*" } else { "" };
            text = format!("{capture}{name}={encoded}{distance}");
        }
        parts.push(SelectorPart {
            text,
            comparable,
            capture,
        });
    }
    Ok(parts)
}

/// Playwright: `isomorphic/selectorParser.ts` `parseSelector`: `if (nested.body.parsed.capture <= lastFrameIndex)`
fn capture_before_frame() -> String {
    "Can not capture the selector before diving into the frame. Only use * after the last frame has been selected".to_string()
}

/// A selector's document-local chunks. Path references always start at the main frame.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Chunks {
    pub(crate) hops: Vec<Hop>,
    pub(crate) last: String,
    pub(crate) from_main: bool,
}

/// One step into a child frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Hop {
    /// A frame chunk: the selector of the `<iframe>` in the current document.
    Chunk(String),
    /// One owner of a main-rooted path ref, by the aria ref its document's snapshots keep on it
    /// (`e7` for `aria-ref=f7…`), found whatever the document's last snapshot holds.
    ///
    /// Deviation: `server/frameSelectors.ts` `FrameSelectors._jumpToAriaRefFrameIfNeeded`: `const jumptToFrame = this.frame._page.frameManager.frames().find(frame => frame.seq === frameSeq);` Playwright names a frame's refs by the frame's number; Neo names them by the path of iframe refs from the main frame (`f7f2e1`), which a later run can still follow. Such a ref stops routing when an iframe on its path is replaced, or its accessible role or name changes (its snapshots then give it a new ref), where Playwright's frame number stays.
    Owner(String),
    /// Neo: a frame named by its identity, the CDP frame id, for a frame behind an iframe that
    /// gets no aria ref (not visible, or receiving no pointer events): `fi`, then the id's 32 hex
    /// digits in lowercase (`aria-ref=fi35f6…b0f5e3`). The frame is entered directly, wherever it
    /// sits, as Playwright jumps to a frame by its number, so a later run still routes it; the
    /// ref stops routing when that frame goes, as Playwright's does.
    Frame(String),
}

/// The hex digits of a CDP frame id (a Chromium devtools frame token), which [`Hop::Frame`]
/// carries.
const FRAME_ID_DIGITS: usize = 32;

/// Neo: the path hop naming the frame `id` by its identity ([`Hop::Frame`]); `None` for an id
/// that is not [`FRAME_ID_DIGITS`] hex digits. BrowserOS's are uppercase; the hop is lowercase
/// so a ref stays `[a-z0-9]+`, as every other ref is.
pub(crate) fn identity_hop(id: &str) -> Option<String> {
    (id.len() == FRAME_ID_DIGITS && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| format!("fi{}", id.to_ascii_lowercase()))
}

pub(crate) fn split(selector: &str) -> Result<Chunks, String> {
    let mut hops = Vec::new();
    let mut current = Vec::new();
    let mut from_main = false;
    let parts = normalize_composites(selector)?;
    if let Some(last_frame) = parts
        .iter()
        .rposition(|part| selector_part(&part.text) == ("internal:control", "enter-frame"))
        && parts[..=last_frame].iter().any(|part| part.capture)
    {
        return Err(capture_before_frame());
    }
    for (index, part) in parts.iter().enumerate() {
        let part = part.text.as_str();
        let (name, body) = selector_part(part);
        if NESTED_SELECTORS.contains(&name)
            && nested_selector(name, body).is_some_and(|nested| has_frame_controls(&nested))
        {
            return Err("Frame locators are not allowed inside composite locators".to_string());
        }
        if name == "internal:control" && body == "any-frame" {
            return Err("Neo does not support internal:control=any-frame yet".to_string());
        }
        if name == "internal:control" && body == "enter-frame" {
            let chunk = current.join(" >> ");
            if chunk.trim().is_empty() {
                return Err("Selector cannot start with entering frame".to_string());
            }
            hops.push(Hop::Chunk(chunk));
            current.clear();
        } else if index == 0
            && name == "aria-ref"
            && let Some((path, element)) = path_ref(body)
        {
            from_main = true;
            hops.extend(path);
            current.push(element);
        } else {
            current.push(part.trim_matches(progress::is_js_whitespace).to_string());
        }
    }
    let last = current.join(" >> ");
    if !hops.is_empty() && last.is_empty() {
        return Err("Selector cannot end with entering frame".to_string());
    }
    Ok(Chunks {
        hops,
        last,
        from_main,
    })
}

/// Path refs encode iframe owner refs, followed by the final document's element ref: the owner
/// hops, then the element's local `aria-ref=eN` selector.
fn path_ref(body: &str) -> Option<(Vec<Hop>, String)> {
    let mut rest = body;
    let mut path = Vec::new();
    while let Some(after_f) = rest.strip_prefix('f') {
        if let Some(after_i) = after_f.strip_prefix('i') {
            let id = after_i.get(..FRAME_ID_DIGITS)?;
            if !id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return None;
            }
            path.push(Hop::Frame(id.to_ascii_uppercase()));
            rest = &after_i[FRAME_ID_DIGITS..];
            continue;
        }
        let digits = after_f.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        path.push(Hop::Owner(format!("e{}", &after_f[..digits])));
        rest = &after_f[digits..];
    }
    let element = rest.strip_prefix('e')?;
    if path.is_empty() || element.is_empty() || !element.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((path, format!("aria-ref={rest}")))
}

/// One attempt's destination and the visibility of its iframe owners. Visibility belongs to
/// the selector path, rather than the shared cached context.
pub(crate) struct Located<C = Arc<FrameContext>> {
    pub(crate) frame: C,
    pub(crate) selector: String,
    pub(crate) frame_visible: bool,
    pub(crate) owners: Vec<FrameOwner>,
}

/// A resolved owner stays in its parent world until the action has finished using its geometry.
#[derive(Clone)]
pub(crate) struct FrameOwner {
    pub(crate) frame: Arc<FrameContext>,
    pub(crate) element: Handle<Element>,
    _group: Arc<GroupGuard>,
}

pub(crate) async fn locate(
    tab: &FrameTab,
    selector: &str,
    strict: bool,
) -> Result<Option<Located>, PageError> {
    let chunks = split(selector).map_err(PageError::Threw)?;
    let initial = if chunks.from_main {
        tab.frames.main()
    } else {
        tab.context().await?
    };
    let owners = if !chunks.from_main && matches!(tab.scope, FrameScope::Frame(_)) {
        direct_owners(tab, &initial.id).await?
    } else {
        Vec::new()
    };
    // Direct receivers start visible; only explicit selector hops contribute visibility.
    // Retain direct owners for geometry and hit checks, independently of that visibility.
    let mut resolver = Resolver {
        tab,
        owners,
        selector,
    };
    let mut located = follow(&mut resolver, initial, chunks, strict).await?;
    if let Some(located) = &mut located {
        located.owners = resolver.owners;
    }
    Ok(located)
}

async fn direct_owners(tab: &FrameTab, frame: &str) -> Result<Vec<FrameOwner>, PageError> {
    let mut owners = Vec::new();
    for (parent, child) in tab.frames.owners(frame).await? {
        let group = tab.group(&parent, "direct-frame-owner");
        let owner = parent
            .host
            .send("DOM.getFrameOwner", json!({"frameId":child}))
            .await
            .map_err(|error| parent.host.classify(&error))?;
        let backend = owner["backendNodeId"].as_i64().ok_or_else(|| {
            PageError::Contract("DOM.getFrameOwner returned no backendNodeId".to_string())
        })?;
        let element = parent.element(group.group(), backend).await?;
        owners.push(FrameOwner {
            frame: parent,
            element,
            _group: Arc::new(group),
        });
    }
    Ok(owners)
}

/// The hop seam keeps the traversal's retry/error rules testable without CDP or a browser.
trait Hops {
    type Context: Send;

    fn enter(
        &mut self,
        context: Self::Context,
        hop: &Hop,
        strict: bool,
    ) -> impl Future<Output = Result<Option<(Self::Context, bool)>, PageError>> + Send;
}

async fn follow<T: Hops>(
    resolver: &mut T,
    mut frame: T::Context,
    chunks: Chunks,
    strict: bool,
) -> Result<Option<Located<T::Context>>, PageError> {
    let mut frame_visible = true;
    for hop in chunks.hops {
        let Some((child, visible)) = resolver.enter(frame, &hop, strict).await? else {
            return Ok(None);
        };
        frame = child;
        frame_visible &= visible;
    }
    Ok(Some(Located {
        frame,
        selector: chunks.last,
        frame_visible,
        owners: Vec::new(),
    }))
}

struct Resolver<'a> {
    tab: &'a FrameTab,
    owners: Vec<FrameOwner>,
    /// The whole selector, for Playwright's message naming it.
    selector: &'a str,
}

impl Resolver<'_> {
    /// Neo: enters the frame a path ref names by identity ([`Hop::Frame`]) directly, as
    /// Playwright's `_jumpToAriaRefFrameIfNeeded` jumps to a frame by its number, and takes its
    /// owners from the main frame for geometry and hit checks, as a direct receiver's are; like a
    /// direct receiver, it starts visible. A frame that is gone fails the selector at once.
    ///
    /// Playwright: `server/frameSelectors.ts` `FrameSelectors._jumpToAriaRefFrameIfNeeded`
    async fn jump(&mut self, id: &str) -> Result<Option<(Arc<FrameContext>, bool)>, PageError> {
        let child = match self.tab.frames.context(id, None).await {
            Err(PageError::Threw(_)) => {
                return Err(PageError::Threw(format!(
                    "Invalid frame in aria-ref selector \"{}\"",
                    self.selector
                )));
            }
            other => other?,
        };
        self.owners = direct_owners(self.tab, &child.id).await?;
        Ok(Some((child, true)))
    }
}

impl Hops for Resolver<'_> {
    type Context = Arc<FrameContext>;

    async fn enter(
        &mut self,
        frame: Arc<FrameContext>,
        hop: &Hop,
        strict: bool,
    ) -> Result<Option<(Arc<FrameContext>, bool)>, PageError> {
        let (element, visible, group) = match hop {
            // Boxed: the jump walks the frame tree and the owners, which would otherwise widen
            // every caller's future.
            Hop::Frame(id) => return Box::pin(self.jump(id)).await,
            Hop::Chunk(selector) => {
                let group = self.tab.group(&frame, "frame-hop");
                let owner = frame
                    .call(
                        Some(group.group()),
                        &InjectedFrameOwnerParams {
                            selector: selector.clone(),
                            strict,
                        },
                    )
                    .await?;
                (owner.element, owner.visible, group)
            }
            Hop::Owner(aria_ref) => {
                let group = self.tab.group(&frame, "frame-hop");
                let owner = frame
                    .call(
                        Some(group.group()),
                        &InjectedFrameOwnerByRefParams {
                            r#ref: aria_ref.clone(),
                        },
                    )
                    .await?;
                (owner.element, owner.visible, group)
            }
        };
        let Some(element) = element else {
            return Ok(None);
        };
        let described = frame
            .host
            .send(
                "DOM.describeNode",
                json!({ "objectId": element.id().object_id() }),
            )
            .await
            .map_err(|error| {
                let error = frame.host.classify(&error);
                if error == PageError::NotConnected {
                    self.tab.frames.forget(&frame);
                }
                error
            })?;
        let Some(id) = described["node"]["frameId"].as_str() else {
            return Ok(None);
        };
        let child = self.tab.frames.context(id, Some(&frame.host)).await?;
        self.owners.push(FrameOwner {
            frame,
            element,
            _group: Arc::new(group),
        });
        Ok(Some((child, visible)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::VecDeque;
    use std::future;

    struct ScriptedHops(VecDeque<Result<Option<(u32, bool)>, PageError>>);

    impl Hops for ScriptedHops {
        type Context = u32;

        fn enter(
            &mut self,
            _context: u32,
            _hop: &Hop,
            _strict: bool,
        ) -> impl Future<Output = Result<Option<(u32, bool)>, PageError>> + Send {
            future::ready(self.0.pop_front().expect("scripted frame outcome"))
        }
    }

    fn chunks() -> Chunks {
        split("iframe >> internal:control=enter-frame >> input").unwrap()
    }

    #[tokio::test]
    async fn missing_iframe_keeps_the_attempt_retryable() {
        let mut script = ScriptedHops([Ok(None), Ok(Some((7, true)))].into());
        assert!(
            follow(&mut script, 0, chunks(), true)
                .await
                .unwrap()
                .is_none()
        );
        let found = follow(&mut script, 0, chunks(), true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.frame, 7);
        assert_eq!(found.selector, "input");
    }

    #[tokio::test]
    async fn strict_and_non_iframe_hops_fail_the_attempt() {
        for message in ["strict mode violation", "<iframe> was expected"] {
            let error = PageError::Threw(message.to_string());
            let mut script = ScriptedHops([Err(error.clone())].into());
            assert_eq!(
                follow(&mut script, 0, chunks(), true).await.err(),
                Some(error)
            );
        }
    }

    #[tokio::test]
    async fn every_hop_contributes_to_visibility() {
        let chunks = split(
            "#a >> internal:control=enter-frame >> #b >> internal:control=enter-frame >> input",
        )
        .unwrap();
        let mut script = ScriptedHops([Ok(Some((1, false))), Ok(Some((2, true)))].into());
        let found = follow(&mut script, 0, chunks, true).await.unwrap().unwrap();
        assert_eq!(found.frame, 2);
        assert!(!found.frame_visible);
    }

    #[test]
    fn splits_only_real_frame_boundaries() {
        for (selector, hops, last) in [
            (
                "#login >> internal:control=enter-frame >> input",
                vec!["#login"],
                "input",
            ),
            (
                "#a >> internal:control=enter-frame >> #b >> nth=2 >> internal:control=enter-frame >> button",
                vec!["#a", "#b >> nth=2"],
                "button",
            ),
            (
                "internal:text=\"internal:control=enter-frame\"i",
                vec![],
                "internal:text=\"internal:control=enter-frame\"i",
            ),
            (
                "internal:role=button[name=\"x >> internal:control=enter-frame\"i]",
                vec![],
                "internal:role=button[name=\"x >> internal:control=enter-frame\"i]",
            ),
            (
                "li >> internal:has=\"internal:text=\\\"internal:control=any-frame\\\"i\"",
                vec![],
                "li >> internal:has=\"internal:text=\\\"internal:control=any-frame\\\"i\"",
            ),
            (
                "text=\"a >> internal:control=enter-frame\"",
                vec![],
                "text=\"a >> internal:control=enter-frame\"",
            ),
            (
                "text=it's >> internal:control=enter-frame >> b",
                vec!["text=it's"],
                "b",
            ),
            (
                "text=it\n's >> internal:control=enter-frame >> b",
                vec![],
                "text=it\n's >> internal:control=enter-frame >> b",
            ),
            (
                "div >> internal:has=\"iframe >> internal:control=enter-frame >> b\",5",
                vec![],
                "div >> internal:has=\"iframe >> internal:control=enter-frame >> b\",5",
            ),
        ] {
            assert_eq!(
                split(selector).unwrap(),
                Chunks {
                    hops: hops
                        .into_iter()
                        .map(|hop| Hop::Chunk(hop.to_string()))
                        .collect(),
                    last: last.to_string(),
                    from_main: false
                },
                "{selector}"
            );
        }
    }

    #[test]
    fn tests_that_nested_selectors_repeating_the_frame_prefix_are_local() {
        let hop = " >> internal:control=enter-frame >> ";
        let f = format!("#f{hop}");
        let inner = format!("{f}#inner{hop}");
        let nested = |engine: &str, selector: &str| {
            format!("{engine}={}", serde_json::to_string(selector).unwrap())
        };
        for (outer, engine, target, local) in [
            (f.as_str(), "internal:and", "button", "button"),
            (f.as_str(), "internal:has", "span", "span"),
            (f.as_str(), "internal:has-not", "span", "span"),
            (f.as_str(), "internal:or", "button", "button"),
            (f.as_str(), "internal:chain", "button", "button"),
            (inner.as_str(), "internal:and", "button", "button"),
            (inner.as_str(), "internal:has", "span", "span"),
            (inner.as_str(), "internal:or", "button", "button"),
            (
                inner.as_str(),
                "internal:and",
                "div >> *css=button",
                "div >> *css=button",
            ),
            (
                f.as_str(),
                "internal:and",
                "div >> *css=button",
                "div >> *css=button",
            ),
        ] {
            let source = format!(
                "{outer}div >> {}",
                nested(engine, &format!("{outer}{target}"))
            );
            let found = split(&source).unwrap_or_else(|error| panic!("{source}: {error}"));
            assert_eq!(
                found.last,
                format!("div >> {}", nested(engine, local)),
                "{source}"
            );
            assert_eq!(
                found.hops.len(),
                outer.matches("enter-frame").count(),
                "{source}"
            );
        }
        let twice = nested(
            "internal:has",
            &format!(
                "{f}div >> {}",
                nested("internal:and", &format!("{f}button"))
            ),
        );
        let local = nested(
            "internal:has",
            &format!("div >> {}", nested("internal:and", "button")),
        );
        assert_eq!(
            split(&format!("{f}section >> {twice}")).unwrap().last,
            format!("section >> {local}"),
            "normalization recurses into each nested selector",
        );
        for engine in ["left-of", "right-of", "above", "below", "near"] {
            let source = format!(
                "{f}div >> {engine}={},50",
                serde_json::to_string(&format!("{f}button")).unwrap()
            );
            assert_eq!(
                split(&source).unwrap().last,
                format!("div >> {engine}=\"button\",50"),
                "distance is retained for {source}"
            );
        }
        let source = format!(
            "{inner}div >> {}",
            nested("internal:and", &format!("{f}button"))
        );
        assert_eq!(
            split(&source).unwrap().last,
            "div >> internal:and=\"button\"",
            "the nested selector's complete prefix matches the outer prefix"
        );
    }

    #[test]
    fn tests_that_frame_prefix_comparison_uses_playwright_serialization() {
        let hop = " >> internal:control=enter-frame >> ";
        for (outer, repeated) in [
            ("css=#f", "#f"),
            ("#f", "css=#f"),
            ("xpath=//iframe", "//iframe"),
            ("xpath=../iframe", "../iframe"),
            ("xpath=(//iframe)", "(//iframe)"),
            ("css:light=#f", ":light(#f)"),
            ("text=\"frame\"", "\"frame\""),
            ("text='frame'", "'frame'"),
            ("css= #f", "css= #f"),
        ] {
            let repeated = format!("{repeated}{hop}css=  button");
            let source = format!(
                "{outer}{hop}div >> internal:and={}",
                serde_json::to_string(&repeated).unwrap()
            );
            let found = split(&source).unwrap_or_else(|error| panic!("{source}: {error}"));
            assert_eq!(
                found.hops,
                [Hop::Chunk(outer.to_string())],
                "the outer owner is unchanged: {source}"
            );
            assert_eq!(
                found.last, "div >> internal:and=\"css=  button\"",
                "local body whitespace is preserved: {source}"
            );
        }
        for (outer, repeated) in [("#f", "css= #f"), ("css= #f", "#f")] {
            let repeated = format!("{repeated}{hop}button");
            let source = format!(
                "{outer}{hop}div >> internal:and={}",
                serde_json::to_string(&repeated).unwrap()
            );
            assert!(
                split(&source).is_err(),
                "prefix body whitespace participates in equality: {source}"
            );
        }
    }

    #[test]
    fn tests_that_nested_prefix_comparison_skips_the_outer_any_frame_token() {
        let source = r##"internal:control=any-frame >> #f >> internal:control=enter-frame >> div >> internal:and="#f >> internal:control=enter-frame >> button""##;
        let parts = normalize_composites(source).unwrap();
        assert_eq!(
            parts.last().unwrap().text,
            "internal:and=\"button\"",
            "the whole-selector any-frame token does not enter prefix comparison"
        );
        assert!(
            split(source).is_err(),
            "serving any-frame remains unsupported"
        );
    }

    #[test]
    fn tests_that_capture_before_a_repeated_frame_prefix_is_refused() {
        for (outer, target) in [
            ("#f", "*css=#f >> internal:control=enter-frame >> button"),
            ("#f", "#f >> *internal:control=enter-frame >> button"),
            (
                "#f >> internal:control=enter-frame >> #inner",
                "#f >> internal:control=enter-frame >> *css=#inner >> internal:control=enter-frame >> button",
            ),
        ] {
            let source = format!(
                "{outer} >> internal:control=enter-frame >> div >> internal:and={}",
                serde_json::to_string(target).unwrap()
            );
            assert!(split(&source).is_err(), "{source}");
        }
    }

    #[test]
    fn tests_that_composite_selectors_entering_a_different_frame_stay_refused() {
        for target in [
            "#other >> internal:control=enter-frame >> button",
            "#f >> internal:control=enter-frame >> #other >> internal:control=enter-frame >> button",
        ] {
            let source = format!(
                "#f >> internal:control=enter-frame >> div >> internal:and={}",
                serde_json::to_string(target).unwrap()
            );
            assert!(split(&source).is_err(), "{source}");
        }
    }

    #[test]
    fn refuses_composite_hops_and_empty_documents() {
        for selector in [
            "li >> internal:has=\"iframe >> internal:control=enter-frame >> b\"",
            "li >> internal:and=\"internal:has=\\\"iframe >> internal:control=enter-frame >> b\\\"\"",
            "button >> near=\"iframe >> internal:control=enter-frame >> b\",50",
            "li >> internal:has=\"internal:control=any-frame >> b\"",
            "internal:control=any-frame >> button",
            " *internal:control=enter-frame ",
            "iframe >> internal:control=enter-frame",
            "iframe >> internal:control=enter-frame >> >> internal:control=enter-frame >> b",
        ] {
            assert!(split(selector).is_err(), "{selector}");
        }
    }

    #[test]
    fn rewrites_path_refs_from_main_and_keeps_ordinary_refs() {
        for (selector, hops, last, from_main) in [
            ("aria-ref=f7f2e1", vec!["e7", "e2"], "aria-ref=e1", true),
            (
                "aria-ref=f7e8 >> button",
                vec!["e7"],
                "aria-ref=e8 >> button",
                true,
            ),
            ("aria-ref=e1", vec![], "aria-ref=e1", false),
            ("aria-ref=f7xe1", vec![], "aria-ref=f7xe1", false),
            ("aria-ref=f7e1x", vec![], "aria-ref=f7e1x", false),
        ] {
            assert_eq!(
                split(selector).unwrap(),
                Chunks {
                    hops: hops
                        .into_iter()
                        .map(|hop| Hop::Owner(hop.to_string()))
                        .collect(),
                    last: last.to_string(),
                    from_main
                },
                "{selector}"
            );
        }
    }

    #[test]
    fn tests_that_identity_refs_name_a_frame_by_its_id_and_stay_lowercase() {
        // A BrowserOS frame id (measured: main, same-origin and out-of-process frames alike).
        const ID: &str = "35F6D2E61FAC8516061D7F5EC28EB0F5";
        let hop = identity_hop(ID).unwrap();
        assert_eq!(hop, "fi35f6d2e61fac8516061d7f5ec28eb0f5");
        assert!(
            hop.bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        );
        assert_eq!(
            split(&format!("aria-ref={hop}e3")).unwrap(),
            Chunks {
                hops: vec![Hop::Frame(ID.to_string())],
                last: "aria-ref=e3".to_string(),
                from_main: true,
            }
        );
        // A frame inside it, by its iframe's ref.
        assert_eq!(
            split(&format!("aria-ref={hop}f2e1 >> button")).unwrap(),
            Chunks {
                hops: vec![Hop::Frame(ID.to_string()), Hop::Owner("e2".to_string())],
                last: "aria-ref=e1 >> button".to_string(),
                from_main: true,
            }
        );
        // Anything else stays a document-local ref: a short id, an uppercase one, a non-hex one.
        for body in [
            format!("fi{}e3", &hop[2..33]),
            format!("fi{ID}e3"),
            format!("fi{}e3", "g".repeat(32)),
        ] {
            let selector = format!("aria-ref={body}");
            assert_eq!(
                split(&selector).unwrap(),
                Chunks {
                    hops: vec![],
                    last: selector.clone(),
                    from_main: false,
                },
                "{selector}"
            );
        }
        assert_eq!(identity_hop(&ID[..31]), None);
        assert_eq!(identity_hop("not-a-frame-id-but-32-chars-long"), None);
    }
}
