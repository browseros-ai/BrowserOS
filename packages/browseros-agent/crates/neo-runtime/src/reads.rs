//! `Frame.textContent`, `Frame.innerText`, `Frame.innerHTML`, `Frame.getAttribute` and
//! `Frame.ariaSnapshot`:
//! Playwright's page and locator reads, in the structure of its
//! server. The text reads use `frames.ts` `_waitForFunctionOnSelector` over
//! `retryWithProgressAndBackoff`; `ariaSnapshot` is `page.ts`
//! `ariaSnapshotJSONForFrame` over `retryWithProgressAndTimeouts`, rendered by `aria_yaml.rs` as
//! `frames.ts` `ariaSnapshot` does.
//!
//! ```text
//!  text_content / inner_text / inner_html / get_attribute   log: waiting for <locator>
//!   wait_for_function_on_selector  waits 0,20,50,100,100,500…  (click's LoopAWaits)
//!   │ prepare the tab
//!   │ first time only: describe → name the locator │ invalid selector → Error, at once
//!   │ glue textContent / innerText / innerHTML / getAttribute {selector, strict, name?}:
//!   │   mark first match
//!   │   matched → log "  locator resolved to <preview>" → {value} (missing attribute → {})
//!   │   nothing matched, or NotConnected → poll again
//!   │   Threw (strict mode violation) or Contract → Error with the page's message; Closed → closed
//!  aria_snapshot(tab, options, progress)               no call log at all
//!   waits 0,1000,2000,4000,8000…  (never capped by the timeout; the last repeats)
//!   │ prepare → glue ariaSnapshot {selector: 'body,frameset', strict: false, mode, depth, boxes}
//!   │   no body yet, or NotConnected → poll again; Threw or Contract → Error; Closed → closed
//!   ├ stitch: for each rendered iframe {ref: eN, depth}, concurrently, and again inside each
//!   │   glue ariaSnapshot {selector: '<hops>aria-ref=eN >> internal:control=enter-frame >>
//!   │     body,frameset', strict: false, depth: depth ? depth - iframe depth - 1 : none}
//!   │   NotConnected → poll again with the same waits; nothing matched, Threw, Contract or the
//!   │     call's own timeout → the iframe node stays as the page gave it; Closed → closed
//!   │   nodes go under the iframe node (its text dropped), their refs prefixed f<N>
//!   └ aria_yaml::render(nodes) → {snapshot}
//! ```
//!
//! The seam is two traits of their own, [`SelectorTab`] and [`ReadTab`], so the reads never grow
//! click's `Tab`; `FrameTab` implements both at the bottom of this file, over click's own
//! `prepare` and `describe` and the selected frame's utility world. [`wait_for_function_on_selector`]
//! takes the page call as a closure shared by text reads and default locator snapshots.
//! The tests drive both loops against a scripted tab under paused time.
//!
//! An `ai` snapshot also records its refs in each document's injected script, so a later
//! `aria-ref=` selector (any run's, since the utility world is per document) resolves the
//! element while it stays connected, until that document's next snapshot or a cross-document
//! commit. A ref inside an iframe is the path to it: `- iframe [ref=e7]:` holds
//! `- textbox [ref=f7e3]`, and a frame inside that frame's `- iframe [ref=f7e2]` holds
//! `[ref=f7f2e1]`; `frame_selector::split` turns one back into hops from the main frame, each
//! finding its iframe by the ref the iframe's own document keeps on it, so a later snapshot of
//! another part of that document leaves the ref routable, as Playwright's frame-numbered refs
//! stay. Playwright numbers its frames instead (`f1e3`), which a later run could not rebuild. An
//! `ai` snapshot taken inside a child frame names its refs the same way: the iframes on the way
//! from the main frame each give their ref (`frameOwnerRef`, which also keeps the owner, so one
//! inside a closed shadow root still routes), and an iframe that gets none (hidden) makes it
//! throw before the frame is snapshotted ([`UNNAMED_FRAME`]). The `default` mode gives no node a ref, so, as in
//! Playwright, its iframe nodes keep only what the page gave them.
//!
//! Neo-only: a lone surrogate (a truncated emoji) in any page call's answer or thrown message
//! reads as U+FFFD (`neo-runtime-injected/src/well-formed.ts`); Rust strings are UTF-8, and a CDP
//! reply escaping one would be dropped. Playwright returns it unchanged.
//!
//! A tree nested deeper than about 60 aria levels never arrives: browseros-cdp parses every CDP
//! reply with `serde_json`'s 128-level limit and drops one it cannot parse, so the snapshot ends at
//! the call's timeout (on BrowserOS neo, 30 nested lists snapshot and 31 time out; GitHub, Hacker
//! News, Wikipedia and MDN pages measured 11 to 16 levels).

use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use futures_util::stream::{FuturesUnordered, StreamExt};
use neo_runtime_spec::injected::{
    AriaChild, AriaFrameSnapshot, AriaIframe, AriaNode, AriaRole, AriaSnapshotMode, AttributeMatch,
    InjectedAriaSnapshotParams, InjectedAriaSnapshotResult, InjectedFrameOwnerRefParams,
    InjectedGetAttributeParams, InjectedInnerHTMLParams, InjectedInnerTextParams,
    InjectedTextContentParams, TextMatch,
};
use neo_runtime_spec::serve::{
    FrameAriaSnapshotJSONParams, FrameAriaSnapshotJSONParamsMode, FrameAriaSnapshotJSONResult,
    FrameAriaSnapshotParams, FrameAriaSnapshotParamsMode, FrameAriaSnapshotResult,
    FrameGetAttributeParams, FrameGetAttributeResult, FrameInnerHTMLParams, FrameInnerHTMLResult,
    FrameInnerTextParams, FrameInnerTextResult, FrameTextContentParams, FrameTextContentResult,
};

use crate::aria_yaml;
use crate::click::{self, LoopAWaits, PageError, SelectorFacts};
use crate::frame_selector::{self, FrameOwner};
use crate::page::FrameTab;
use crate::progress::{CallError, ErrorName, PlaywrightError, Progress};

/// The selector Playwright snapshots a frame's document by when the caller gave none
/// (`page.ts` `ariaSnapshotJSONForFrame`): a `<frameset>` document has no `<body>`.
const FRAME_ROOT: &str = "body,frameset";

/// What follows an iframe's selector to continue inside its document (Playwright's
/// `frameRootSelector` is `aria-ref=${ref} >> internal:control=enter-frame >> body,frameset`).
const FRAME_HOP: &str = " >> internal:control=enter-frame >> ";

/// The waits before each snapshot attempt: Playwright's `[1000, 2000, 4000, 8000]` after an
/// immediate first try, the last repeating, and never capped by the call's timeout.
const SNAPSHOT_WAITS_MS: [u64; 5] = [0, 1000, 2000, 4000, 8000];

/// Deviation: `server/page.ts` `ariaSnapshotJSONForFrame`: `return { ...resolved.result, resolvedFrame: resolved.frame };` Playwright names a child frame's refs by the frame's number, which needs no ref on its `<iframe>`; Neo's refs inside a frame are the path of iframe refs from the main frame, and an iframe that is not visible or receives no pointer events gets no ref unless an earlier snapshot gave it one. So an AI snapshot inside a frame behind such an iframe throws, before that frame is snapshotted, rather than answer refs no selector could route.
const UNNAMED_FRAME: &str = "Neo cannot name refs inside a frame whose <iframe> gets no ref (it is not visible or receives no pointer events); snapshot without mode: 'ai'";

// ---------------------------------------------------------------------------------------------
// The seam.
// ---------------------------------------------------------------------------------------------

/// What every read of a selector's first match needs from a tab. Each future is cancel-safe:
/// [`Progress::race`] may drop it, losing only the answer.
pub(crate) trait SelectorTab: Send + Sync {
    /// Makes the tab ready for page steps, as click's `Tab::prepare` does: the run's first
    /// browser call enables the page's domains and starts its watcher, which is also what tells
    /// a call that the tab closed.
    fn prepare(&self) -> impl Future<Output = Result<(), PageError>> + Send;

    /// `describeSelector`: the `waiting for` text, or the parse error.
    fn describe(
        &self,
        selector: &str,
    ) -> impl Future<Output = Result<SelectorFacts, PageError>> + Send;
}

/// The page reads, one glue call each.
pub(crate) trait ReadTab: SelectorTab {
    /// The `textContent` glue: the first match's `textContent` and preview, `None` while nothing
    /// matches. A strict violation is `PageError::Threw` with V8's `Error: strict mode …`.
    fn text_content(
        &self,
        selector: &str,
        strict: bool,
    ) -> impl Future<Output = Result<Option<TextMatch>, PageError>> + Send;

    /// The `innerText` glue: first match's rendered text, refusing non-HTML elements.
    fn inner_text(
        &self,
        selector: &str,
        strict: bool,
    ) -> impl Future<Output = Result<Option<TextMatch>, PageError>> + Send;

    /// The `innerHTML` glue: the first match's markup, any element kind.
    fn inner_html(
        &self,
        selector: &str,
        strict: bool,
    ) -> impl Future<Output = Result<Option<TextMatch>, PageError>> + Send;

    /// The `getAttribute` glue: `None` while no element matches; a matched element can have
    /// no value for the requested attribute and still ends the read.
    fn get_attribute(
        &self,
        selector: &str,
        strict: bool,
        name: &str,
    ) -> impl Future<Output = Result<Option<AttributeMatch>, PageError>> + Send;

    /// The `ariaSnapshot` glue: the tree and preview of the first match of `params.selector`.
    /// The snapshot is absent while nothing matches. An AI read the caller asked for, in a child
    /// frame's document, also answers that document's path from the main frame (`f7`, `f7f2`)
    /// for its refs; `stitching` marks a read of an iframe's document for the stitcher, which
    /// prefixes it itself.
    fn aria_snapshot(
        &self,
        params: &InjectedAriaSnapshotParams,
        stitching: bool,
    ) -> impl Future<Output = Result<AriaRead, PageError>> + Send;
}

/// One `ariaSnapshot` read.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AriaRead {
    pub(crate) result: InjectedAriaSnapshotResult,
    /// The main-rooted path in front of the refs of the document it read: empty in the main
    /// frame, outside `ai` mode and for the stitcher's reads.
    pub(crate) base: String,
}

