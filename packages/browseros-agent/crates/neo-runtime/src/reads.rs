//! `Frame.textContent` and `Frame.ariaSnapshot` (no selector, as `page.ariaSnapshot` sends it):
//! Playwright's page reads, in the structure of its server so the call logs and attempt counts
//! come out the same. `textContent` is `frames.ts` `_waitForFunctionOnSelector` over
//! `retryWithProgressAndBackoff`; `ariaSnapshot` is `page.ts` `ariaSnapshotJSONForFrame` over
//! `retryWithProgressAndTimeouts`, rendered by `aria_yaml.rs` as `frames.ts` `ariaSnapshot` does.
//!
//! ```text
//!  text_content(tab, options, progress)               log: waiting for <locator>
//!   wait_for_function_on_selector  waits 0,20,50,100,100,500…  (click's LoopAWaits)
//!   │ prepare the tab
//!   │ first time only: describe → name the locator │ invalid selector → Error, at once
//!   │ evaluate: glue textContent {selector, strict}  (marks the first match)
//!   │   matched → log "  locator resolved to <preview>" → {value}
//!   │   nothing matched, or NotConnected → poll again
//!   │   Threw (strict mode violation) or Contract → Error with the page's message; Closed → closed
//!  aria_snapshot(tab, options, progress)               no call log at all
//!   waits 0,1000,2000,4000,8000…  (never capped by the timeout; the last repeats)
//!   │ prepare → glue ariaSnapshot {selector: 'body,frameset', strict: false, mode, depth, boxes}
//!   │   no body yet, or NotConnected → poll again; Threw or Contract → Error; Closed → closed
//!   └ aria_yaml::render(nodes) → {snapshot}
//! ```
//!
//! The seam is two traits of their own, [`SelectorTab`] and [`ReadTab`], so the reads never grow
//! click's `Tab`; `Arc<PageSession>` implements both at the bottom of this file, over click's own
//! `prepare` and `describe` and the main frame's utility world. [`wait_for_function_on_selector`]
//! takes the page call as a closure, so any read of the first match (`inputValue`, `innerText`)
//! can reuse the loop. The tests drive both loops against a scripted tab under paused time.
//!
//! An `ai` snapshot also records its refs in the document's injected script, so a later
//! `aria-ref=eN` selector (any run's, since the utility world is per document) resolves the
//! element while it stays connected, until the next snapshot or a cross-document commit. Neo
//! serves the main frame only: an iframe's node renders as the page gave it, without the iframe's
//! document, which is what Playwright prints when it cannot read that document.
//!
//! Neo-only: a lone surrogate (a truncated emoji) in any page call's answer or thrown message
//! reads as U+FFFD (`neo-runtime-injected/src/well-formed.ts`); Rust strings are UTF-8, and a CDP
//! reply escaping one would be dropped. Playwright returns it unchanged.
//!
//! A tree nested deeper than about 60 aria levels never arrives: browseros-cdp parses every CDP
//! reply with `serde_json`'s 128-level limit and drops one it cannot parse, so the snapshot ends at
//! the call's timeout (on BrowserOS neo, 30 nested lists snapshot and 31 time out; GitHub, Hacker
//! News, Wikipedia and MDN pages measured 11 to 16 levels).

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use neo_runtime_spec::injected::{
    AriaFrameSnapshot, AriaSnapshotMode, InjectedAriaSnapshotParams, InjectedTextContentParams,
    TextMatch,
};
use neo_runtime_spec::serve::{
    FrameAriaSnapshotParams, FrameAriaSnapshotParamsMode, FrameAriaSnapshotResult,
    FrameTextContentParams, FrameTextContentResult,
};

use crate::aria_yaml;
use crate::click::{self, LoopAWaits, PageError, SelectorFacts};
use crate::page::PageSession;
use crate::progress::{CallError, PlaywrightError, Progress};

