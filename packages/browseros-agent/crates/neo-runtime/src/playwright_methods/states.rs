//! `Frame.isVisible` and `Frame.isHidden` read once; `Frame.isChecked`, `Frame.isEnabled`,
//! `Frame.isDisabled` and `Frame.isEditable` wait for the selector's first match.
//!
//! ```text
//! is_visible / is_hidden → prepare → describe (NotConnected: wait for context and retry)
//!                → glue isVisible once → bool (no match / evaluation NotConnected: false)
//!                is_hidden answers the negation, as Playwright's `isHidden` does
//! element_state(state) → reads::wait_for_function_on_selector
//!                → glue elementStateOnSelector {state} → bool
//!                no match / NotConnected: poll; page throw (wrong element kind, strict): Error;
//!                detached result: Error
//! ```
//!
//! Every read uses the selected frame's utility world without an input turn or delivery ticket.

use std::future::Future;

use neo_runtime_spec::injected::{
    ElementStateMatch, InjectedElementStateOnSelectorParams, InjectedIsVisibleParams,
    ReadableElementState,
};

use crate::click::{LoopAWaits, PageError, SelectorFacts, Stop};
use crate::frame_selector;
use crate::page::FrameTab;
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::reads::{self, Matched, SelectorTab};

/// A state read's selector and Playwright's strict matching option.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StateOptions {
    pub(crate) selector: String,
    pub(crate) strict: bool,
}

impl StateOptions {
    /// Every state command's params are `{selector, strict?}`; page and frame reads default to
    /// non-strict, and locator reads send `strict: true`.
    pub(crate) fn new(selector: String, strict: Option<bool>) -> Self {
        Self {
            selector,
            strict: strict.unwrap_or(false),
        }
    }
}

/// One page evaluation per state attempt, cancel-safe because neither read changes input.
pub(crate) trait StateTab: SelectorTab {
    fn is_visible(
        &self,
        selector: &str,
        strict: bool,
    ) -> impl Future<Output = Result<bool, PageError>> + Send;

    /// The `elementStateOnSelector` glue: the first match's state, `None` while nothing matches.
    fn element_state(
        &self,
        selector: &str,
        strict: bool,
        state: ReadableElementState,
    ) -> impl Future<Output = Result<Option<ElementStateMatch>, PageError>> + Send;
}

/// Playwright's `Frame.isVisibleInternal`: [`is_visible`]'s one read, with no call-log line and
/// no description first, for the locator-handler checkpoint. A context lost during the read
/// reads false; a page exception (a strict mode violation) or a closed tab ends the call.
///
/// Playwright: `server/frames.ts` `Frame.isVisibleInternal`: `if (this.isNonRetriableError(e))`
pub(crate) async fn is_visible_internal<T: StateTab>(
    tab: &T,
    options: &StateOptions,
    progress: &Progress,
) -> Result<bool, CallError> {
    let attempt = progress
        .race(tab.is_visible(&options.selector, options.strict))
        .await?
        .map_err(Stop::from);
    match attempt {
        Ok(value) => Ok(value),
        Err(Stop::Retry) => Ok(false),
        Err(Stop::End(error)) => Err(error),
    }
}

pub(crate) async fn is_visible<T: StateTab>(
    tab: &T,
    options: &StateOptions,
    progress: &Progress,
) -> Result<bool, CallError> {
    let facts = describe_when_ready(tab, &options.selector, progress).await;
    let locator = facts
        .as_ref()
        .map_or(options.selector.as_str(), |facts| facts.locator.as_str());
    progress.log(format!("  checking visibility of {locator}"));
    let facts = facts?;
    if let Some(message) = facts.invalid {
        return Err(PlaywrightError::error(message).into());
    }
    let attempt = progress
        .race(tab.is_visible(&options.selector, options.strict))
        .await?
        .map_err(Stop::from);
    match attempt {
        Ok(value) => Ok(value),
        // Only a context lost during this evaluation reads false, as in isVisibleInternal.
        Err(Stop::Retry) => Ok(false),
        Err(Stop::End(error)) => Err(error),
    }
}

/// Serves `Frame.isHidden`: one visibility read, negated.
///
/// Playwright: `server/frames.ts` `Frame.isHidden`: `return !(await this.isVisible(progress, target));`
pub(crate) async fn is_hidden<T: StateTab>(
    tab: &T,
    options: &StateOptions,
    progress: &Progress,
) -> Result<bool, CallError> {
    is_visible(tab, options, progress)
        .await
        .map(|visible| !visible)
}