/// What a read found on the first match: the value the call replies with, and
/// `InjectedScript.previewNode` of the element for the `locator resolved to` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Matched<R> {
    pub(crate) value: R,
    pub(crate) preview: String,
}

// ---------------------------------------------------------------------------------------------
// The policy.
// ---------------------------------------------------------------------------------------------

/// One `Frame.textContent`, `Frame.innerText` or `Frame.innerHTML` call's options, decoded once
/// by its route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReadOptions {
    pub(crate) selector: String,
    /// Locator reads send `strict: true`; page and frame reads default to non-strict.
    pub(crate) strict: bool,
}

impl ReadOptions {
    pub(crate) fn from_params(params: FrameTextContentParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
        }
    }

    pub(crate) fn from_inner_text(params: FrameInnerTextParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
        }
    }

    pub(crate) fn from_inner_html(params: FrameInnerHTMLParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
        }
    }
}

/// One `Frame.getAttribute`'s options, decoded once by the dispatcher's route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttributeOptions {
    pub(crate) selector: String,
    pub(crate) strict: bool,
    pub(crate) name: String,
}

impl AttributeOptions {
    pub(crate) fn from_params(params: FrameGetAttributeParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
            name: params.name,
        }
    }
}

/// One `Frame.ariaSnapshot`'s options, decoded once by the dispatcher's route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SnapshotOptions {
    /// A non-empty selector scopes the snapshot to one strict match.
    pub(crate) selector: Option<String>,
    /// `mode ?? 'default'`.
    pub(crate) mode: AriaSnapshotMode,
    /// Passed to the page as given: Playwright's renderer treats 0 as no limit.
    pub(crate) depth: Option<i64>,
    pub(crate) boxes: Option<bool>,
}

// Neo: decodes the generated snapshot params once for the runtime route.
impl SnapshotOptions {
    /// `Frame.ariaSnapshotJSON` takes `Frame.ariaSnapshot`'s params.
    pub(crate) fn from_json_params(params: FrameAriaSnapshotJSONParams) -> Self {
        let FrameAriaSnapshotJSONParams {
            mode,
            selector,
            depth,
            boxes,
        } = params;
        Self::from_params(FrameAriaSnapshotParams {
            mode: mode.map(|mode| match mode {
                FrameAriaSnapshotJSONParamsMode::Ai => FrameAriaSnapshotParamsMode::Ai,
                FrameAriaSnapshotJSONParamsMode::Default => FrameAriaSnapshotParamsMode::Default,
            }),
            selector,
            depth,
            boxes,
        })
    }

    /// Playwright: `server/frames.ts` `Frame.ariaSnapshotJSON`: `if (options.selector && options.mode !== 'ai') {`
    pub(crate) fn from_params(params: FrameAriaSnapshotParams) -> Self {
        let FrameAriaSnapshotParams {
            mode,
            selector,
            depth,
            boxes,
        } = params;
        Self {
            selector: selector.filter(|selector| !selector.is_empty()),
            mode: match mode {
                Some(FrameAriaSnapshotParamsMode::Ai) => AriaSnapshotMode::Ai,
                Some(FrameAriaSnapshotParamsMode::Default) | None => AriaSnapshotMode::Default,
            },
            depth,
            boxes,
        }
    }
}

/// Why one attempt ended without a value.
#[derive(Debug)]
enum Stop {
    /// A page step answered [`PageError::NotConnected`]: poll again, as Playwright retries every
    /// error its `isNonRetriableError` does not name.
    Again,
    /// The call ends with this error.
    End(CallError),
}

impl From<CallError> for Stop {
    fn from(error: CallError) -> Self {
        Self::End(error)
    }
}

impl From<PageError> for Stop {
    fn from(error: PageError) -> Self {
        match error {
            PageError::NotConnected => Self::Again,
            PageError::Threw(message) | PageError::Contract(message) => {
                Self::End(PlaywrightError::error(message).into())
            }
            PageError::Closed => Self::End(PlaywrightError::target_closed().into()),
        }
    }
}

/// Serves `Frame.textContent`: `{value}`, the first match's `textContent`.
pub(crate) async fn text_content<T: ReadTab>(
    tab: &T,
    options: &ReadOptions,
    progress: &Progress,
) -> Result<FrameTextContentResult, CallError> {
    let value = wait_for_function_on_selector(tab, &options.selector, progress, || async {
        let matched = tab.text_content(&options.selector, options.strict).await?;
        Ok(matched.map(|TextMatch { value, preview }| Matched { value, preview }))
    })
    .await?;
    Ok(FrameTextContentResult { value: Some(value) })
}

/// Serves `Frame.innerText`: `{value}`, the first match's rendered text.
pub(crate) async fn inner_text<T: ReadTab>(
    tab: &T,
    options: &ReadOptions,
    progress: &Progress,
) -> Result<FrameInnerTextResult, CallError> {
    let value = wait_for_function_on_selector(tab, &options.selector, progress, || async {
        let matched = tab.inner_text(&options.selector, options.strict).await?;
        Ok(matched.map(|TextMatch { value, preview }| Matched { value, preview }))
    })
    .await?;
    Ok(FrameInnerTextResult { value })
}

/// Serves `Frame.innerHTML`: `{value}`, the first match's markup.
///
/// Playwright: `server/frames.ts` `Frame.innerHTML`: `(injected, element) => ({ result: element.innerHTML })`
pub(crate) async fn inner_html<T: ReadTab>(
    tab: &T,
    options: &ReadOptions,
    progress: &Progress,
) -> Result<FrameInnerHTMLResult, CallError> {
    let value = wait_for_function_on_selector(tab, &options.selector, progress, || async {
        let matched = tab.inner_html(&options.selector, options.strict).await?;
        Ok(matched.map(|TextMatch { value, preview }| Matched { value, preview }))
    })
    .await?;
    Ok(FrameInnerHTMLResult { value })
}

/// Serves `Frame.getAttribute`: a missing attribute ends the read with no `value`.
pub(crate) async fn get_attribute<T: ReadTab>(
    tab: &T,
    options: &AttributeOptions,
    progress: &Progress,
) -> Result<FrameGetAttributeResult, CallError> {
    let value = wait_for_function_on_selector(tab, &options.selector, progress, || async {
        let matched = tab
            .get_attribute(&options.selector, options.strict, &options.name)
            .await?;
        Ok(matched.map(|AttributeMatch { value, preview }| Matched { value, preview }))
    })
    .await?;
    Ok(FrameGetAttributeResult { value })
}

/// Playwright's `_waitForFunctionOnSelector`: names the selector, then polls `evaluate` with
/// loop A's backoff until it reads the first match, logging `  locator resolved to <preview>`.
/// `evaluate` is one page call that resolves `selector` (marking its first match, as
/// Playwright's `markTargets: 'first'`), `None` while nothing matches.
///
/// # Errors
///
/// An invalid selector, a page exception (a strict mode violation) or a page answer outside the
/// spec ends the call with `Error`; the tab closing with `TargetClosedError`; the call's
/// deadline with `TimeoutError`.
pub(crate) async fn wait_for_function_on_selector<T, R, F, Fut>(
    tab: &T,
    selector: &str,
    progress: &Progress,
    mut evaluate: F,
) -> Result<R, CallError>
where
    T: SelectorTab,
    F: FnMut() -> Fut + Send,
    Fut: Future<Output = Result<Option<Matched<R>>, PageError>> + Send,
{
    // Playwright names the locator before any page call; Neo learns its text from the page, so
    // the line reads the raw selector until `describe` answers (Playwright's own fallback).
    progress.name_locator(selector);
    let mut named = false;
    let mut waits = LoopAWaits::new(progress.timeout_ms());
    loop {
        let wait = waits.next_wait();
        if !wait.is_zero() {
            progress.wait(wait).await?;
        }
        match read_once(tab, selector, progress, &mut named, &mut evaluate).await {
            Ok(Some(value)) => return Ok(value),
            Ok(None) | Err(Stop::Again) => {}
            Err(Stop::End(error)) => return Err(error),
        }
    }
}

/// One attempt of [`wait_for_function_on_selector`]: `Ok(None)` to poll again.
async fn read_once<T, R, F, Fut>(
    tab: &T,
    selector: &str,
    progress: &Progress,
    named: &mut bool,
    evaluate: &mut F,
) -> Result<Option<R>, Stop>
where
    T: SelectorTab,
    F: FnMut() -> Fut + Send,
    Fut: Future<Output = Result<Option<Matched<R>>, PageError>> + Send,
{
    progress.race(tab.prepare()).await??;
    if !*named {
        let facts = progress.race(tab.describe(selector)).await??;
        progress.name_locator(&facts.locator);
        *named = true;
        // Playwright parses the selector before its first page call and fails at once.
        if let Some(message) = facts.invalid {
            return Err(Stop::End(PlaywrightError::error(message).into()));
        }
    }
    let Some(matched) = progress.race(evaluate()).await?? else {
        return Ok(None);
    };
    progress.log(format!("  locator resolved to {}", matched.preview));
    Ok(Some(matched.value))
}

/// Serves `Frame.ariaSnapshot`: the tree as Playwright's YAML.
///
/// Playwright: `server/frames.ts` `Frame.ariaSnapshot`: `return { snapshot: renderAriaSnapshotAsYaml(snapshot) };`
pub(crate) async fn aria_snapshot<T: ReadTab>(
    tab: &T,
    options: &SnapshotOptions,
    progress: &Progress,
) -> Result<FrameAriaSnapshotResult, CallError> {
    render_snapshot(&aria_nodes(tab, options, progress).await?)
}

/// Serves `Frame.ariaSnapshotJSON`: the same tree as Playwright's `AriaSnapshotJSON`, the nodes
/// the YAML renders.
///
/// Playwright: `server/frames.ts` `Frame.ariaSnapshotJSON`: `return { snapshot: await ariaSnapshotJSONForFrame(progress, this, options.selector, options) };`
pub(crate) async fn aria_snapshot_json<T: ReadTab>(
    tab: &T,
    options: &SnapshotOptions,
    progress: &Progress,
) -> Result<FrameAriaSnapshotJSONResult, CallError> {
    let nodes = aria_nodes(tab, options, progress).await?;
    let snapshot = serde_json::to_value(nodes)
        .map_err(|error| PlaywrightError::error(format!("neo-runtime: {error}")))?;
    Ok(FrameAriaSnapshotJSONResult { snapshot })
}