/// The selector Playwright snapshots a frame's document by when the caller gave none
/// (`page.ts` `ariaSnapshotJSONForFrame`): a `<frameset>` document has no `<body>`.
const FRAME_ROOT: &str = "body,frameset";

/// The waits before each snapshot attempt: Playwright's `[1000, 2000, 4000, 8000]` after an
/// immediate first try, the last repeating, and never capped by the call's timeout.
const SNAPSHOT_WAITS_MS: [u64; 5] = [0, 1000, 2000, 4000, 8000];

/// Neo's refusal of `locator.ariaSnapshot`, whose default mode is Playwright's action loop.
const LOCATOR_SNAPSHOT_UNSERVED: &str = "Neo does not serve locator.ariaSnapshot yet";

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

    /// The `ariaSnapshot` glue: the tree of the first match of `params.selector`, `None` while
    /// nothing matches.
    fn aria_snapshot(
        &self,
        params: &InjectedAriaSnapshotParams,
    ) -> impl Future<Output = Result<Option<AriaFrameSnapshot>, PageError>> + Send;
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

/// One `Frame.textContent`'s options, decoded once by the dispatcher's route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReadOptions {
    pub(crate) selector: String,
    /// `strict: true` from `locator.textContent`; absent (non-strict) from `page.textContent`
    /// and `frame.textContent`.
    pub(crate) strict: bool,
}

impl ReadOptions {
    pub(crate) fn from_params(params: FrameTextContentParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
        }
    }
}

/// One `Frame.ariaSnapshot`'s options, decoded once by the dispatcher's route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SnapshotOptions {
    /// `mode ?? 'default'`.
    pub(crate) mode: AriaSnapshotMode,
    /// Passed to the page as given: Playwright's renderer treats 0 as no limit.
    pub(crate) depth: Option<i64>,
    pub(crate) boxes: Option<bool>,
}

impl SnapshotOptions {
    /// Reads Playwright's params, refusing a selector before any work: that is
    /// `locator.ariaSnapshot`, which Neo does not serve. An empty selector is no selector, as
    /// Playwright's `options.selector &&` reads it.
    pub(crate) fn from_params(params: FrameAriaSnapshotParams) -> Result<Self, PlaywrightError> {
        let FrameAriaSnapshotParams {
            mode,
            selector,
            depth,
            boxes,
        } = params;
        if selector.is_some_and(|selector| !selector.is_empty()) {
            return Err(PlaywrightError::error(LOCATOR_SNAPSHOT_UNSERVED));
        }
        Ok(Self {
            mode: match mode {
                Some(FrameAriaSnapshotParamsMode::Ai) => AriaSnapshotMode::Ai,
                Some(FrameAriaSnapshotParamsMode::Default) | None => AriaSnapshotMode::Default,
            },
            depth,
            boxes,
        })
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

/// Serves `Frame.ariaSnapshot` without a selector: `{snapshot}`, the main frame's document as
/// Playwright's YAML.
pub(crate) async fn aria_snapshot<T: ReadTab>(
    tab: &T,
    options: &SnapshotOptions,
    progress: &Progress,
) -> Result<FrameAriaSnapshotResult, CallError> {
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
            Ok(Some(snapshot)) => {
                let snapshot = aria_yaml::render(&snapshot.nodes)
                    .map_err(|error| PlaywrightError::error(format!("neo-runtime: {error}")))?;
                return Ok(FrameAriaSnapshotResult { snapshot });
            }
            Ok(None) | Err(Stop::Again) => {}
            Err(Stop::End(error)) => return Err(error),
        }
    }
}

/// One snapshot attempt: `Ok(None)` while the document has no body yet.
async fn snapshot_once<T: ReadTab>(
    tab: &T,
    params: &InjectedAriaSnapshotParams,
    progress: &Progress,
) -> Result<Option<AriaFrameSnapshot>, Stop> {
    progress.race(tab.prepare()).await??;
    Ok(progress.race(tab.aria_snapshot(params)).await??)
}

