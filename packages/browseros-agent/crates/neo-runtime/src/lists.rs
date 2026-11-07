//! `Locator.count` and `Locator.allTextContents`: non-strict, one-shot collection reads.
//!
//! ```text
//! Frame.queryCount / Frame.evalOnSelectorAll (only the allTextContents function)
//!   dispatch → query_count / all_text_contents
//!     prepare → describe (invalid selector → Error) → one collection read
//!       → {value: count} / {value: SerializedValue}
//!     NotConnected → count 0 / Error; Closed → TargetClosedError
//!     invalid selector or contract error → Error; run end/cancel → stop
//! ```
//!
//! No target marks, waiting or call-log lines; `describe` validates without naming the locator.
//! The glue queries all matches in document order. Text comes from Neo's isolated utility world,
//! so page-defined `textContent` overrides (a `Node.prototype` patch, a custom element class or a
//! per-instance property) can differ from Playwright's main-world `$$eval`.
//! CDP request timeouts also become `NotConnected`, so `count` can return 0 on a frozen page.

use std::future::Future;

use neo_runtime_spec::injected::{InjectedAllTextContentsParams, InjectedQueryCountParams};
use neo_runtime_spec::serve::{
    FrameEvalOnSelectorAllResult, FrameQueryCountResult, SerializedValue,
};

use crate::click::PageError;
use crate::page::FrameTab;
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::reads::SelectorTab;

/// The source text Neo's SDK sends, from Playwright 26137face `client/locator.ts` `allTextContents`.
/// Compiled Playwright clients can spell it differently. Only this SDK expression is served;
/// arbitrary page JS must never bypass the typed glue boundary in `world.rs`.
pub(crate) const ALL_TEXT_CONTENTS_EXPRESSION: &str = "ee => ee.map(e => e.textContent || '')";

/// Collection page reads; futures are cancel-safe, losing only their answer when dropped.
pub(crate) trait ListTab: SelectorTab {
    fn query_count(&self, selector: &str) -> impl Future<Output = Result<i64, PageError>> + Send;