/// Serves a document or locator snapshot. AI snapshots rooted in the main frame stitch
/// rendered iframe documents and prefix their refs with paths from that main frame.
async fn aria_nodes<T: ReadTab>(
    tab: &T,
    options: &SnapshotOptions,
    progress: &Progress,
) -> Result<Vec<AriaNode>, CallError> {
    if let Some(selector) = &options.selector {
        return locator_aria_snapshot(tab, selector, options, progress).await;
    }
    let (root, base) = snapshot_root(tab, options, progress).await?;
    let nodes = if options.mode == AriaSnapshotMode::Ai {
        stitch(tab, options, progress, root, &base).await?
    } else {
        root.nodes
    };
    Ok(nodes)
}

/// The call's own document, polled until it has a body, and the path in front of its refs.
async fn snapshot_root<T: ReadTab>(
    tab: &T,
    options: &SnapshotOptions,
    progress: &Progress,
) -> Result<(AriaFrameSnapshot, String), CallError> {
    // Non-strict: a document of nested framesets has several `frameset` elements.
    let params = InjectedAriaSnapshotParams {
        selector: FRAME_ROOT.to_string(),
        strict: false,
        mode: options.mode,
        depth: options.depth,
        boxes: options.boxes,
    };
    let mut attempt: usize = 0;
    loop {
        let wait = SNAPSHOT_WAITS_MS[attempt.min(SNAPSHOT_WAITS_MS.len() - 1)];
        attempt = attempt.saturating_add(1);
        if wait > 0 {
            progress.wait(Duration::from_millis(wait)).await?;
        }
        match snapshot_once(tab, &params, progress).await {
            Ok(Some(snapshot)) => return Ok(snapshot),
            Ok(None) | Err(Stop::Again) => {}
            Err(Stop::End(error)) => return Err(error),
        }
    }
}

/// Playwright: `server/frames.ts` `Frame.ariaSnapshotJSON`: `if (options.selector && options.mode !== 'ai') {`
async fn locator_aria_snapshot<T: ReadTab>(
    tab: &T,
    selector: &str,
    options: &SnapshotOptions,
    progress: &Progress,
) -> Result<Vec<AriaNode>, CallError> {
    let params = InjectedAriaSnapshotParams {
        selector: selector.to_string(),
        strict: true,
        mode: options.mode,
        depth: options.depth,
        boxes: options.boxes,
    };
    if options.mode == AriaSnapshotMode::Ai {
        return locator_ai_snapshot(tab, &params, options, progress).await;
    }
    let snapshot = wait_for_function_on_selector(tab, selector, progress, || async {
        let result = tab.aria_snapshot(&params, false).await?.result;
        let Some(value) = result.snapshot else {
            return Ok(None);
        };
        // Neo: a successful glue read returns both values from the same matched element.
        let preview = result.preview.ok_or_else(|| {
            PageError::Contract(
                "neo-runtime: aria snapshot is missing its element preview".to_string(),
            )
        })?;
        Ok(Some(Matched { value, preview }))
    })
    .await?;
    Ok(snapshot.nodes)
}

/// Playwright: `server/page.ts` `ariaSnapshotJSONForFrame`: `frame.retryWithProgressAndTimeouts(progress, [1000, 2000, 4000, 8000],`
async fn locator_ai_snapshot<T: ReadTab>(
    tab: &T,
    params: &InjectedAriaSnapshotParams,
    options: &SnapshotOptions,
    progress: &Progress,
) -> Result<Vec<AriaNode>, CallError> {
    let mut attempt: usize = 0;
    loop {
        let wait = SNAPSHOT_WAITS_MS[attempt.min(SNAPSHOT_WAITS_MS.len() - 1)];
        attempt = attempt.saturating_add(1);
        if wait > 0 {
            progress.wait(Duration::from_millis(wait)).await?;
        }
        match snapshot_once(tab, params, progress).await {
            Ok(Some((snapshot, base))) => {
                return stitch(tab, options, progress, snapshot, &base).await;
            }
            Ok(None) => {
                // Playwright: `server/page.ts` `ariaSnapshotJSONForFrame`: `throw new NonRecoverableDOMError(`
                return Err(PlaywrightError::error(format!(
                    "Selector \"{}\" does not match any element",
                    params.selector
                ))
                .into());
            }
            Err(Stop::Again) => {}
            Err(Stop::End(error)) => return Err(error),
        }
    }
}

/// Playwright: `server/frames.ts` `Frame.ariaSnapshot`: `return { snapshot: renderAriaSnapshotAsYaml(snapshot) };`
fn render_snapshot(nodes: &[AriaNode]) -> Result<FrameAriaSnapshotResult, CallError> {
    let snapshot = aria_yaml::render(nodes)
        .map_err(|error| PlaywrightError::error(format!("neo-runtime: {error}")))?;
    Ok(FrameAriaSnapshotResult { snapshot })
}

/// One snapshot attempt and the path in front of its refs: `Ok(None)` while the document has
/// no body yet.
async fn snapshot_once<T: ReadTab>(
    tab: &T,
    params: &InjectedAriaSnapshotParams,
    progress: &Progress,
) -> Result<Option<(AriaFrameSnapshot, String)>, Stop> {
    progress.race(tab.prepare()).await??;
    let read = progress.race(tab.aria_snapshot(params, false)).await??;
    Ok(read.result.snapshot.map(|snapshot| (snapshot, read.base)))
}

/// One document of a stitched snapshot, in the order its snapshot answered: the root first, a
/// document always after the one holding its iframe.
struct Document {
    nodes: Vec<AriaNode>,
    /// Where it goes; `None` for the root.
    slot: Option<Slot>,
    /// The documents of this one's iframes, prefixed and ready, by their iframe node's ref.
    stitched: HashMap<String, Vec<AriaNode>>,
}

/// Where an iframe's document goes: under the iframe node `iframe` (its ref, `eN`) of document
/// `owner`, an index into the stitched documents, with its refs prefixed by `prefix` (`fN`).
struct Slot {
    owner: usize,
    iframe: String,
    prefix: String,
}

/// One iframe's document still to snapshot.
struct Pending {
    slot: Slot,
    /// The selector of the iframe element from the call's frame: its owner's hops, then its own
    /// ref (`aria-ref=e7 >> internal:control=enter-frame >> aria-ref=e2` one document down).
    hops: String,
    /// Playwright's `childDepth` for this document.
    depth: Option<i64>,
}

/// Playwright's `ariaSnapshotJSONForFrame` past the root snapshot: snapshots every rendered
/// iframe's document through the iframe's ref (`aria-ref=eN >> internal:control=enter-frame >>
/// body,frameset`, resolved by the tab's selector seam like any other selector, so a hop into a
/// frame is the same hop a locator takes), then each of those documents' iframes, and nests each
/// document under its iframe node. A document's nodes take its iframe's path as a ref prefix
/// (`f7` for `- iframe [ref=e7]`, `f7f2` one level deeper), the form `frame_selector::split`
/// reads back into those hops, from the main frame, in any later run.
///
/// Children are snapshotted concurrently, each as soon as the document holding its iframe has
/// answered, from a queue rather than by recursion, so no frame tree deepens the stack. A child
/// that cannot be read leaves its iframe node as the page gave it; only the tab closing or the
/// run ending ends the call.
///
/// A root inside a child frame comes with its own path from the main frame, `base` (`f7`): its
/// iframes are reached by path refs from the main frame (`aria-ref=f7e2 >> …`), however the call
/// reached the root, and every ref of the result takes `base` in front.
async fn stitch<T: ReadTab>(
    tab: &T,
    options: &SnapshotOptions,
    progress: &Progress,
    root: AriaFrameSnapshot,
    base: &str,
) -> Result<Vec<AriaNode>, CallError> {
    let mut documents = vec![Document {
        nodes: root.nodes,
        slot: None,
        stitched: HashMap::new(),
    }];
    let mut snapshots = FuturesUnordered::new();
    for child in children(0, "", base, options.depth, &root.iframes) {
        snapshots.push(snapshot_child(tab, options, progress, child));
    }
    while let Some((child, snapshot)) = snapshots.next().await {
        // Playwright stitches a document only when it has nodes (`childSnapshot?.length`).
        let Some(snapshot) = snapshot?.filter(|snapshot| !snapshot.nodes.is_empty()) else {
            continue;
        };
        let index = documents.len();
        let hops = format!("{}{FRAME_HOP}", child.hops);
        for grandchild in children(index, &hops, "", child.depth, &snapshot.iframes) {
            snapshots.push(snapshot_child(tab, options, progress, grandchild));
        }
        documents.push(Document {
            nodes: snapshot.nodes,
            slot: Some(child.slot),
            stitched: HashMap::new(),
        });
    }
    // Every document comes after the one holding its iframe, so walking back from the last
    // finishes each document before its own owner takes it.
    while let Some(mut document) = documents.pop() {
        merge_iframes(&mut document.nodes, &mut document.stitched);
        let Some(slot) = document.slot else {
            if !base.is_empty() {
                prefix_refs(&mut document.nodes, base);
            }
            return Ok(document.nodes);
        };
        prefix_refs(&mut document.nodes, &slot.prefix);
        if let Some(owner) = documents.get_mut(slot.owner) {
            owner.stitched.insert(slot.iframe, document.nodes);
        }
    }
    Err(PlaywrightError::error("neo-runtime: the stitched snapshot lost its root").into())
}

/// The documents to snapshot for `iframes`, the rendered iframe nodes of document `owner`
/// reached through `hops`, each iframe named by its ref behind `base` (the root's own path, so
/// `aria-ref=f7e2` starts from the main frame). Playwright's `childDepth`: a depth limit that is
/// set and not zero, less the iframe's depth and its own level; JS's falsy `0` means no limit.
fn children(
    owner: usize,
    hops: &str,
    base: &str,
    depth: Option<i64>,
    iframes: &[AriaIframe],
) -> Vec<Pending> {
    iframes
        .iter()
        .filter_map(|iframe| {
            // A ref outside `eN` cannot become a path ref: its document could never be acted on.
            let number = iframe.r#ref.strip_prefix('e').filter(|number| {
                !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())
            })?;
            Some(Pending {
                slot: Slot {
                    owner,
                    iframe: iframe.r#ref.clone(),
                    prefix: format!("f{number}"),
                },
                hops: format!("{hops}aria-ref={base}{}", iframe.r#ref),
                depth: depth
                    .filter(|depth| *depth != 0)
                    .map(|depth| depth.saturating_sub(iframe.depth).saturating_sub(1)),
            })
        })
        .collect()
}