// ---------------------------------------------------------------------------------------------
// The tab, for reads.
// ---------------------------------------------------------------------------------------------

/// The same steps click takes, so a read and a click prepare the tab and name a selector alike.
impl SelectorTab for Arc<PageSession> {
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

/// By value, with no object group: neither result holds a page object.
impl ReadTab for Arc<PageSession> {
    async fn text_content(
        &self,
        selector: &str,
        strict: bool,
    ) -> Result<Option<TextMatch>, PageError> {
        let read = self
            .world
            .call(
                None,
                &InjectedTextContentParams {
                    selector: selector.to_string(),
                    strict,
                },
            )
            .await?;
        Ok(read.matched)
    }

    async fn aria_snapshot(
        &self,
        params: &InjectedAriaSnapshotParams,
    ) -> Result<Option<AriaFrameSnapshot>, PageError> {
        Ok(self.world.call(None, params).await?.snapshot)
    }
}

#[cfg(test)]
mod tests {
    //! Both loops against [`FakeReads`] under paused tokio time, so every attempt count and
    //! wait is exact. Each page step takes 1 ms, a loopback CDP round trip, as in click's tests.

    use std::collections::VecDeque;
    use std::sync::Mutex;

    use neo_runtime_spec::injected::{AriaChild, AriaNode, AriaRole};
    use tokio::time;
    use tokio::time::Instant;

    use super::*;
    use crate::progress::ErrorName;

    const ROUND_TRIP: Duration = Duration::from_millis(1);
    const SELECTOR: &str = "internal:role=listitem";
    const LOCATOR: &str = "getByRole('listitem')";
    const PREVIEW: &str = "<li>Apples</li>";

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

    /// A scripted tab. Every page step records its name and the time it started, in ms.
    struct FakeReads {
        prepare: Step<Result<(), PageError>>,
        describe: Step<Result<SelectorFacts, PageError>>,
        text: Step<Result<Option<TextMatch>, PageError>>,
        snapshot: Step<Result<Option<AriaFrameSnapshot>, PageError>>,
        calls: Mutex<Vec<(String, u128)>>,
        snapshot_params: Mutex<Vec<InjectedAriaSnapshotParams>>,
        started: Instant,
    }

    impl FakeReads {
        /// A list item that every step finds at once.
        fn ready() -> Self {
            Self {
                prepare: Step::always(Duration::ZERO, Ok(())),
                describe: Step::always(
                    ROUND_TRIP,
                    Ok(SelectorFacts {
                        locator: LOCATOR.to_string(),
                        invalid: None,
                    }),
                ),
                text: Step::always(ROUND_TRIP, Ok(Some(apples()))),
                snapshot: Step::always(ROUND_TRIP, Ok(Some(tree()))),
                calls: Mutex::default(),
                snapshot_params: Mutex::default(),
                started: Instant::now(),
            }
        }

        fn record(&self, call: &str) {
            let at = self.started.elapsed().as_millis();
            self.calls
                .lock()
                .expect("calls")
                .push((call.to_string(), at));
        }

        fn calls(&self) -> Vec<String> {
            self.calls
                .lock()
                .expect("calls")
                .iter()
                .map(|(call, _)| call.clone())
                .collect()
        }

        /// When each call named `name` started, in ms since the tab was made.
        fn starts(&self, name: &str) -> Vec<u128> {
            self.calls
                .lock()
                .expect("calls")
                .iter()
                .filter(|(call, _)| call == name)
                .map(|(_, at)| *at)
                .collect()
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

        async fn aria_snapshot(
            &self,
            params: &InjectedAriaSnapshotParams,
        ) -> Result<Option<AriaFrameSnapshot>, PageError> {
            self.record("ariaSnapshot");
            self.snapshot_params
                .lock()
                .expect("params")
                .push(params.clone());
            self.snapshot.answer().await
        }
    }