/// Playwright waits for a frame context before evaluating. Neo also needs that context to parse
/// the selector, so a transient preparation failure must not hide an invalid selector as false.
async fn describe_when_ready<T: SelectorTab>(
    tab: &T,
    selector: &str,
    progress: &Progress,
) -> Result<SelectorFacts, CallError> {
    let mut waits = LoopAWaits::new(progress.timeout_ms());
    loop {
        let wait = waits.next_wait();
        if !wait.is_zero() {
            progress.wait(wait).await?;
        }
        let attempt: Result<SelectorFacts, Stop> = async {
            progress.race(tab.prepare()).await??;
            Ok(progress.race(tab.describe(selector)).await??)
        }
        .await;
        match attempt {
            Ok(facts) => return Ok(facts),
            Err(Stop::Retry) => {}
            Err(Stop::End(error)) => return Err(error),
        }
    }
}

/// Serves `Frame.isChecked`, `Frame.isEnabled`, `Frame.isDisabled` and `Frame.isEditable`:
/// waits for the first match, then reads `state` on it once. A page throw (an element that
/// cannot have the state, a strict violation) ends the call at once, as Playwright's evaluation
/// errors do; an element detached mid-read ends it with Playwright's detached error.
///
/// Playwright: `server/frames.ts` `Frame._elementState`: `return { result: injected.elementState(element, data.state) };`
pub(crate) async fn element_state<T: StateTab>(
    tab: &T,
    options: &StateOptions,
    state: ReadableElementState,
    progress: &Progress,
) -> Result<bool, CallError> {
    let (matches, received) =
        reads::wait_for_function_on_selector(tab, &options.selector, progress, || async {
            Ok(tab
                .element_state(&options.selector, options.strict, state)
                .await?
                .map(|matched| Matched {
                    value: (matched.matches, matched.received),
                    preview: matched.preview,
                }))
        })
        .await?;
    // Playwright: `server/frames.ts` `Frame._elementState`: `if (result.received === 'error:notconnected')`
    if received.as_deref() == Some(NOT_CONNECTED) {
        return Err(PlaywrightError::error(ELEMENT_NOT_ATTACHED).into());
    }
    Ok(matches)
}

/// What `InjectedScript.elementState` reports for an element no longer in its document.
///
/// Playwright: `server/frames.ts` `Frame._elementState`: `'error:notconnected'`
const NOT_CONNECTED: &str = "error:notconnected";

/// Playwright: `server/dom.ts` `throwElementIsNotAttached`: `'Element is not attached to the DOM'`
const ELEMENT_NOT_ATTACHED: &str = "Element is not attached to the DOM";

/// By-value reads: neither result retains a page object or needs an object group.
impl StateTab for FrameTab {
    async fn is_visible(&self, selector: &str, strict: bool) -> Result<bool, PageError> {
        let Some(located) = frame_selector::locate(self, selector, strict).await? else {
            return Ok(false);
        };
        Ok(located
            .frame
            .call(
                None,
                &InjectedIsVisibleParams {
                    selector: located.selector,
                    strict,
                    frame_visible: located.frame_visible,
                },
            )
            .await?
            .value)
    }

    async fn element_state(
        &self,
        selector: &str,
        strict: bool,
        state: ReadableElementState,
    ) -> Result<Option<ElementStateMatch>, PageError> {
        let Some(located) = frame_selector::locate(self, selector, strict).await? else {
            return Ok(None);
        };
        Ok(located
            .frame
            .call(
                None,
                &InjectedElementStateOnSelectorParams {
                    selector: located.selector,
                    strict,
                    state,
                },
            )
            .await?
            .matched)
    }
}

#[cfg(test)]
mod tests {
    //! Scripted page answers under paused time, asserting values and public error categories.

    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::time::Duration;

    use tokio::time;

    use super::*;
    use crate::progress::PlaywrightError;
    use crate::run::RunEnd;

    /// The last answer repeats; a second, successful answer exposes an unwanted retry.
    struct Step<T>(Mutex<VecDeque<T>>);

    impl<T: Clone> Step<T> {
        fn new(answers: impl IntoIterator<Item = T>) -> Self {
            Self(Mutex::new(answers.into_iter().collect()))
        }