/// One iframe's document, as Playwright's nested `ariaSnapshotJSONForFrame` reads it with a
/// selector: polled with the root's waits while its world is gone (a frame mid-navigation), and
/// `None` once nothing matches, the page throws or answers outside the spec, or the call runs
/// out of time, all of which leave the iframe node as the page gave it. Playwright catches every
/// child error; Neo still ends the call when the tab closes or the run ends, as every call does.
async fn snapshot_child<T: ReadTab>(
    tab: &T,
    options: &SnapshotOptions,
    progress: &Progress,
    child: Pending,
) -> (Pending, Result<Option<AriaFrameSnapshot>, CallError>) {
    // Non-strict: a frameset document has several `frameset` elements.
    let params = InjectedAriaSnapshotParams {
        selector: format!("{}{FRAME_HOP}{FRAME_ROOT}", child.hops),
        strict: false,
        mode: options.mode,
        depth: child.depth,
        boxes: options.boxes,
    };
    let mut attempt: usize = 0;
    let snapshot = loop {
        let wait = SNAPSHOT_WAITS_MS[attempt.min(SNAPSHOT_WAITS_MS.len() - 1)];
        attempt = attempt.saturating_add(1);
        if wait > 0
            && let Err(error) = progress.wait(Duration::from_millis(wait)).await
        {
            break out_of_time(error);
        }
        match progress.race(tab.aria_snapshot(&params, true)).await {
            Ok(Ok(read)) => break Ok(read.result.snapshot),
            Ok(Err(PageError::NotConnected)) => {}
            Ok(Err(PageError::Threw(_) | PageError::Contract(_))) => break Ok(None),
            Ok(Err(PageError::Closed)) => break Err(PlaywrightError::target_closed().into()),
            Err(error) => break out_of_time(error),
        }
    };
    (child, snapshot)
}

/// A child read stopped by the call's own deadline gives up quietly, so the call still answers
/// with every document it did read, as Playwright's does; any other stop ends the call.
fn out_of_time(error: CallError) -> Result<Option<AriaFrameSnapshot>, CallError> {
    match error {
        CallError::Playwright(PlaywrightError {
            name: ErrorName::TimeoutError,
            ..
        }) => Ok(None),
        error => Err(error),
    }
}

/// Playwright's `mergeIframeChildren`: each `iframe` node whose ref has a stitched document takes
/// that document as its children, dropping the node's own text (an iframe's fallback content).
/// Never looks inside an iframe node, so only this document's own iframes are matched.
fn merge_iframes(nodes: &mut [AriaNode], stitched: &mut HashMap<String, Vec<AriaNode>>) {
    if stitched.is_empty() {
        return;
    }
    let mut stack: Vec<&mut AriaNode> = nodes.iter_mut().collect();
    while let Some(node) = stack.pop() {
        if node.role == AriaRole::Iframe
            && let Some(aria_ref) = &node.r#ref
        {
            if let Some(document) = stitched.remove(aria_ref) {
                node.text = None;
                node.children = Some(
                    document
                        .into_iter()
                        .map(|child| AriaChild::Node(Box::new(child)))
                        .collect(),
                );
            }
            continue;
        }
        stack.extend(child_nodes(node));
    }
}

/// Prefixes every ref in `nodes` and below.
fn prefix_refs(nodes: &mut [AriaNode], prefix: &str) {
    let mut stack: Vec<&mut AriaNode> = nodes.iter_mut().collect();
    while let Some(node) = stack.pop() {
        if let Some(aria_ref) = &mut node.r#ref {
            aria_ref.insert_str(0, prefix);
        }
        stack.extend(child_nodes(node));
    }
}

/// Neo: the main-rooted path of the document behind `owners` (main frame first): `f<N>` for each
/// iframe, `N` the number of the ref its document's snapshots give it (`frameOwnerRef`, which
/// leaves that document's last snapshot, and so every ref the caller holds there, as it was).
async fn owner_path(owners: &[FrameOwner]) -> Result<String, PageError> {
    let mut path = String::new();
    for owner in owners {
        let named = owner
            .frame
            .call(
                None,
                &InjectedFrameOwnerRefParams {
                    element: owner.element.clone(),
                },
            )
            .await?;
        let number = named
            .r#ref
            .as_deref()
            .and_then(|aria_ref| aria_ref.strip_prefix('e'))
            .filter(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()));
        let Some(number) = number else {
            return Err(PageError::Threw(UNNAMED_FRAME.to_string()));
        };
        path.push('f');
        path.push_str(number);
    }
    Ok(path)
}

/// A node's child nodes, skipping its text runs.
fn child_nodes(node: &mut AriaNode) -> impl Iterator<Item = &mut AriaNode> {
    node.children
        .iter_mut()
        .flatten()
        .filter_map(|child| match child {
            AriaChild::Node(node) => Some(node.as_mut()),
            AriaChild::Text(_) => None,
        })
}

// ---------------------------------------------------------------------------------------------
// The tab, for reads.
// ---------------------------------------------------------------------------------------------

/// The same steps click takes, so a read and a click prepare the tab and name a selector alike.
impl SelectorTab for FrameTab {
    fn prepare(&self) -> impl Future<Output = Result<(), PageError>> + Send {
        click::Tab::prepare(self)
    }

    fn describe(
        &self,
        selector: &str,
    ) -> impl Future<Output = Result<SelectorFacts, PageError>> + Send {
        click::Tab::describe(self, selector)
    }
}

/// By value, with no object group: none of these results holds a page object.
impl ReadTab for FrameTab {
    async fn text_content(
        &self,
        selector: &str,
        strict: bool,
    ) -> Result<Option<TextMatch>, PageError> {
        let Some(located) = frame_selector::locate(self, selector, strict).await? else {
            return Ok(None);
        };
        let read = located
            .frame
            .call(
                None,
                &InjectedTextContentParams {
                    selector: located.selector,
                    strict,
                },
            )
            .await?;
        Ok(read.matched)
    }

    async fn inner_text(
        &self,
        selector: &str,
        strict: bool,
    ) -> Result<Option<TextMatch>, PageError> {
        let Some(located) = frame_selector::locate(self, selector, strict).await? else {
            return Ok(None);
        };
        let read = located
            .frame
            .call(
                None,
                &InjectedInnerTextParams {
                    selector: located.selector,
                    strict,
                },
            )
            .await?;
        Ok(read.matched)
    }

    async fn inner_html(
        &self,
        selector: &str,
        strict: bool,
    ) -> Result<Option<TextMatch>, PageError> {
        let Some(located) = frame_selector::locate(self, selector, strict).await? else {
            return Ok(None);
        };
        let read = located
            .frame
            .call(
                None,
                &InjectedInnerHTMLParams {
                    selector: located.selector,
                    strict,
                },
            )
            .await?;
        Ok(read.matched)
    }

    async fn get_attribute(
        &self,
        selector: &str,
        strict: bool,
        name: &str,
    ) -> Result<Option<AttributeMatch>, PageError> {
        let Some(located) = frame_selector::locate(self, selector, strict).await? else {
            return Ok(None);
        };
        let read = located
            .frame
            .call(
                None,
                &InjectedGetAttributeParams {
                    selector: located.selector,
                    strict,
                    name: name.to_string(),
                },
            )
            .await?;
        Ok(read.matched)
    }

    /// Through `locate`, like the other reads: an iframe's document is reached by the hops in
    /// front of `body,frameset` (`aria-ref=e7 >> internal:control=enter-frame >> …`). An AI read
    /// the caller asked for inside a child frame names its document by the iframes on the way
    /// from the main frame, whether a frame locator, a `Frame` receiver or a path ref reached it.
    async fn aria_snapshot(
        &self,
        params: &InjectedAriaSnapshotParams,
        stitching: bool,
    ) -> Result<AriaRead, PageError> {
        let Some(located) = frame_selector::locate(self, &params.selector, params.strict).await?
        else {
            return Ok(AriaRead {
                result: InjectedAriaSnapshotResult {
                    snapshot: None,
                    preview: None,
                },
                base: String::new(),
            });
        };
        // The path first: behind an iframe that gets no ref, the call throws before the frame's
        // document is snapshotted, so its refs keep their lifetime.
        let base = if !stitching
            && params.mode == AriaSnapshotMode::Ai
            && located.frame.id != self.frames.main().id
        {
            owner_path(&located.owners).await?
        } else {
            String::new()
        };
        let frame_params = InjectedAriaSnapshotParams {
            selector: located.selector.clone(),
            ..params.clone()
        };
        let result = located.frame.call(None, &frame_params).await?;
        Ok(AriaRead { result, base })
    }
}

#[cfg(test)]
mod tests {
    //! Both loops against [`FakeReads`] under paused tokio time, so their backoff waits pass at
    //! once. Each page step takes 1 ms, a loopback CDP round trip, as in click's tests. The tests
    //! check what the call answers and which page steps it took, never its call log or schedule.

    use std::collections::VecDeque;
    use std::sync::Mutex;

    use neo_runtime_spec::serve::{FrameGetAttributeResult, FrameInnerTextResult};
    use tokio::sync::Barrier;
    use tokio::time;
    use tokio::time::Instant;

    use super::*;

    const ROUND_TRIP: Duration = Duration::from_millis(1);
    const SELECTOR: &str = "internal:role=listitem";

    /// A scripted page step: each call takes `duration` and pops the next answer; the last
    /// answer repeats.
    struct Step<T> {
        duration: Duration,
        answers: Mutex<VecDeque<T>>,
    }

    impl<T: Clone> Step<T> {
        fn new(duration: Duration, answers: impl IntoIterator<Item = T>) -> Self {
            Self {
                duration,
                answers: Mutex::new(answers.into_iter().collect()),
            }
        }

        fn always(duration: Duration, answer: T) -> Self {
            Self::new(duration, [answer])
        }