    fn all_text_contents(
        &self,
        selector: &str,
    ) -> impl Future<Output = Result<Vec<String>, PageError>> + Send;
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

/// `frames.ts` `evalOnSelectorAll` has no catch: a lost document is an error, not an empty list.
pub(crate) async fn all_text_contents<T: ListTab>(
    tab: &T,
    selector: &str,
    progress: &Progress,
) -> Result<FrameEvalOnSelectorAllResult, CallError> {
    progress.race(tab.prepare()).await?.map_err(page_error)?;
    let facts = progress
        .race(tab.describe(selector))
        .await?
        .map_err(page_error)?;
    if let Some(invalid) = facts.invalid {
        return Err(PlaywrightError::error(invalid).into());
    }
    let values = progress
        .race(tab.all_text_contents(selector))
        .await?
        .map_err(page_error)?;
    Ok(FrameEvalOnSelectorAllResult {
        value: serialize_strings(values),
    })
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

/// `packages/protocol/src/serializers.ts` `innerSerializeValue` at 26137face: the first array
/// gets id 1 and each string is `{s}`. All other `SerializedValue` alternatives remain absent.
pub(crate) fn serialize_strings(values: Vec<String>) -> SerializedValue {
    let empty = SerializedValue {
        n: None,
        b: None,
        s: None,
        v: None,
        d: None,
        u: None,
        bi: None,
        ta: None,
        e: None,
        r: None,
        a: None,
        o: None,
        h: None,
        r#fn: None,
        id: None,
        r#ref: None,
    };
    SerializedValue {
        a: Some(
            values
                .into_iter()
                .map(|s| SerializedValue {
                    s: Some(s),
                    ..empty.clone()
                })
                .collect(),
        ),
        id: Some(1),
        ..empty
    }
}

/// By-value page calls, with no object group or page handles to retain.
impl ListTab for FrameTab {
    async fn query_count(&self, selector: &str) -> Result<i64, PageError> {
        Ok(self
            .world
            .call(
                None,
                &InjectedQueryCountParams {
                    selector: selector.to_string(),
                },
            )
            .await?
            .value)
    }

    async fn all_text_contents(&self, selector: &str) -> Result<Vec<String>, PageError> {
        Ok(self
            .world
            .call(
                None,
                &InjectedAllTextContentsParams {
                    selector: selector.to_string(),
                },
            )
            .await?
            .values)
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
    use crate::click::SelectorFacts;
    use crate::progress::ErrorName;

    const SELECTOR: &str = ".item";

    enum Step {
        Prepare(Result<(), PageError>),
        Describe(Result<SelectorFacts, PageError>),
        Count(Result<i64, PageError>),
        Texts(Result<Vec<String>, PageError>),
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

    #[test]
    fn tests_that_the_sdk_bundle_sends_the_served_collection_expression() {
        let bundle = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../neo-sdk/dist/sdk.js"
        ));
        assert!(
            bundle.contains(ALL_TEXT_CONTENTS_EXPRESSION),
            "the embedded SDK expression must match the runtime's route gate"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_invalid_selectors_stop_after_describe_without_a_read() {
        for count in [true, false] {
            let tab = FakeLists::new([
                Step::Prepare(Ok(())),
                Step::Describe(Ok(SelectorFacts {
                    locator: SELECTOR.to_string(),
                    invalid: Some("invalid selector".to_string()),
                })),
            ]);
            let progress = Progress::for_test(0.0);
            let error = if count {
                query_count(&tab, SELECTOR, &progress)
                    .await
                    .expect_err("invalid count selector")
            } else {
                all_text_contents(&tab, SELECTOR, &progress)
                    .await
                    .expect_err("invalid text selector")
            };
            assert_error(error, ErrorName::Error);
            tab.assert_calls(&["prepare", "describe"]);
        }
    }

    impl ListTab for FakeLists {
        async fn query_count(&self, selector: &str) -> Result<i64, PageError> {
            assert_eq!(selector, SELECTOR);
            let Step::Count(result) = self.step("queryCount").await else {
                panic!("expected queryCount");
            };
            result
        }

        async fn all_text_contents(&self, selector: &str) -> Result<Vec<String>, PageError> {
            assert_eq!(selector, SELECTOR);
            let Step::Texts(result) = self.step("allTextContents").await else {
                panic!("expected allTextContents");
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
    async fn tests_that_all_text_contents_preserves_empty_and_ordered_strings() {
        for values in [
            vec![],
            vec![
                "visible".to_string(),
                "hidden".to_string(),
                String::new(),
                "nested 🦊".to_string(),
            ],
        ] {
            let tab = FakeLists::new([
                Step::Prepare(Ok(())),
                valid_selector(),
                Step::Texts(Ok(values.clone())),
            ]);
            let result = all_text_contents(&tab, SELECTOR, &Progress::for_test(0.0))
                .await
                .expect("texts");
            let expected: Vec<_> = values.iter().map(|s| json!({"s": s})).collect();
            assert_eq!(
                serde_json::to_value(result).expect("wire result"),
                json!({"value": {"a": expected, "id": 1}})
            );
            tab.assert_calls(&["prepare", "describe", "allTextContents"]);
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
    async fn tests_that_all_text_contents_fails_when_a_page_step_loses_the_document() {
        for steps in [
            vec![Step::Prepare(Err(PageError::NotConnected))],
            vec![
                Step::Prepare(Ok(())),
                Step::Describe(Err(PageError::NotConnected)),
            ],
            vec![
                Step::Prepare(Ok(())),
                valid_selector(),
                Step::Texts(Err(PageError::NotConnected)),
            ],
        ] {
            let expected = &["prepare", "describe", "allTextContents"][..steps.len()];
            let tab = FakeLists::new(steps);
            assert_error(
                all_text_contents(&tab, SELECTOR, &Progress::for_test(0.0))
                    .await
                    .expect_err("document lost"),
                ErrorName::Error,
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

                let text_steps = match failing_step {
                    0 => vec![Step::Prepare(Err(error.clone()))],
                    1 => vec![Step::Prepare(Ok(())), Step::Describe(Err(error.clone()))],
                    _ => vec![
                        Step::Prepare(Ok(())),
                        valid_selector(),
                        Step::Texts(Err(error.clone())),
                    ],
                };
                let tab = FakeLists::new(text_steps);
                assert_error(
                    all_text_contents(&tab, SELECTOR, &Progress::for_test(0.0))
                        .await
                        .expect_err("texts fail"),
                    name,
                );
                tab.assert_calls(&["prepare", "describe", "allTextContents"][..=failing_step]);
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
