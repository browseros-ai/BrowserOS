//! Splits Playwright selectors at frame boundaries, preserving quoted and composite selectors.

use std::future::Future;
use std::sync::Arc;

use neo_runtime_spec::injected::{
    ElementState, ElementStatesOutcome, InjectedCheckElementStatesParams, InjectedFrameOwnerParams,
};
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
    let parsed: Vec<Value> = serde_json::from_str(&format!("[{body}]")).ok()?;
    match parsed.as_slice() {
        [Value::String(selector)] => Some(selector.clone()),
        [Value::String(selector), Value::Number(_)]
            if matches!(name, "left-of" | "right-of" | "above" | "below" | "near") =>
        {
            Some(selector.clone())
        }
        _ => None,
    }
}

/// A selector's document-local chunks. Path references always start at the main frame.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Chunks {
    pub(crate) hops: Vec<String>,
    pub(crate) last: String,
    pub(crate) from_main: bool,
}

pub(crate) fn split(selector: &str) -> Result<Chunks, String> {
    let mut hops = Vec::new();
    let mut current = Vec::new();
    let mut from_main = false;
    for (index, part) in selector_parts(selector).into_iter().enumerate() {
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
            hops.push(chunk);
            current.clear();
        } else if index == 0
            && name == "aria-ref"
            && let Some(mut path) = path_ref(body)
        {
            from_main = true;
            let last = path
                .pop()
                .ok_or_else(|| "Empty frame reference".to_string())?;
            hops.extend(path);
            current.push(last);
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

/// Path refs encode iframe owner refs, followed by the final document's element ref.
fn path_ref(body: &str) -> Option<Vec<String>> {
    let mut rest = body;
    let mut path = Vec::new();
    while let Some(after_f) = rest.strip_prefix('f') {
        let digits = after_f.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        path.push(format!("aria-ref=e{}", &after_f[..digits]));
        rest = &after_f[digits..];
    }
    let element = rest.strip_prefix('e')?;
    if path.is_empty() || element.is_empty() || !element.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    path.push(format!("aria-ref={rest}"));
    Some(path)
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
    let mut visible = true;
    for owner in &owners {
        let states = owner
            .frame
            .call(
                None,
                &InjectedCheckElementStatesParams {
                    element: owner.element.clone(),
                    states: vec![ElementState::Visible],
                    frame_visible: true,
                },
            )
            .await?;
        match states.outcome {
            None => {}
            Some(ElementStatesOutcome::NotConnected(_)) => return Err(PageError::NotConnected),
            Some(ElementStatesOutcome::Missing(_)) => visible = false,
        }
    }
    let mut resolver = Resolver { tab, owners };
    let mut located = follow(&mut resolver, initial, chunks, strict).await?;
    if let Some(located) = &mut located {
        located.owners = resolver.owners;
        located.frame_visible &= visible;
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
        selector: &str,
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
}

impl Hops for Resolver<'_> {
    type Context = Arc<FrameContext>;

    async fn enter(
        &mut self,
        frame: Arc<FrameContext>,
        selector: &str,
        strict: bool,
    ) -> Result<Option<(Arc<FrameContext>, bool)>, PageError> {
        let group = self.tab.group(&frame, "frame-hop");
        let owner = frame
            .call(
                Some(group.group()),
                &InjectedFrameOwnerParams {
                    selector: selector.to_string(),
                    strict,
                },
            )
            .await?;
        let Some(element) = owner.element else {
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
        Ok(Some((child, owner.visible)))
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
            _selector: &str,
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
                    hops: hops.into_iter().map(str::to_string).collect(),
                    last: last.to_string(),
                    from_main: false
                },
                "{selector}"
            );
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
            (
                "aria-ref=f7f2e1",
                vec!["aria-ref=e7", "aria-ref=e2"],
                "aria-ref=e1",
                true,
            ),
            (
                "aria-ref=f7e8 >> button",
                vec!["aria-ref=e7"],
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
                    hops: hops.into_iter().map(str::to_string).collect(),
                    last: last.to_string(),
                    from_main
                },
                "{selector}"
            );
        }
    }
}