        async fn answer(&self) -> T {
            let answer = {
                let mut answers = self.answers.lock().expect("a fake step's lock");
                if answers.len() > 1 {
                    answers.pop_front().expect("checked non-empty")
                } else {
                    answers.front().cloned().expect("a fake step has an answer")
                }
            };
            time::sleep(self.duration).await;
            answer
        }
    }

    /// A scripted tab. Every page step records its name.
    struct FakeReads {
        prepare: Step<Result<(), PageError>>,
        describe: Step<Result<SelectorFacts, PageError>>,
        text: Step<Result<Option<TextMatch>, PageError>>,
        inner: Step<Result<Option<TextMatch>, PageError>>,
        html: Step<Result<Option<TextMatch>, PageError>>,
        attribute: Step<Result<Option<AttributeMatch>, PageError>>,
        read_params: Mutex<Vec<(String, bool, Option<String>)>>,
        /// The call frame's document, `body,frameset`, and a locator's subtree, `SELECTOR`.
        snapshot: Step<Result<Option<AriaFrameSnapshot>, PageError>>,
        /// Each iframe's document, by the full selector that reaches it.
        frames: HashMap<String, Step<Result<Option<AriaFrameSnapshot>, PageError>>>,
        /// Frame selectors whose snapshots each wait, before answering, until all of them have
        /// started: they complete only when they are read concurrently.
        meet: Option<(Vec<String>, Barrier)>,
        /// The path every read the caller asked for answers in front of its refs (`f7` for a
        /// root inside a child frame); empty for the main frame.
        base: String,
        calls: Mutex<Vec<String>>,
        snapshot_params: Mutex<Vec<InjectedAriaSnapshotParams>>,
    }

    impl FakeReads {
        /// A list item that every step finds at once.
        fn ready() -> Self {
            Self {
                prepare: Step::always(Duration::ZERO, Ok(())),
                describe: Step::always(
                    ROUND_TRIP,
                    Ok(SelectorFacts {
                        locator: "getByRole('listitem')".to_string(),
                        invalid: None,
                    }),
                ),
                text: Step::always(ROUND_TRIP, Ok(Some(apples()))),
                inner: Step::always(ROUND_TRIP, Ok(Some(apples()))),
                html: Step::always(ROUND_TRIP, Ok(Some(markup()))),
                attribute: Step::always(ROUND_TRIP, Ok(Some(attribute(Some("fruit"))))),
                read_params: Mutex::default(),
                snapshot: Step::always(ROUND_TRIP, Ok(Some(tree()))),
                frames: HashMap::new(),
                meet: None,
                base: String::new(),
                calls: Mutex::default(),
                snapshot_params: Mutex::default(),
            }
        }

        /// A tab whose page is `root`, with `frames`' documents behind their selectors.
        fn framed(
            root: AriaFrameSnapshot,
            frames: impl IntoIterator<Item = (String, Result<Option<AriaFrameSnapshot>, PageError>)>,
        ) -> Self {
            let mut tab = Self::ready();
            tab.snapshot = Step::always(ROUND_TRIP, Ok(Some(root)));
            tab.frames = frames
                .into_iter()
                .map(|(selector, answer)| (selector, Step::always(ROUND_TRIP, answer)))
                .collect();
            tab
        }

        /// The selectors of every snapshot taken, in order.
        fn snapshot_selectors(&self) -> Vec<String> {
            self.snapshot_params
                .lock()
                .expect("params")
                .iter()
                .map(|params| params.selector.clone())
                .collect()
        }

        fn record(&self, call: &str) {
            self.calls.lock().expect("calls").push(call.to_string());
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("calls").clone()
        }

        fn count(&self, name: &str) -> usize {
            self.calls().iter().filter(|call| *call == name).count()
        }
    }

    impl SelectorTab for FakeReads {
        async fn prepare(&self) -> Result<(), PageError> {
            self.prepare.answer().await
        }

        async fn describe(&self, _selector: &str) -> Result<SelectorFacts, PageError> {
            self.record("describe");
            self.describe.answer().await
        }
    }

    impl ReadTab for FakeReads {
        async fn text_content(
            &self,
            _selector: &str,
            _strict: bool,
        ) -> Result<Option<TextMatch>, PageError> {
            self.record("textContent");
            self.text.answer().await
        }

        async fn inner_text(
            &self,
            selector: &str,
            strict: bool,
        ) -> Result<Option<TextMatch>, PageError> {
            self.record("innerText");
            self.read_params
                .lock()
                .expect("params")
                .push((selector.to_string(), strict, None));
            self.inner.answer().await
        }

        async fn inner_html(
            &self,
            selector: &str,
            strict: bool,
        ) -> Result<Option<TextMatch>, PageError> {
            self.record("innerHTML");
            self.read_params
                .lock()
                .expect("params")
                .push((selector.to_string(), strict, None));
            self.html.answer().await
        }

        async fn get_attribute(
            &self,
            selector: &str,
            strict: bool,
            name: &str,
        ) -> Result<Option<AttributeMatch>, PageError> {
            self.record("getAttribute");
            self.read_params.lock().expect("params").push((
                selector.to_string(),
                strict,
                Some(name.to_string()),
            ));
            self.attribute.answer().await
        }

        async fn aria_snapshot(
            &self,
            params: &InjectedAriaSnapshotParams,
            stitching: bool,
        ) -> Result<AriaRead, PageError> {
            self.record("ariaSnapshot");
            self.snapshot_params
                .lock()
                .expect("params")
                .push(params.clone());
            // The call frame's document and a locator's own subtree answer from `snapshot`; every
            // other selector reaches an iframe's document and must be scripted in `frames`.
            let snapshot = if params.selector == FRAME_ROOT || !params.selector.contains(FRAME_HOP)
            {
                self.snapshot.answer().await?
            } else {
                if let Some((selectors, barrier)) = &self.meet
                    && selectors.contains(&params.selector)
                {
                    barrier.wait().await;
                }
                self.frames
                    .get(&params.selector)
                    .unwrap_or_else(|| panic!("no document scripted for {}", params.selector))
                    .answer()
                    .await?
            };
            let preview = snapshot.as_ref().map(|_| "<ul>…</ul>".to_string());
            Ok(AriaRead {
                result: InjectedAriaSnapshotResult { snapshot, preview },
                base: if stitching {
                    String::new()
                } else {
                    self.base.clone()
                },
            })
        }
    }

    fn apples() -> TextMatch {
        TextMatch {
            value: "Apples".to_string(),
            preview: "<li>Apples</li>".to_string(),
        }
    }

    fn markup() -> TextMatch {
        TextMatch {
            value: "<b>Apples</b> &amp; pears".to_string(),
            preview: "<li>Apples &amp; pears</li>".to_string(),
        }
    }

    async fn read_html(
        tab: &FakeReads,
        timeout_ms: f64,
    ) -> Result<FrameInnerHTMLResult, CallError> {
        inner_html(
            tab,
            &ReadOptions {
                selector: SELECTOR.to_string(),
                strict: true,
            },
            &Progress::for_test(timeout_ms),
        )
        .await
    }

    fn attribute(value: Option<&str>) -> AttributeMatch {
        AttributeMatch {
            value: value.map(str::to_string),
            preview: "<li data-kind=fruit>Apples</li>".to_string(),
        }
    }

    async fn read_inner(
        tab: &FakeReads,
        timeout_ms: f64,
    ) -> Result<FrameInnerTextResult, CallError> {
        inner_text(
            tab,
            &ReadOptions {
                selector: SELECTOR.to_string(),
                strict: true,
            },
            &Progress::for_test(timeout_ms),
        )
        .await
    }

    async fn read_attribute(
        tab: &FakeReads,
        timeout_ms: f64,
    ) -> Result<FrameGetAttributeResult, CallError> {
        get_attribute(
            tab,
            &AttributeOptions {
                selector: SELECTOR.to_string(),
                strict: true,
                name: "data-kind".to_string(),
            },
            &Progress::for_test(timeout_ms),
        )
        .await
    }