    fn apples() -> TextMatch {
        TextMatch {
            value: "Apples".to_string(),
            preview: PREVIEW.to_string(),
        }
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

    fn read_options(strict: bool) -> ReadOptions {
        ReadOptions {
            selector: SELECTOR.to_string(),
            strict,
        }
    }

    fn snapshot_options() -> SnapshotOptions {
        SnapshotOptions {
            mode: AriaSnapshotMode::Ai,
            depth: None,
            boxes: None,
        }
    }

    async fn read(
        tab: &FakeReads,
        timeout_ms: f64,
    ) -> (Result<FrameTextContentResult, CallError>, Vec<String>) {
        let progress = Progress::for_test(timeout_ms);
        let result = text_content(tab, &read_options(true), &progress).await;
        (result, progress.folded_log())
    }

    async fn snapshot(
        tab: &FakeReads,
        timeout_ms: f64,
    ) -> (Result<FrameAriaSnapshotResult, CallError>, Vec<String>) {
        let progress = Progress::for_test(timeout_ms);
        let result = aria_snapshot(tab, &snapshot_options(), &progress).await;
        (result, progress.folded_log())
    }

    fn timeout(timeout_ms: f64) -> CallError {
        PlaywrightError::timeout(timeout_ms).into()
    }

    #[tokio::test(start_paused = true)]
    async fn a_match_is_read_at_once_and_logged_as_playwright_does() {
        let tab = FakeReads::ready();
        let (result, log) = read(&tab, 30_000.0).await;

        assert_eq!(
            result,
            Ok(FrameTextContentResult {
                value: Some("Apples".to_string())
            })
        );
        assert_eq!(
            log,
            [
                format!("  - waiting for {LOCATOR}"),
                format!("    - locator resolved to {PREVIEW}"),
            ]
        );
        assert_eq!(tab.calls(), ["describe", "textContent"]);
    }

    /// The `hidden` click golden's arithmetic: with a 500 ms timeout the backoff drops 500 and
    /// repeats 100, so 7 reads fit (0, 20, 70, 170, 270, 370, 470 ms plus their round trips).
    #[tokio::test(start_paused = true)]
    async fn nothing_matching_polls_with_loop_a_backoff_and_times_out_naming_the_locator() {
        let mut tab = FakeReads::ready();
        tab.text = Step::always(ROUND_TRIP, Ok(None));
        let (result, log) = read(&tab, 500.0).await;

        assert_eq!(result, Err(timeout(500.0)));
        assert_eq!(log, [format!("  - waiting for {LOCATOR}")]);
        assert_eq!(
            tab.starts("textContent"),
            [1, 22, 73, 174, 275, 376, 477],
            "the read waits 0, 20, 50, then 100 ms after each round trip"
        );
        assert_eq!(tab.count_of("describe"), 1, "the locator is named once");
    }

    #[tokio::test(start_paused = true)]
    async fn an_element_appended_later_is_read_once_it_exists() {
        let mut tab = FakeReads::ready();
        tab.text = Step::new(
            ROUND_TRIP,
            [Ok(None), Ok(None), Ok(None), Ok(Some(apples()))],
        );
        let (result, log) = read(&tab, 30_000.0).await;

        assert_eq!(
            result.map(|result| result.value),
            Ok(Some("Apples".to_string()))
        );
        assert_eq!(tab.starts("textContent"), [1, 22, 73, 174]);
        assert_eq!(
            log,
            [
                format!("  - waiting for {LOCATOR}"),
                format!("    - locator resolved to {PREVIEW}"),
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_strict_mode_violation_fails_at_once_with_the_pages_message() {
        let message = "Error: strict mode violation: getByRole('listitem') resolved to 2 elements:\n    1) <li>Apples</li> aka getByText('Apples')\n    2) <li>Pears</li> aka getByText('Pears')\n";
        let mut tab = FakeReads::ready();
        tab.text = Step::always(ROUND_TRIP, Err(PageError::Threw(message.to_string())));
        let started = Instant::now();
        let (result, log) = read(&tab, 30_000.0).await;

        assert_eq!(result, Err(PlaywrightError::error(message).into()));
        assert_eq!(log, [format!("  - waiting for {LOCATOR}")]);
        assert_eq!(started.elapsed(), ROUND_TRIP * 2, "describe and one read");
    }

    #[tokio::test(start_paused = true)]
    async fn an_invalid_selector_fails_at_once_naming_it() {
        let mut tab = FakeReads::ready();
        tab.describe = Step::always(
            ROUND_TRIP,
            Ok(SelectorFacts {
                locator: "li >>".to_string(),
                invalid: Some("Unexpected token \">>\" while parsing css selector".to_string()),
            }),
        );
        let (result, log) = read(&tab, 30_000.0).await;

        assert_eq!(
            result,
            Err(
                PlaywrightError::error("Unexpected token \">>\" while parsing css selector").into()
            )
        );
        assert_eq!(log, ["  - waiting for li >>"]);
        assert_eq!(tab.calls(), ["describe"], "nothing is read");
    }

    #[tokio::test(start_paused = true)]
    async fn a_document_that_goes_mid_read_is_read_again_after_the_backoff() {
        let mut tab = FakeReads::ready();
        tab.text = Step::new(
            ROUND_TRIP,
            [Err(PageError::NotConnected), Ok(Some(apples()))],
        );
        let (result, _) = read(&tab, 30_000.0).await;

        assert_eq!(
            result.map(|result| result.value),
            Ok(Some("Apples".to_string()))
        );
        assert_eq!(tab.starts("textContent"), [1, 22]);

        let mut tab = FakeReads::ready();
        tab.prepare = Step::new(Duration::ZERO, [Err(PageError::NotConnected), Ok(())]);
        let (result, _) = read(&tab, 30_000.0).await;
        assert!(
            result.is_ok(),
            "a failed preparation retries too: {result:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_closed_tab_or_a_broken_contract_ends_the_read() {
        let mut tab = FakeReads::ready();
        tab.text = Step::always(ROUND_TRIP, Err(PageError::Closed));
        let (result, _) = read(&tab, 30_000.0).await;
        let Err(CallError::Playwright(error)) = result else {
            panic!("expected a Playwright error, got {result:?}");
        };
        assert_eq!(error.name, ErrorName::TargetClosedError);

        let mut tab = FakeReads::ready();
        tab.text = Step::always(
            ROUND_TRIP,
            Err(PageError::Contract("textContent answered 5".to_string())),
        );
        let (result, _) = read(&tab, 30_000.0).await;
        assert_eq!(
            result,
            Err(PlaywrightError::error("textContent answered 5").into())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_page_that_never_answers_keeps_the_raw_selector_in_the_log() {
        let mut tab = FakeReads::ready();
        tab.describe = Step::always(ROUND_TRIP, Err(PageError::NotConnected));
        let (result, log) = read(&tab, 500.0).await;

        assert_eq!(result, Err(timeout(500.0)));
        assert_eq!(log, [format!("  - waiting for {SELECTOR}")]);
        assert_eq!(tab.count_of("textContent"), 0);
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
        let (result, _) = read(&tab, 30_000.0).await;
        assert_eq!(
            result,
            Ok(FrameTextContentResult {
                value: Some(String::new())
            })
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_snapshot_renders_the_pages_tree_as_yaml_with_no_call_log() {
        let tab = FakeReads::ready();
        let (result, log) = snapshot(&tab, 30_000.0).await;

        assert_eq!(
            result,
            Ok(FrameAriaSnapshotResult {
                snapshot: "- list [ref=e1]:\n  - listitem [ref=e2]: Apples".to_string()
            })
        );
        assert!(log.is_empty(), "{log:?}");
        assert_eq!(
            tab.snapshot_params.lock().expect("params").as_slice(),
            [InjectedAriaSnapshotParams {
                selector: "body,frameset".to_string(),
                strict: false,
                mode: AriaSnapshotMode::Ai,
                depth: None,
                boxes: None,
            }]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_document_without_a_body_is_snapshotted_once_it_has_one() {
        let mut tab = FakeReads::ready();
        tab.snapshot = Step::new(ROUND_TRIP, [Ok(None), Ok(None), Ok(Some(tree()))]);
        let (result, _) = snapshot(&tab, 30_000.0).await;

        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            tab.starts("ariaSnapshot"),
            [0, 1001, 3002],
            "waits of 1000 and 2000 ms after each round trip"
        );
    }

    /// Playwright never caps these waits by the timeout: a 1500 ms call reads twice (at 0 and
    /// 1001 ms), and its next try would have been 2000 ms later.
    #[tokio::test(start_paused = true)]
    async fn snapshot_waits_are_not_capped_and_the_last_repeats() {
        let mut tab = FakeReads::ready();
        tab.snapshot = Step::always(ROUND_TRIP, Ok(None));
        let (result, log) = snapshot(&tab, 1500.0).await;
        assert_eq!(result, Err(timeout(1500.0)));
        assert!(log.is_empty(), "no call log at all: {log:?}");
        assert_eq!(tab.starts("ariaSnapshot"), [0, 1001]);

        let mut tab = FakeReads::ready();
        tab.snapshot = Step::always(ROUND_TRIP, Ok(None));
        let (result, _) = snapshot(&tab, 40_000.0).await;
        assert_eq!(result, Err(timeout(40_000.0)));
        assert_eq!(
            tab.starts("ariaSnapshot"),
            [0, 1001, 3002, 7003, 15_004, 23_005, 31_006, 39_007]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_snapshot_retries_a_gone_document_and_ends_on_an_exception_or_close() {
        let mut tab = FakeReads::ready();
        tab.snapshot = Step::new(ROUND_TRIP, [Err(PageError::NotConnected), Ok(Some(tree()))]);
        let (result, _) = snapshot(&tab, 30_000.0).await;
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(tab.starts("ariaSnapshot"), [0, 1001]);

        let mut tab = FakeReads::ready();
        tab.snapshot = Step::always(
            ROUND_TRIP,
            Err(PageError::Threw(
                "Error: Can only capture aria snapshot of Element nodes.".to_string(),
            )),
        );
        let (result, _) = snapshot(&tab, 30_000.0).await;
        assert_eq!(
            result,
            Err(
                PlaywrightError::error("Error: Can only capture aria snapshot of Element nodes.")
                    .into()
            )
        );

        let mut tab = FakeReads::ready();
        tab.prepare = Step::always(Duration::ZERO, Err(PageError::Closed));
        let (result, _) = snapshot(&tab, 30_000.0).await;
        assert_eq!(result, Err(PlaywrightError::target_closed().into()));
    }

    #[test]
    fn snapshot_options_refuse_a_selector_and_default_the_mode() {
        let decode = |params: serde_json::Value| {
            SnapshotOptions::from_params(serde_json::from_value(params).expect("params"))
        };
        assert_eq!(
            decode(serde_json::json!({ "selector": "#list", "mode": "ai" })),
            Err(PlaywrightError::error(
                "Neo does not serve locator.ariaSnapshot yet"
            ))
        );
        assert_eq!(
            decode(serde_json::json!({ "selector": "", "depth": 2, "boxes": true })),
            Ok(SnapshotOptions {
                mode: AriaSnapshotMode::Default,
                depth: Some(2),
                boxes: Some(true),
            }),
            "an empty selector is none, as Playwright reads it"
        );
        assert_eq!(
            decode(serde_json::json!({ "mode": "ai" })).map(|options| options.mode),
            Ok(AriaSnapshotMode::Ai)
        );
    }

    impl FakeReads {
        fn count_of(&self, name: &str) -> usize {
            self.starts(name).len()
        }
    }
}
