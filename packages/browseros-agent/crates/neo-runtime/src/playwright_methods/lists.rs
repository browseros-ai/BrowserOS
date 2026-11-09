//! `Locator.count`: a non-strict, one-shot collection read.
//!
//! Frame.queryCount → prepare → describe → `query_count` → {value: count}.
//! `NotConnected` returns zero; invalid selectors and closed targets remain errors.
//! Text collections use the main-world evaluation route in `evaluate.rs`.

use std::future::Future;

use neo_runtime_spec::injected::InjectedQueryCountParams;
use neo_runtime_spec::serve::FrameQueryCountResult;

use crate::browser_representations::page::FrameTab;
use crate::chromium_bridge::frame_selector;
use crate::playwright_methods::click::PageError;
use crate::playwright_methods::reads::SelectorTab;
use crate::progress::{CallError, PlaywrightError, Progress};

/// Collection page reads; futures are cancel-safe, losing only their answer when dropped.
pub(crate) trait ListTab: SelectorTab {
    fn query_count(&self, selector: &str) -> impl Future<Output = Result<i64, PageError>> + Send;
}

/// `frames.ts` `queryCount` catches retriable page errors and returns zero, without retrying.
pub(crate) async fn query_count<T: ListTab>(
    tab: &T,
    selector: &str,
    progress: &Progress,
) -> Result<FrameQueryCountResult, CallError> {
    if let Err(error) = progress.race(tab.prepare()).await? {
        return count_result(Err(error));
    }
    let facts = match progress.race(tab.describe(selector)).await? {
        Ok(facts) => facts,
        Err(error) => return count_result(Err(error)),
    };
    if let Some(invalid) = facts.invalid {
        return Err(PlaywrightError::error(invalid).into());
    }
    count_result(progress.race(tab.query_count(selector)).await?)
}

/// Apply `queryCount`'s retriable-error fallback to any of its page steps.
fn count_result(result: Result<i64, PageError>) -> Result<FrameQueryCountResult, CallError> {
    let value = match result {
        Ok(value) => value,
        Err(PageError::NotConnected) => 0,
        Err(error) => return Err(page_error(error)),
    };
    Ok(FrameQueryCountResult { value })
}

fn page_error(error: PageError) -> CallError {
    match error {
        PageError::Closed => PlaywrightError::target_closed().into(),
        PageError::Threw(message) | PageError::Contract(message) => {
            PlaywrightError::error(message).into()
        }
        PageError::NotConnected => {
            PlaywrightError::error("neo-runtime: document changed during collection read").into()
        }
    }
}

/// By-value page calls, with no object group or page handles to retain.
impl ListTab for FrameTab {
    async fn query_count(&self, selector: &str) -> Result<i64, PageError> {
        let Some(located) = frame_selector::locate(self, selector, false).await? else {
            return Ok(0);
        };
        Ok(located
            .frame
            .call(
                None,
                &InjectedQueryCountParams {
                    selector: located.selector,
                },
            )
            .await?
            .value)
    }
}

#[cfg(test)]
mod tests {
    //! One-shot reads against a scripted tab under paused time. Assert replies and page steps,
    //! never call-log wording or timing.

    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::time::Duration;

    use serde_json::json;
    use tokio::time;

    use super::*;
    use crate::playwright_methods::click::SelectorFacts;
    use crate::progress::ErrorName;

    const SELECTOR: &str = ".item";

    enum Step {
        Prepare(Result<(), PageError>),
        Describe(Result<SelectorFacts, PageError>),
        Count(Result<i64, PageError>),
    }

    struct FakeLists {
        steps: Mutex<VecDeque<Step>>,
        calls: Mutex<Vec<&'static str>>,
    }

    impl FakeLists {
        fn new(steps: impl IntoIterator<Item = Step>) -> Self {
            Self {
                steps: Mutex::new(steps.into_iter().collect()),
                calls: Mutex::default(),
            }
        }

        async fn step(&self, name: &'static str) -> Step {
            self.calls.lock().expect("calls").push(name);
            time::sleep(Duration::from_millis(1)).await;
            self.steps
                .lock()
                .expect("steps")
                .pop_front()
                .expect("a scripted step")
        }