    fn node(role: AriaRole) -> AriaNode {
        AriaNode {
            role,
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

    /// `- list:` with one item, as an ai snapshot gives it.
    fn tree() -> AriaFrameSnapshot {
        let item = AriaNode {
            text: Some("Apples".to_string()),
            r#ref: Some("e2".to_string()),
            ..node(AriaRole::Listitem)
        };
        AriaFrameSnapshot {
            nodes: vec![AriaNode {
                children: Some(vec![AriaChild::Node(Box::new(item))]),
                r#ref: Some("e1".to_string()),
                ..node(AriaRole::List)
            }],
            iframes: Vec::new(),
        }
    }

    const TREE_YAML: &str = "- list [ref=e1]:\n  - listitem [ref=e2]: Apples";

    async fn read(tab: &FakeReads, timeout_ms: f64) -> Result<FrameTextContentResult, CallError> {
        let progress = Progress::for_test(timeout_ms);
        let options = ReadOptions {
            selector: SELECTOR.to_string(),
            strict: true,
        };
        text_content(tab, &options, &progress).await
    }

    async fn snapshot(
        tab: &FakeReads,
        timeout_ms: f64,
    ) -> Result<FrameAriaSnapshotResult, CallError> {
        let progress = Progress::for_test(timeout_ms);
        let options = SnapshotOptions {
            selector: None,
            mode: AriaSnapshotMode::Ai,
            depth: None,
            boxes: None,
        };
        aria_snapshot(tab, &options, &progress).await
    }

    fn timeout(timeout_ms: f64) -> CallError {
        PlaywrightError::timeout(timeout_ms).into()
    }

    fn text(value: &str) -> FrameTextContentResult {
        FrameTextContentResult {
            value: Some(value.to_string()),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_match_is_read_at_once() {
        let tab = FakeReads::ready();
        assert_eq!(read(&tab, 30_000.0).await, Ok(text("Apples")));
        assert_eq!(tab.calls(), ["describe", "textContent"]);
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_matching_polls_until_the_timeout() {
        let mut tab = FakeReads::ready();
        tab.text = Step::always(ROUND_TRIP, Ok(None));

        assert_eq!(read(&tab, 500.0).await, Err(timeout(500.0)));
        assert!(tab.count("textContent") > 1, "it polled: {:?}", tab.calls());
        assert_eq!(tab.count("describe"), 1, "the selector is described once");
    }

    #[tokio::test(start_paused = true)]
    async fn an_element_appended_later_is_read_once_it_exists() {
        let mut tab = FakeReads::ready();
        tab.text = Step::new(
            ROUND_TRIP,
            [Ok(None), Ok(None), Ok(None), Ok(Some(apples()))],
        );
        assert_eq!(read(&tab, 30_000.0).await, Ok(text("Apples")));
    }

    /// A strict mode violation ends the read at once with the page's message, before any wait.
    #[tokio::test(start_paused = true)]
    async fn a_strict_mode_violation_fails_at_once_with_the_pages_message() {
        let message = "Error: strict mode violation: getByRole('listitem') resolved to 2 elements";
        let mut tab = FakeReads::ready();
        tab.text = Step::always(ROUND_TRIP, Err(PageError::Threw(message.to_string())));
        let started = Instant::now();

        assert_eq!(
            read(&tab, 30_000.0).await,
            Err(PlaywrightError::error(message).into())
        );
        assert_eq!(tab.count("textContent"), 1);
        assert!(
            started.elapsed() < Duration::from_millis(20),
            "no backoff wait before failing: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_invalid_selector_fails_at_once_and_reads_nothing() {
        let mut tab = FakeReads::ready();
        tab.describe = Step::always(
            ROUND_TRIP,
            Ok(SelectorFacts {
                locator: "li >>".to_string(),
                invalid: Some("Unexpected token".to_string()),
            }),
        );
        assert_eq!(
            read(&tab, 30_000.0).await,
            Err(PlaywrightError::error("Unexpected token").into())
        );
        assert_eq!(tab.calls(), ["describe"], "nothing is read");
    }

    #[tokio::test(start_paused = true)]
    async fn a_document_that_goes_mid_read_is_read_again() {
        let mut tab = FakeReads::ready();
        tab.text = Step::new(
            ROUND_TRIP,
            [Err(PageError::NotConnected), Ok(Some(apples()))],
        );
        assert_eq!(read(&tab, 30_000.0).await, Ok(text("Apples")));

        let mut tab = FakeReads::ready();
        tab.prepare = Step::new(Duration::ZERO, [Err(PageError::NotConnected), Ok(())]);
        assert_eq!(
            read(&tab, 30_000.0).await,
            Ok(text("Apples")),
            "a failed preparation retries too"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_closed_tab_or_a_broken_contract_ends_the_read() {
        let mut tab = FakeReads::ready();
        tab.text = Step::always(ROUND_TRIP, Err(PageError::Closed));
        let result = read(&tab, 30_000.0).await;
        let Err(CallError::Playwright(error)) = result else {
            panic!("expected a Playwright error, got {result:?}");
        };
        assert_eq!(error.name, ErrorName::TargetClosedError);

        let mut tab = FakeReads::ready();
        tab.text = Step::always(
            ROUND_TRIP,
            Err(PageError::Contract("textContent answered 5".to_string())),
        );
        let result = read(&tab, 30_000.0).await;
        let Err(CallError::Playwright(error)) = result else {
            panic!("expected a Playwright error, got {result:?}");
        };
        assert_eq!(error.name, ErrorName::Error);
    }

    #[tokio::test(start_paused = true)]
    async fn a_page_that_never_attaches_times_out_without_reading() {
        let mut tab = FakeReads::ready();
        tab.describe = Step::always(ROUND_TRIP, Err(PageError::NotConnected));

        assert_eq!(read(&tab, 500.0).await, Err(timeout(500.0)));
        assert_eq!(tab.count("textContent"), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn an_empty_text_is_a_value_not_a_miss() {
        let mut tab = FakeReads::ready();
        tab.text = Step::always(
            ROUND_TRIP,
            Ok(Some(TextMatch {
                value: String::new(),
                preview: "<li></li>".to_string(),
            })),
        );
        assert_eq!(read(&tab, 30_000.0).await, Ok(text("")));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_inner_text_reads_the_match_at_once() {
        let tab = FakeReads::ready();
        assert_eq!(
            read_inner(&tab, 30_000.0).await,
            Ok(FrameInnerTextResult {
                value: "Apples".to_string()
            })
        );
        assert_eq!(tab.calls(), ["describe", "innerText"]);
        assert_eq!(
            *tab.read_params.lock().expect("params"),
            [(SELECTOR.to_string(), true, None)]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_inner_text_preserves_empty_text() {
        let mut tab = FakeReads::ready();
        tab.inner = Step::always(
            ROUND_TRIP,
            Ok(Some(TextMatch {
                value: String::new(),
                ..apples()
            })),
        );
        assert_eq!(
            read_inner(&tab, 30_000.0).await,
            Ok(FrameInnerTextResult {
                value: String::new()
            })
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_inner_text_waits_for_a_later_element() {
        let mut tab = FakeReads::ready();
        tab.inner = Step::new(ROUND_TRIP, [Ok(None), Ok(Some(apples()))]);
        assert_eq!(
            read_inner(&tab, 30_000.0).await,
            Ok(FrameInnerTextResult {
                value: "Apples".to_string()
            })
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_inner_text_times_out_without_an_element() {
        let mut tab = FakeReads::ready();
        tab.inner = Step::always(ROUND_TRIP, Ok(None));
        assert_eq!(read_inner(&tab, 500.0).await, Err(timeout(500.0)));
        assert!(tab.count("innerText") > 1);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_inner_text_fails_at_once_for_non_html_and_strict_violations() {
        for message in [
            "Error: Node is not an HTMLElement",
            "Error: strict mode violation",
        ] {
            let mut tab = FakeReads::ready();
            tab.inner = Step::always(ROUND_TRIP, Err(PageError::Threw(message.to_string())));
            let started = Instant::now();
            assert_eq!(
                read_inner(&tab, 30_000.0).await,
                Err(PlaywrightError::error(message).into())
            );
            assert!(started.elapsed() < Duration::from_millis(20));
            assert_eq!(tab.count("innerText"), 1);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_inner_text_retries_a_replaced_document() {
        let mut tab = FakeReads::ready();
        tab.inner = Step::new(
            ROUND_TRIP,
            [Err(PageError::NotConnected), Ok(Some(apples()))],
        );
        assert_eq!(
            read_inner(&tab, 30_000.0).await,
            Ok(FrameInnerTextResult {
                value: "Apples".to_string()
            })
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_inner_html_reads_the_match_and_its_strictness() {
        let tab = FakeReads::ready();
        assert_eq!(
            read_html(&tab, 30_000.0).await,
            Ok(FrameInnerHTMLResult {
                value: "<b>Apples</b> &amp; pears".to_string()
            })
        );
        assert_eq!(tab.calls(), ["describe", "innerHTML"]);
        assert_eq!(
            *tab.read_params.lock().expect("params"),
            [(SELECTOR.to_string(), true, None)]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_inner_html_waits_for_a_later_element_and_times_out_without_one() {
        let mut later = FakeReads::ready();
        later.html = Step::new(
            ROUND_TRIP,
            [Ok(None), Err(PageError::NotConnected), Ok(Some(markup()))],
        );
        assert_eq!(
            read_html(&later, 30_000.0).await,
            Ok(FrameInnerHTMLResult {
                value: "<b>Apples</b> &amp; pears".to_string()
            })
        );
        let mut never = FakeReads::ready();
        never.html = Step::always(ROUND_TRIP, Ok(None));
        assert_eq!(read_html(&never, 500.0).await, Err(timeout(500.0)));
        assert!(never.count("innerHTML") > 1);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_inner_html_fails_at_once_for_a_strict_violation() {
        let mut tab = FakeReads::ready();
        tab.html = Step::always(
            ROUND_TRIP,
            Err(PageError::Threw("Error: strict mode violation".to_string())),
        );
        assert_eq!(
            read_html(&tab, 30_000.0).await,
            Err(PlaywrightError::error("Error: strict mode violation").into())
        );
        assert_eq!(tab.count("innerHTML"), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_get_attribute_reads_the_match_at_once() {
        let tab = FakeReads::ready();
        assert_eq!(
            read_attribute(&tab, 30_000.0).await,
            Ok(FrameGetAttributeResult {
                value: Some("fruit".to_string())
            })
        );
        assert_eq!(tab.calls(), ["describe", "getAttribute"]);
        assert_eq!(
            *tab.read_params.lock().expect("params"),
            [(SELECTOR.to_string(), true, Some("data-kind".to_string()))]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_missing_attribute_returns_at_once_without_waiting_for_its_value() {
        let mut tab = FakeReads::ready();
        tab.attribute = Step::new(
            ROUND_TRIP,
            [
                Ok(Some(attribute(None))),
                Ok(Some(attribute(Some("later")))),
            ],
        );
        let started = Instant::now();
        assert_eq!(
            read_attribute(&tab, 30_000.0).await,
            Ok(FrameGetAttributeResult { value: None })
        );
        assert!(started.elapsed() < Duration::from_millis(20));
        assert_eq!(tab.count("getAttribute"), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_get_attribute_preserves_an_empty_value() {
        let mut tab = FakeReads::ready();
        tab.attribute = Step::always(ROUND_TRIP, Ok(Some(attribute(Some("")))));
        assert_eq!(
            read_attribute(&tab, 30_000.0).await,
            Ok(FrameGetAttributeResult {
                value: Some(String::new())
            })
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_get_attribute_times_out_without_an_element() {
        let mut tab = FakeReads::ready();
        tab.attribute = Step::always(ROUND_TRIP, Ok(None));
        assert_eq!(read_attribute(&tab, 500.0).await, Err(timeout(500.0)));
        assert!(tab.count("getAttribute") > 1);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_get_attribute_waits_for_a_later_element() {
        let mut tab = FakeReads::ready();
        tab.attribute = Step::new(ROUND_TRIP, [Ok(None), Ok(Some(attribute(None)))]);
        assert_eq!(
            read_attribute(&tab, 30_000.0).await,
            Ok(FrameGetAttributeResult { value: None })
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_get_attribute_retries_a_replaced_document() {
        let mut tab = FakeReads::ready();
        tab.attribute = Step::new(
            ROUND_TRIP,
            [
                Err(PageError::NotConnected),
                Ok(Some(attribute(Some("fruit")))),
            ],
        );
        assert_eq!(
            read_attribute(&tab, 30_000.0).await,
            Ok(FrameGetAttributeResult {
                value: Some("fruit".to_string())
            })
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_get_attribute_fails_at_once_for_a_strict_violation() {
        let mut tab = FakeReads::ready();
        let message = "Error: strict mode violation";
        tab.attribute = Step::always(ROUND_TRIP, Err(PageError::Threw(message.to_string())));
        let started = Instant::now();
        assert_eq!(
            read_attribute(&tab, 30_000.0).await,
            Err(PlaywrightError::error(message).into())
        );
        assert!(started.elapsed() < Duration::from_millis(20));
        assert_eq!(tab.count("getAttribute"), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_both_text_reads_fail_on_invalid_selectors_before_reading() {
        let mut tab = FakeReads::ready();
        tab.describe = Step::always(
            ROUND_TRIP,
            Ok(SelectorFacts {
                locator: "li >>".to_string(),
                invalid: Some("Unexpected token".to_string()),
            }),
        );
        let error = Err(PlaywrightError::error("Unexpected token").into());
        assert_eq!(
            read_inner(&tab, 30_000.0).await.map(|result| result.value),
            error
        );
        assert_eq!(
            read_attribute(&tab, 30_000.0)
                .await
                .map(|result| result.value.unwrap_or_default()),
            error
        );
        assert_eq!(tab.calls(), ["describe", "describe"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_snapshot_renders_the_documents_tree_as_yaml() {
        let tab = FakeReads::ready();
        assert_eq!(
            snapshot(&tab, 30_000.0).await,
            Ok(FrameAriaSnapshotResult {
                snapshot: TREE_YAML.to_string()
            })
        );
        assert_eq!(
            tab.snapshot_params.lock().expect("params").as_slice(),
            [InjectedAriaSnapshotParams {
                selector: "body,frameset".to_string(),
                strict: false,
                mode: AriaSnapshotMode::Ai,
                depth: None,
                boxes: None,
            }],
            "the whole document, not strict, in the caller's mode"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_document_without_a_body_is_snapshotted_once_it_has_one() {
        let mut tab = FakeReads::ready();
        tab.snapshot = Step::new(ROUND_TRIP, [Ok(None), Ok(None), Ok(Some(tree()))]);
        assert_eq!(
            snapshot(&tab, 30_000.0).await,
            Ok(FrameAriaSnapshotResult {
                snapshot: TREE_YAML.to_string()
            })
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_document_that_never_gets_a_body_times_out() {
        let mut tab = FakeReads::ready();
        tab.snapshot = Step::always(ROUND_TRIP, Ok(None));

        assert_eq!(snapshot(&tab, 1500.0).await, Err(timeout(1500.0)));
        assert!(
            tab.count("ariaSnapshot") > 1,
            "it polled: {:?}",
            tab.calls()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_snapshot_retries_a_gone_document_and_ends_on_an_exception_or_close() {
        let mut tab = FakeReads::ready();
        tab.snapshot = Step::new(ROUND_TRIP, [Err(PageError::NotConnected), Ok(Some(tree()))]);
        assert!(snapshot(&tab, 30_000.0).await.is_ok());

        let message = "Error: Can only capture aria snapshot of Element nodes.";
        let mut tab = FakeReads::ready();
        tab.snapshot = Step::always(ROUND_TRIP, Err(PageError::Threw(message.to_string())));
        assert_eq!(
            snapshot(&tab, 30_000.0).await,
            Err(PlaywrightError::error(message).into())
        );

        let mut tab = FakeReads::ready();
        tab.prepare = Step::always(Duration::ZERO, Err(PageError::Closed));
        assert_eq!(
            snapshot(&tab, 30_000.0).await,
            Err(PlaywrightError::target_closed().into())
        );
    }

    /// A node with a ref, and `children` when there are any.
    fn element(
        role: AriaRole,
        name: Option<&str>,
        aria_ref: &str,
        children: Vec<AriaNode>,
    ) -> AriaNode {
        AriaNode {
            name: name.map(str::to_string),
            r#ref: Some(aria_ref.to_string()),
            children: (!children.is_empty()).then(|| {
                children
                    .into_iter()
                    .map(|child| AriaChild::Node(Box::new(child)))
                    .collect()
            }),
            ..node(role)
        }
    }

    /// An `iframe` node as the page gives it, with its fallback text when it has one.
    fn iframe_node(aria_ref: &str, fallback: Option<&str>) -> AriaNode {
        AriaNode {
            r#ref: Some(aria_ref.to_string()),
            text: fallback.map(str::to_string),
            ..node(AriaRole::Iframe)
        }
    }

    /// One document's answer: its nodes and its rendered iframes as `(ref, depth)`.
    fn document(nodes: Vec<AriaNode>, iframes: &[(&str, i64)]) -> AriaFrameSnapshot {
        AriaFrameSnapshot {
            nodes,
            iframes: iframes
                .iter()
                .map(|(aria_ref, depth)| AriaIframe {
                    r#ref: (*aria_ref).to_string(),
                    depth: *depth,
                })
                .collect(),
        }
    }

    /// The selector of the document behind `path`, one iframe ref per level from the call's
    /// frame.
    fn behind(path: &[&str]) -> String {
        path.iter()
            .map(|aria_ref| format!("aria-ref={aria_ref}"))
            .chain(["body,frameset".to_string()])
            .collect::<Vec<_>>()
            .join(" >> internal:control=enter-frame >> ")
    }

    const FALLBACK: &str = "Your browser does not support frames";

    /// `#same`'s document.
    fn same_document() -> AriaFrameSnapshot {
        document(
            vec![element(AriaRole::Button, Some("Same"), "e2", Vec::new())],
            &[],
        )
    }

    /// The page every stitching test reads: a heading, a frame with fallback text (`e3`), and a
    /// frame (`e4`) whose document holds a frame of its own (its `e3`).
    fn framed_page() -> FakeReads {
        let root = document(
            vec![element(
                AriaRole::Generic,
                None,
                "e1",
                vec![
                    AriaNode {
                        level: Some(1),
                        ..element(AriaRole::Heading, Some("Top"), "e2", Vec::new())
                    },
                    iframe_node("e3", Some(FALLBACK)),
                    iframe_node("e4", None),
                ],
            )],
            &[("e3", 1), ("e4", 1)],
        );
        let cross = document(
            vec![element(
                AriaRole::Generic,
                None,
                "e1",
                vec![
                    element(AriaRole::Textbox, Some("Email"), "e2", Vec::new()),
                    iframe_node("e3", None),
                ],
            )],
            &[("e3", 1)],
        );
        let leaf = document(
            vec![element(AriaRole::Button, Some("Leaf"), "e2", Vec::new())],
            &[],
        );
        FakeReads::framed(
            root,
            [
                (behind(&["e3"]), Ok(Some(same_document()))),
                (behind(&["e4"]), Ok(Some(cross))),
                (behind(&["e4", "e3"]), Ok(Some(leaf))),
            ],
        )
    }

    /// [`framed_page`] with every document stitched in.
    const FRAMED_YAML: &str = "- generic [ref=e1]:
  - heading \"Top\" [level=1] [ref=e2]
  - iframe [ref=e3]:
    - button \"Same\" [ref=f3e2]
  - iframe [ref=e4]:
    - generic [ref=f4e1]:
      - textbox \"Email\" [ref=f4e2]
      - iframe [ref=f4e3]:
        - button \"Leaf\" [ref=f4f3e2]";

    /// [`FRAMED_YAML`] with `e3` as the page gave it.
    const FRAMED_YAML_WITHOUT_SAME: &str = "- generic [ref=e1]:
  - heading \"Top\" [level=1] [ref=e2]
  - iframe [ref=e3]: Your browser does not support frames
  - iframe [ref=e4]:
    - generic [ref=f4e1]:
      - textbox \"Email\" [ref=f4e2]
      - iframe [ref=f4e3]:
        - button \"Leaf\" [ref=f4f3e2]";

    fn yaml(snapshot: &str) -> FrameAriaSnapshotResult {
        FrameAriaSnapshotResult {
            snapshot: snapshot.to_string(),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_an_ai_snapshot_stitches_each_frames_document_under_its_iframe() {
        let tab = framed_page();
        assert_eq!(snapshot(&tab, 30_000.0).await, Ok(yaml(FRAMED_YAML)));

        let selectors = tab.snapshot_selectors();
        assert_eq!(
            selectors[0], "body,frameset",
            "the call's own document first"
        );
        let mut frames = selectors[1..].to_vec();
        frames.sort();
        assert_eq!(
            frames,
            [
                "aria-ref=e3 >> internal:control=enter-frame >> body,frameset",
                "aria-ref=e4 >> internal:control=enter-frame >> aria-ref=e3 >> \
                 internal:control=enter-frame >> body,frameset",
                "aria-ref=e4 >> internal:control=enter-frame >> body,frameset",
            ],
            "each frame is entered through its iframe's ref, once"
        );
        let nested = selectors
            .iter()
            .position(|selector| *selector == behind(&["e4", "e3"]));
        let holder = selectors
            .iter()
            .position(|selector| *selector == behind(&["e4"]));
        assert!(
            nested > holder,
            "a frame is entered only once the document holding its iframe has answered: \
             {selectors:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_locator_snapshot_inside_a_frame_names_its_refs_from_the_main_frame() {
        let root = document(
            vec![element(
                AriaRole::Generic,
                None,
                "e1",
                vec![
                    element(AriaRole::Textbox, Some("Email"), "e2", Vec::new()),
                    iframe_node("e3", None),
                ],
            )],
            &[("e3", 1)],
        );
        let leaf = document(
            vec![element(AriaRole::Button, Some("Leaf"), "e2", Vec::new())],
            &[],
        );
        // The root is the frame behind the main frame's `e4`: its read answers that path.
        let mut tab = FakeReads::framed(root, [(behind(&["f4e3"]), Ok(Some(leaf)))]);
        tab.base = "f4".to_string();
        let options = SnapshotOptions {
            selector: Some(SELECTOR.to_string()),
            mode: AriaSnapshotMode::Ai,
            depth: None,
            boxes: None,
        };
        assert_eq!(
            aria_snapshot(&tab, &options, &Progress::for_test(30_000.0)).await,
            Ok(yaml(
                "- generic [ref=f4e1]:
  - textbox \"Email\" [ref=f4e2]
  - iframe [ref=f4e3]:
    - button \"Leaf\" [ref=f4f3e2]"
            )),
            "every ref takes the root's path, its own frames' refs after it"
        );
        assert_eq!(
            tab.snapshot_selectors()[1],
            behind(&["f4e3"]),
            "the root's iframe is entered by its path ref, from the main frame"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_every_frame_gets_the_callers_mode_and_boxes_and_its_own_depth() {
        let tab = framed_page();
        let options = SnapshotOptions {
            selector: None,
            mode: AriaSnapshotMode::Ai,
            depth: Some(4),
            boxes: Some(true),
        };
        let progress = Progress::for_test(30_000.0);
        assert!(aria_snapshot(&tab, &options, &progress).await.is_ok());

        let mut params = tab.snapshot_params.lock().expect("params").clone();
        params.sort_by(|a, b| a.selector.cmp(&b.selector));
        let seen: Vec<_> = params
            .iter()
            .map(|params| (params.selector.as_str(), params.depth))
            .collect();
        assert_eq!(
            seen,
            [
                (behind(&["e3"]).as_str(), Some(2)),
                (behind(&["e4", "e3"]).as_str(), Some(0)),
                (behind(&["e4"]).as_str(), Some(2)),
                ("body,frameset", Some(4)),
            ],
            "Playwright's childDepth: the parent's depth less the iframe's depth and one"
        );
        assert!(
            params.iter().all(|params| !params.strict
                && params.mode == AriaSnapshotMode::Ai
                && params.boxes == Some(true)),
            "{params:?}"
        );
    }

    #[test]
    fn tests_that_a_frames_depth_follows_playwrights_child_depth() {
        let depth_of = |depth: Option<i64>, iframe_depth: i64| {
            let iframe = AriaIframe {
                r#ref: "e1".to_string(),
                depth: iframe_depth,
            };
            children(0, "", "", depth, &[iframe])[0].depth
        };
        assert_eq!(depth_of(None, 2), None, "no limit stays none");
        assert_eq!(depth_of(Some(5), 2), Some(2));
        assert_eq!(
            depth_of(Some(3), 2),
            Some(0),
            "0 reaches the page, whose renderer reads it as no limit"
        );
        assert_eq!(
            depth_of(Some(2), 2),
            Some(-1),
            "an iframe at the limit still has its document read, without a limit, as in \
             Playwright"
        );
        assert_eq!(depth_of(Some(0), 2), None, "JS's falsy 0 is no limit");
        assert_eq!(
            depth_of(Some(i64::MIN), 2),
            Some(i64::MIN),
            "saturates, never overflows"
        );
    }

    #[test]
    fn tests_that_only_document_local_refs_are_entered() {
        let iframes: Vec<AriaIframe> = ["e3", "f1e2", "e", "e3x", "x3", "e12"]
            .into_iter()
            .map(|aria_ref| AriaIframe {
                r#ref: aria_ref.to_string(),
                depth: 0,
            })
            .collect();
        let entered: Vec<String> = children(0, "", "", None, &iframes)
            .into_iter()
            .map(|child| child.hops)
            .collect();
        assert_eq!(
            entered,
            ["aria-ref=e3", "aria-ref=e12"],
            "a ref a path ref cannot name is left as the page gave it"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_frame_that_cannot_be_read_leaves_its_iframe_as_the_page_gave_it() {
        for failure in [
            Ok(None),
            Ok(Some(document(Vec::new(), &[]))),
            Err(PageError::Threw(
                "Error: the frame's page threw".to_string(),
            )),
            Err(PageError::Contract("ariaSnapshot answered 5".to_string())),
        ] {
            let mut tab = framed_page();
            tab.frames
                .insert(behind(&["e3"]), Step::always(ROUND_TRIP, failure.clone()));
            assert_eq!(
                snapshot(&tab, 30_000.0).await,
                Ok(yaml(FRAMED_YAML_WITHOUT_SAME)),
                "{failure:?}: its fallback text stays, the other frames are stitched"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_frame_that_cannot_be_read_hides_the_frames_inside_it() {
        let mut tab = framed_page();
        tab.frames.insert(
            behind(&["e4"]),
            Step::always(ROUND_TRIP, Err(PageError::Threw("Error: no".to_string()))),
        );
        assert_eq!(
            snapshot(&tab, 30_000.0).await,
            Ok(yaml(
                "- generic [ref=e1]:
  - heading \"Top\" [level=1] [ref=e2]
  - iframe [ref=e3]:
    - button \"Same\" [ref=f3e2]
  - iframe [ref=e4]"
            ))
        );
        assert!(
            !tab.snapshot_selectors().contains(&behind(&["e4", "e3"])),
            "nothing is read through a frame that was not read"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_frame_whose_document_went_away_is_read_again() {
        let mut tab = framed_page();
        tab.frames.insert(
            behind(&["e3"]),
            Step::new(
                ROUND_TRIP,
                [Err(PageError::NotConnected), Ok(Some(same_document()))],
            ),
        );
        assert_eq!(snapshot(&tab, 30_000.0).await, Ok(yaml(FRAMED_YAML)));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_frame_still_gone_at_the_timeout_is_left_out_and_the_rest_answers() {
        let mut tab = framed_page();
        tab.frames.insert(
            behind(&["e3"]),
            Step::always(ROUND_TRIP, Err(PageError::NotConnected)),
        );
        assert_eq!(
            snapshot(&tab, 5_000.0).await,
            Ok(yaml(FRAMED_YAML_WITHOUT_SAME)),
            "Playwright answers with the frames it read when one keeps failing to the timeout"
        );
        let polls = tab
            .snapshot_selectors()
            .iter()
            .filter(|selector| **selector == behind(&["e3"]))
            .count();
        assert!(polls > 1, "the gone frame was polled");
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_the_tab_closing_while_a_frame_is_read_ends_the_call() {
        let mut tab = framed_page();
        tab.frames.insert(
            behind(&["e4", "e3"]),
            Step::always(ROUND_TRIP, Err(PageError::Closed)),
        );
        assert_eq!(
            snapshot(&tab, 30_000.0).await,
            Err(PlaywrightError::target_closed().into())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_sibling_frames_are_read_concurrently() {
        let mut tab = framed_page();
        // Each sibling's read waits until both have started: read one after the other, the
        // first would wait until the call's timeout and neither would be stitched.
        tab.meet = Some((vec![behind(&["e3"]), behind(&["e4"])], Barrier::new(2)));
        assert_eq!(snapshot(&tab, 30_000.0).await, Ok(yaml(FRAMED_YAML)));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_default_snapshot_has_no_refs_to_enter_so_its_iframes_render_as_given() {
        let mut tab = FakeReads::ready();
        // The glue lists no iframes in the default mode: Playwright gives no node a ref there.
        tab.snapshot = Step::always(
            ROUND_TRIP,
            Ok(Some(document(
                vec![AriaNode {
                    text: Some(FALLBACK.to_string()),
                    ..node(AriaRole::Iframe)
                }],
                &[],
            ))),
        );
        let options = SnapshotOptions {
            selector: None,
            mode: AriaSnapshotMode::Default,
            depth: None,
            boxes: None,
        };
        assert_eq!(
            aria_snapshot(&tab, &options, &Progress::for_test(30_000.0)).await,
            Ok(yaml("- iframe: Your browser does not support frames"))
        );
        assert_eq!(tab.snapshot_selectors(), ["body,frameset"]);
    }

    #[test]
    fn snapshot_options_accept_a_selector_and_default_the_mode() {
        let decode = |params: serde_json::Value| {
            SnapshotOptions::from_params(serde_json::from_value(params).expect("params"))
        };
        assert_eq!(
            decode(serde_json::json!({ "selector": "#list", "mode": "ai" })),
            SnapshotOptions {
                selector: Some("#list".to_string()),
                mode: AriaSnapshotMode::Ai,
                depth: None,
                boxes: None,
            }
        );
        assert_eq!(
            decode(serde_json::json!({ "selector": "", "depth": 2, "boxes": true })),
            SnapshotOptions {
                selector: None,
                mode: AriaSnapshotMode::Default,
                depth: Some(2),
                boxes: Some(true),
            },
            "an empty selector is none, as Playwright reads it"
        );
        assert_eq!(
            decode(serde_json::json!({ "mode": "ai" })).mode,
            AriaSnapshotMode::Ai
        );
    }

    // Neo: exercises scoped snapshots against the same scripted tab as page reads.
    async fn locator_snapshot(
        tab: &FakeReads,
        mode: &str,
    ) -> Result<FrameAriaSnapshotResult, CallError> {
        let options = SnapshotOptions::from_params(
            serde_json::from_value(serde_json::json!({
                "selector": SELECTOR,
                "mode": mode,
                "depth": 2,
            }))
            .expect("params"),
        );
        aria_snapshot(tab, &options, &Progress::for_test(30_000.0)).await
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_locator_snapshot_waits_for_its_subtree() {
        let mut tab = FakeReads::ready();
        tab.snapshot = Step::new(ROUND_TRIP, [Ok(None), Ok(Some(tree()))]);
        let result = locator_snapshot(&tab, "default").await.expect("snapshot");
        assert_eq!(result.snapshot, TREE_YAML);
        assert!(
            tab.snapshot_params
                .lock()
                .expect("params")
                .iter()
                .all(|params| {
                    params.selector == SELECTOR
                        && params.strict
                        && params.mode == AriaSnapshotMode::Default
                        && params.depth == Some(2)
                })
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_locator_snapshot_rejects_strict_violations() {
        for mode in ["default", "ai"] {
            let mut tab = FakeReads::ready();
            tab.snapshot = Step::always(
                ROUND_TRIP,
                Err(PageError::Threw("strict mode violation".to_string())),
            );
            let error = locator_snapshot(&tab, mode)
                .await
                .expect_err("strict violation");
            let CallError::Playwright(error) = error else {
                panic!("expected a Playwright error");
            };
            assert_eq!(error.name, ErrorName::Error);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_an_ai_locator_snapshot_rejects_no_match_without_polling() {
        let mut tab = FakeReads::ready();
        // A later match would succeed if the AI branch incorrectly auto-waited.
        tab.snapshot = Step::new(ROUND_TRIP, [Ok(None), Ok(Some(tree()))]);
        let error = locator_snapshot(&tab, "ai").await.expect_err("no match");
        let CallError::Playwright(error) = error else {
            panic!("expected a Playwright error");
        };
        assert_eq!(error.name, ErrorName::Error);
    }
}