        async fn answer(&self) -> T {
            let answer = {
                let mut answers = self.0.lock().expect("scripted answers");
                if answers.len() > 1 {
                    answers.pop_front().expect("nonempty answers")
                } else {
                    answers.front().cloned().expect("a scripted answer")
                }
            };
            time::sleep(Duration::from_millis(1)).await;
            answer
        }
    }

    struct FakeStates {
        prepare: Step<Result<(), PageError>>,
        describe: Step<Result<SelectorFacts, PageError>>,
        visible: Step<Result<bool, PageError>>,
        checked: Step<Result<Option<ElementStateMatch>, PageError>>,
        /// The state each `element_state` page call asked for.
        states: Mutex<Vec<ReadableElementState>>,
        calls: Mutex<Vec<&'static str>>,
    }

    impl FakeStates {
        fn ready() -> Self {
            Self {
                prepare: Step::new([Ok(())]),
                describe: Step::new([Ok(facts())]),
                visible: Step::new([Ok(true)]),
                checked: Step::new([Ok(Some(matched(true)))]),
                states: Mutex::default(),
                calls: Mutex::default(),
            }
        }

        fn record(&self, name: &'static str) {
            self.calls.lock().expect("page calls").push(name);
        }

        fn calls(&self) -> Vec<&'static str> {
            self.calls.lock().expect("page calls").clone()
        }
    }

    impl SelectorTab for FakeStates {
        async fn prepare(&self) -> Result<(), PageError> {
            self.prepare.answer().await
        }

        async fn describe(&self, _selector: &str) -> Result<SelectorFacts, PageError> {
            self.describe.answer().await
        }
    }

    impl StateTab for FakeStates {
        async fn is_visible(&self, _selector: &str, _strict: bool) -> Result<bool, PageError> {
            self.record("isVisible");
            self.visible.answer().await
        }

        async fn element_state(
            &self,
            _selector: &str,
            _strict: bool,
            state: ReadableElementState,
        ) -> Result<Option<ElementStateMatch>, PageError> {
            self.record("elementState");
            self.states.lock().expect("states").push(state);
            self.checked.answer().await
        }
    }

    fn facts() -> SelectorFacts {
        SelectorFacts {
            locator: "locator('#control')".to_string(),
            invalid: None,
        }
    }

    fn matched(checked: bool) -> ElementStateMatch {
        ElementStateMatch {
            matches: checked,
            received: Some(if checked { "checked" } else { "unchecked" }.to_string()),
            preview: "<input id=\"control\" type=\"checkbox\"/>".to_string(),
        }
    }

    fn options() -> StateOptions {
        StateOptions {
            selector: "#control".to_string(),
            strict: true,
        }
    }

    async fn visible(tab: &FakeStates) -> Result<bool, CallError> {
        is_visible(tab, &options(), &Progress::for_test(0.0)).await
    }

    async fn checked(tab: &FakeStates) -> Result<bool, CallError> {
        element_state(
            tab,
            &options(),
            ReadableElementState::Checked,
            &Progress::for_test(500.0),
        )
        .await
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_visible_and_hidden_elements_return_booleans() {
        for value in [true, false] {
            let mut tab = FakeStates::ready();
            tab.visible = Step::new([Ok(value)]);
            assert_eq!(visible(&tab).await, Ok(value));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_missing_visibility_returns_false_without_polling() {
        let mut tab = FakeStates::ready();
        tab.visible = Step::new([Ok(false), Ok(true)]);
        assert_eq!(visible(&tab).await, Ok(false));
        assert_eq!(tab.calls(), ["isVisible"]);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_visibility_waits_for_preparation_and_description_to_recover() {
        let mut preparing = FakeStates::ready();
        preparing.prepare = Step::new([Err(PageError::NotConnected), Ok(())]);
        let mut describing = FakeStates::ready();
        describing.describe = Step::new([Err(PageError::NotConnected), Ok(facts())]);
        for tab in [preparing, describing] {
            assert_eq!(visible(&tab).await, Ok(true));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_visibility_returns_false_when_its_evaluation_disconnects() {
        let mut reading = FakeStates::ready();
        reading.visible = Step::new([Err(PageError::NotConnected), Ok(true)]);
        assert_eq!(visible(&reading).await, Ok(false));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_visibility_rejects_an_invalid_selector_after_description_recovers() {
        let mut tab = FakeStates::ready();
        tab.describe = Step::new([
            Err(PageError::NotConnected),
            Ok(SelectorFacts {
                invalid: Some("invalid selector".to_string()),
                ..facts()
            }),
        ]);
        assert_eq!(
            visible(&tab).await,
            Err(PlaywrightError::error("invalid selector").into())
        );
        assert!(tab.calls().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_visibility_readiness_is_bounded_by_the_run() {
        let mut preparing = FakeStates::ready();
        preparing.prepare = Step::new([Err(PageError::NotConnected)]);
        let mut describing = FakeStates::ready();
        describing.describe = Step::new([Err(PageError::NotConnected)]);
        for tab in [preparing, describing] {
            // The call has no timeout; paused time advances to the test run's deadline.
            assert_eq!(
                visible(&tab).await,
                Err(CallError::RunEnded(RunEnd::Timeout))
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_visibility_rejects_page_throws_and_contract_errors() {
        for error in [
            PageError::Threw("strict mode violation".to_string()),
            PageError::Contract("missing value".to_string()),
        ] {
            let mut tab = FakeStates::ready();
            let message = match &error {
                PageError::Threw(message) | PageError::Contract(message) => message.clone(),
                _ => unreachable!(),
            };
            tab.visible = Step::new([Err(error), Ok(true)]);
            assert_eq!(
                visible(&tab).await,
                Err(PlaywrightError::error(message).into())
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_invalid_selectors_fail_both_reads_before_the_state_page_call() {
        let mut tab = FakeStates::ready();
        tab.describe = Step::new([Ok(SelectorFacts {
            invalid: Some("invalid selector".to_string()),
            ..facts()
        })]);
        let expected = Err(PlaywrightError::error("invalid selector").into());
        assert_eq!(visible(&tab).await, expected);
        assert_eq!(checked(&tab).await, expected);
        assert!(tab.calls().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_closed_pages_end_both_reads_with_target_closed() {
        let mut tab = FakeStates::ready();
        tab.visible = Step::new([Err(PageError::Closed), Ok(true)]);
        tab.checked = Step::new([Err(PageError::Closed), Ok(Some(matched(true)))]);
        let expected = Err(PlaywrightError::target_closed().into());
        assert_eq!(visible(&tab).await, expected);
        assert_eq!(checked(&tab).await, expected);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_checked_and_unchecked_matches_return_booleans() {
        for value in [true, false] {
            let mut tab = FakeStates::ready();
            tab.checked = Step::new([Ok(Some(matched(value)))]);
            assert_eq!(checked(&tab).await, Ok(value));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_checked_waits_for_a_missing_match() {
        let mut tab = FakeStates::ready();
        tab.checked = Step::new([Ok(None), Ok(Some(matched(false)))]);
        assert_eq!(checked(&tab).await, Ok(false));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_checked_retries_a_disconnected_page() {
        let mut tab = FakeStates::ready();
        tab.checked = Step::new([Err(PageError::NotConnected), Ok(Some(matched(true)))]);
        assert_eq!(checked(&tab).await, Ok(true));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_missing_checked_match_times_out_at_the_call_deadline() {
        let mut tab = FakeStates::ready();
        tab.checked = Step::new([Ok(None)]);
        assert_eq!(
            checked(&tab).await,
            Err(PlaywrightError::timeout(500.0).into())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_checked_rejects_wrong_elements_strict_matches_and_contract_errors() {
        for error in [
            PageError::Threw("Not a checkbox or radio button".to_string()),
            PageError::Threw("strict mode violation".to_string()),
            PageError::Contract("missing checked state".to_string()),
        ] {
            let mut tab = FakeStates::ready();
            let message = match &error {
                PageError::Threw(message) | PageError::Contract(message) => message.clone(),
                _ => unreachable!(),
            };
            tab.checked = Step::new([Err(error), Ok(Some(matched(true)))]);
            assert_eq!(
                checked(&tab).await,
                Err(PlaywrightError::error(message).into())
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_detached_checked_result_is_an_error() {
        let mut tab = FakeStates::ready();
        tab.checked = Step::new([Ok(Some(ElementStateMatch {
            received: Some("error:notconnected".to_string()),
            ..matched(false)
        }))]);
        assert_eq!(
            checked(&tab).await,
            Err(PlaywrightError::error("Element is not attached to the DOM").into())
        );
    }
}
