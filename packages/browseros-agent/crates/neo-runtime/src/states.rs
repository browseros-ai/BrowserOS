//! `Frame.isVisible` reads once; `Frame.isChecked` waits for the selector's first match.
//!
//! ```text
//! is_visible → prepare → describe (NotConnected: wait for context and retry)
//!                → glue isVisible once → bool (no match / evaluation NotConnected: false)
//! is_checked → reads::wait_for_function_on_selector → glue isChecked → bool
//!                no match / NotConnected: poll; detached result: Error
//! ```
//!
//! Both reads use the main frame's utility world without an input turn or delivery ticket.

use std::future::Future;

use neo_runtime_spec::injected::{
    CheckedStateMatch, InjectedIsCheckedParams, InjectedIsVisibleParams,
};
use neo_runtime_spec::serve::{FrameIsCheckedParams, FrameIsVisibleParams};

use crate::click::{LoopAWaits, PageError, SelectorFacts, Stop};
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
    pub(crate) fn from_is_visible(params: FrameIsVisibleParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
        }
    }

    pub(crate) fn from_is_checked(params: FrameIsCheckedParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
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

    fn is_checked(
        &self,
        selector: &str,
        strict: bool,
    ) -> impl Future<Output = Result<Option<CheckedStateMatch>, PageError>> + Send;
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

pub(crate) async fn is_checked<T: StateTab>(
    tab: &T,
    options: &StateOptions,
    progress: &Progress,
) -> Result<bool, CallError> {
    let (checked, received) =
        reads::wait_for_function_on_selector(tab, &options.selector, progress, || async {
            Ok(tab
                .is_checked(&options.selector, options.strict)
                .await?
                .map(|matched| Matched {
                    value: (matched.checked, matched.received),
                    preview: matched.preview,
                }))
        })
        .await?;
    if received.as_deref() == Some("error:notconnected") {
        return Err(PlaywrightError::error("Element is not attached to the DOM").into());
    }
    Ok(checked)
}

/// By-value reads: neither result retains a page object or needs an object group.
impl StateTab for FrameTab {
    async fn is_visible(&self, selector: &str, strict: bool) -> Result<bool, PageError> {
        Ok(self
            .world()
            .call(
                None,
                &InjectedIsVisibleParams {
                    selector: selector.to_string(),
                    strict,
                },
            )
            .await?
            .value)
    }

    async fn is_checked(
        &self,
        selector: &str,
        strict: bool,
    ) -> Result<Option<CheckedStateMatch>, PageError> {
        Ok(self
            .world()
            .call(
                None,
                &InjectedIsCheckedParams {
                    selector: selector.to_string(),
                    strict,
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
        checked: Step<Result<Option<CheckedStateMatch>, PageError>>,
        calls: Mutex<Vec<&'static str>>,
    }

    impl FakeStates {
        fn ready() -> Self {
            Self {
                prepare: Step::new([Ok(())]),
                describe: Step::new([Ok(facts())]),
                visible: Step::new([Ok(true)]),
                checked: Step::new([Ok(Some(matched(true)))]),
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

        async fn is_checked(
            &self,
            _selector: &str,
            _strict: bool,
        ) -> Result<Option<CheckedStateMatch>, PageError> {
            self.record("isChecked");
            self.checked.answer().await
        }
    }

    fn facts() -> SelectorFacts {
        SelectorFacts {
            locator: "locator('#control')".to_string(),
            invalid: None,
        }
    }

    fn matched(checked: bool) -> CheckedStateMatch {
        CheckedStateMatch {
            checked,
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
        is_checked(tab, &options(), &Progress::for_test(500.0)).await
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
        tab.checked = Step::new([Ok(Some(CheckedStateMatch {
            received: Some("error:notconnected".to_string()),
            ..matched(false)
        }))]);
        assert_eq!(
            checked(&tab).await,
            Err(PlaywrightError::error("Element is not attached to the DOM").into())
        );
    }
}