        fn assert_calls(&self, expected: &[&str]) {
            assert_eq!(*self.calls.lock().expect("calls"), expected);
            assert!(
                self.steps.lock().expect("steps").is_empty(),
                "every scripted step ran"
            );
        }
    }

    impl SelectorTab for FakeLists {
        async fn prepare(&self) -> Result<(), PageError> {
            let Step::Prepare(result) = self.step("prepare").await else {
                panic!("expected prepare");
            };
            result
        }

        async fn describe(&self, selector: &str) -> Result<SelectorFacts, PageError> {
            assert_eq!(selector, SELECTOR);
            let Step::Describe(result) = self.step("describe").await else {
                panic!("expected describe");
            };
            result
        }
    }

    fn valid_selector() -> Step {
        Step::Describe(Ok(SelectorFacts {
            locator: "locator('.item')".to_string(),
            invalid: None,
        }))
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_invalid_selectors_stop_after_describe_without_a_read() {
        let tab = FakeLists::new([
            Step::Prepare(Ok(())),
            Step::Describe(Ok(SelectorFacts {
                locator: SELECTOR.to_string(),
                invalid: Some("invalid selector".to_string()),
            })),
        ]);
        let error = query_count(&tab, SELECTOR, &Progress::for_test(0.0))
            .await
            .expect_err("invalid count selector");
        assert_error(error, ErrorName::Error);
        tab.assert_calls(&["prepare", "describe"]);
    }

    impl ListTab for FakeLists {
        async fn query_count(&self, selector: &str) -> Result<i64, PageError> {
            assert_eq!(selector, SELECTOR);
            let Step::Count(result) = self.step("queryCount").await else {
                panic!("expected queryCount");
            };
            result
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_count_returns_zero_or_many_from_one_snapshot() {
        for value in [0, 3] {
            let tab = FakeLists::new([
                Step::Prepare(Ok(())),
                valid_selector(),
                Step::Count(Ok(value)),
            ]);
            let result = query_count(&tab, SELECTOR, &Progress::for_test(0.0))
                .await
                .expect("count");
            assert_eq!(
                serde_json::to_value(result).expect("wire result"),
                json!({"value": value})
            );
            tab.assert_calls(&["prepare", "describe", "queryCount"]);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_count_returns_zero_when_a_page_step_loses_the_document() {
        for steps in [
            vec![Step::Prepare(Err(PageError::NotConnected))],
            vec![
                Step::Prepare(Ok(())),
                Step::Describe(Err(PageError::NotConnected)),
            ],
            vec![
                Step::Prepare(Ok(())),
                valid_selector(),
                Step::Count(Err(PageError::NotConnected)),
            ],
        ] {
            let expected = &["prepare", "describe", "queryCount"][..steps.len()];
            let tab = FakeLists::new(steps);
            assert_eq!(
                query_count(&tab, SELECTOR, &Progress::for_test(0.0))
                    .await
                    .expect("zero")
                    .value,
                0
            );
            tab.assert_calls(expected);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_collection_reads_fail_at_once_for_closed_invalid_or_contract_errors() {
        for (error, name) in [
            (PageError::Closed, ErrorName::TargetClosedError),
            (
                PageError::Threw("invalid selector".to_string()),
                ErrorName::Error,
            ),
            (
                PageError::Contract("bad bundle".to_string()),
                ErrorName::Error,
            ),
        ] {
            for failing_step in 0..3 {
                let count_steps = match failing_step {
                    0 => vec![Step::Prepare(Err(error.clone()))],
                    1 => vec![Step::Prepare(Ok(())), Step::Describe(Err(error.clone()))],
                    _ => vec![
                        Step::Prepare(Ok(())),
                        valid_selector(),
                        Step::Count(Err(error.clone())),
                    ],
                };
                let tab = FakeLists::new(count_steps);
                assert_error(
                    query_count(&tab, SELECTOR, &Progress::for_test(0.0))
                        .await
                        .expect_err("count fails"),
                    name,
                );
                tab.assert_calls(&["prepare", "describe", "queryCount"][..=failing_step]);
            }
        }
    }

    fn assert_error(error: CallError, expected: ErrorName) {
        let CallError::Playwright(error) = error else {
            panic!("a Playwright error");
        };
        assert_eq!(error.name, expected);
    }
}
