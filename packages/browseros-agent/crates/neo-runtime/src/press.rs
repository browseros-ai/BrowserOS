//! `Frame.press`, in the structure of Playwright's server: `frames.ts` `press` (loop A with the
//! action pre-checks, and no loop B) around `dom.ts` `_press` (focus, then `Keyboard.press`,
//! inside `waitForSignalsCreatedBy` unless `noWaitAfter`).
//!
//! ```text
//!  press(tab, options, progress)               log: waiting for <locator>
//!   loop A (click.rs): pre-check, describe, resolve → element
//!   │ the browser's platform, read once per call (ControlOrMeta, the mac editing commands)
//!   │ log: elementHandle.press("<key>")
//!   │ wait for the tab's input turn → unless noWaitAfter: watch navigations
//!   │ focus (selection reset unless it already had focus)
//!   │   not connected → turn released; "element was detached from the DOM, retrying"
//!   │ plan the keys (keyboard.rs) │ an unknown token → Error, no key sent
//!   │ begin ticket → write_keys (detached writer) → acked: Done │ failed: the call ends, Unknown
//!   │ turn released → unless noWaitAfter: the barrier, `  navigated to "<url>"` per commit
//! ```
//!
//! The input turn runs from the focus to the last key's answer, so no other call's input, from
//! this run or another on the tab, lands between the focus and the keys or between two keys. It
//! is taken only after every wait for the element and released before the barrier, which only
//! waits: a press waiting for its element never holds up the input that would produce it.
//!
//! Neo-only behaviour, each deliberate: an unknown key sends nothing (Playwright presses the
//! tokens before it and leaves them held); a press that stops mid-way still releases every key
//! it pressed; a failed key write ends the call with delivery `Unknown`, never a retry.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use neo_runtime_spec::serve::FramePressParams;

use crate::click::{self, ActionResult, ElementAction, PageError, PressReport, Stop};
use crate::forms::{FocusOutcome, FormTab};
use crate::keyboard::{self, KeyboardTab, Platform};
use crate::progress::{CallError, DeliveryCell, PlaywrightError, Progress};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PressOptions {
    pub(crate) selector: String,
    pub(crate) strict: bool,
    pub(crate) key: String,
    pub(crate) delay: Option<f64>,
    pub(crate) wait_after: bool,
}

impl PressOptions {
    pub(crate) fn from_params(params: FramePressParams) -> Self {
        Self {
            selector: params.selector,
            strict: params.strict.unwrap_or(false),
            key: params.key,
            delay: params.delay,
            wait_after: !params.no_wait_after.unwrap_or(false),
        }
    }

    /// `if (options.delay)`: none or zero presses without sleeping; a negative delay sleeps 0,
    /// as `setTimeout` does.
    fn key_delay(&self) -> Option<Duration> {
        self.delay.filter(|delay| *delay != 0.0).map(|delay| {
            Duration::try_from_secs_f64(delay.max(0.0) / 1000.0).unwrap_or(Duration::MAX)
        })
    }
}

/// Serves `Frame.press`. `Ok(())` is Playwright's `{id}` reply; the call log and delivery are
/// in `progress`.
pub(crate) async fn press<T: FormTab + KeyboardTab>(
    tab: &T,
    options: &PressOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    // Press is an input method in the spec, so dispatch always gives it a cell.
    let delivery = progress
        .delivery()
        .ok_or_else(|| PlaywrightError::error("neo-runtime: Frame.press has no delivery cell"))?;
    let action = PressAction {
        options,
        delivery,
        platform: OnceLock::new(),
    };
    click::retry_with_progress_if_not_connected(
        tab,
        &options.selector,
        options.strict,
        false,
        progress,
        &action,
    )
    .await
}

struct PressAction<'a> {
    options: &'a PressOptions,
    delivery: &'a Arc<DeliveryCell>,
    /// Read by the first attempt that resolves the element, and kept for the call's later
    /// attempts.
    platform: OnceLock<Platform>,
}

impl<T: FormTab + KeyboardTab> ElementAction<T> for PressAction<'_> {
    type Output = ();

    async fn act(
        &self,
        tab: &T,
        element: &T::Element,
        progress: &Progress,
    ) -> Result<ActionResult<()>, Stop> {
        let platform = if let Some(platform) = self.platform.get() {
            *platform
        } else {
            let platform = progress.race(tab.platform()).await??;
            *self.platform.get_or_init(|| platform)
        };
        progress.log(format!("elementHandle.press(\"{}\")", self.options.key));
        let input = progress.race(tab.input()).await?;
        // Subscribed once the turn is this press's and before any of its input, as click does:
        // the barrier waits for every navigation this press's input requests, and never for one
        // that the input it waited behind requested.
        let watched = self.options.wait_after.then(|| tab.watch_navigations());
        match progress
            .race(tab.focus(element, true, input.clone()))
            .await??
        {
            FocusOutcome::Done => {}
            // Returning drops `input`: the turn is free while loop A resolves again.
            FocusOutcome::ErrorNotconnected => return Ok(ActionResult::NotConnected),
        }
        // The whole key is planned before any key goes down, and an unknown token fails where
        // Playwright's does, after the focus.
        let events = keyboard::plan_press(&self.options.key, platform)
            .map_err(|unknown| PlaywrightError::error(unknown.to_string()))?;
        // The last chance to stop before an irreversible step.
        progress.check()?;
        let Ok(ticket) = self.delivery.begin() else {
            return Err(PlaywrightError::error(
                "neo-runtime: a press tried to write keys again after input",
            )
            .into());
        };
        // A call that stops here drops the report's future: the writer cuts any delay short,
        // still releases every key it pressed, and marks the ticket itself.
        let report = progress
            .race(tab.write_keys(events, self.options.key_delay(), ticket, input.clone()))
            .await?;
        // The last key is answered; the tab's next input may start. The barrier only waits.
        drop(input);
        match report {
            PressReport::Acked(acked) => acked.delivered(),
            // Nothing was written because the session's socket was replaced, or the session
            // went mid-press: either way the tab is gone for this run.
            PressReport::NothingWritten | PressReport::Failed { closed: true, .. } => {
                return Err(PlaywrightError::target_closed().into());
            }
            PressReport::Failed {
                closed: false,
                message,
            } => return Err(PlaywrightError::error(message).into()),
        }
        if let Some(watched) = watched {
            match progress.race(tab.settle_after_input(watched)).await? {
                Ok(committed) => {
                    for url in committed {
                        progress.log(format!("  navigated to \"{url}\""));
                    }
                }
                Err(PageError::Closed) => return Err(PlaywrightError::target_closed().into()),
                // Playwright's epilogue swallows its errors; the keys were already pressed.
                Err(_) => {}
            }
        }
        Ok(ActionResult::Done(()))
    }
}

#[cfg(test)]
mod tests {
    use tokio::time;

    use super::*;
    use crate::click::tests::{FakeTab, Step, Writes};
    use crate::forms::tests::FormSteps;
    use crate::progress::ErrorName;
    use crate::run::Delivery;

    const TRIP: Duration = Duration::from_millis(1);

    fn options(key: &str, extra: &serde_json::Value) -> PressOptions {
        let mut params = serde_json::json!({ "selector": "#name", "strict": true, "key": key });
        if let (Some(params), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
            params.extend(extra.clone());
        }
        PressOptions::from_params(serde_json::from_value(params).expect("press params"))
    }

    /// Runs one press against `tab` with Playwright's `timeout`: its outcome and delivery.
    async fn run(
        tab: &FakeTab,
        options: &PressOptions,
        timeout_ms: f64,
    ) -> (Result<(), CallError>, Delivery) {
        let progress = Progress::for_test(timeout_ms);
        let result = press(tab, options, &progress).await;
        let delivery = progress.delivery().expect("an input call").delivery();
        (result, delivery)
    }

    fn error_name(result: &Result<(), CallError>) -> Option<ErrorName> {
        match result {
            Err(CallError::Playwright(error)) => Some(error.name),
            _ => None,
        }
    }

    fn keys(tab: &FakeTab) -> Vec<String> {
        tab.calls()
            .into_iter()
            .filter(|call| {
                call.starts_with("keyDown ")
                    || call.starts_with("rawKeyDown ")
                    || call.starts_with("keyUp ")
            })
            .collect()
    }

    fn focusing(focus: impl IntoIterator<Item = Result<FocusOutcome, PageError>>) -> FakeTab {
        FakeTab {
            forms: FormSteps {
                focus: Step::new(TRIP, focus),
                ..FormSteps::ready()
            },
            ..FakeTab::ready()
        }
    }

    /// The whole press happens inside one input turn: the turn is taken after the element is
    /// found, then the focus, then every key, and it is free before the navigation barrier.
    #[tokio::test(start_paused = true)]
    async fn a_press_focuses_and_writes_its_keys_inside_one_turn() {
        let tab = FakeTab::ready();
        let (result, delivery) =
            run(&tab, &options("Shift+A", &serde_json::json!({})), 1000.0).await;
        assert_eq!(result, Ok(()));
        assert_eq!(delivery, Delivery::Done);
        assert_eq!(
            tab.calls(),
            [
                "describe",
                "resolve",
                "platform",
                "input",
                "watch_navigations",
                "focus true",
                "rawKeyDown Shift",
                "keyDown A",
                "keyUp A",
                "keyUp Shift",
                "input released",
                "settle",
            ]
        );
    }

    /// A press that waits for its turn behind other input subscribes to navigations only once
    /// it holds the turn, so the barrier never counts the earlier input's navigations.
    #[tokio::test(start_paused = true)]
    async fn navigations_are_watched_only_once_the_press_holds_its_turn() {
        let tab = FakeTab {
            forms: FormSteps {
                input_wait: Duration::from_millis(50),
                ..FormSteps::ready()
            },
            ..FakeTab::ready()
        };
        let (result, delivery) = run(&tab, &options("a", &serde_json::json!({})), 1000.0).await;
        assert_eq!((result, delivery), (Ok(()), Delivery::Done));
        let calls = tab.calls();
        let at = |name: &str| {
            calls
                .iter()
                .position(|call| call == name)
                .unwrap_or_else(|| panic!("{name} in {calls:?}"))
        };
        assert!(
            at("input") < at("watch_navigations") && at("watch_navigations") < at("focus true"),
            "{calls:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn no_wait_after_skips_the_navigation_barrier() {
        let tab = FakeTab::ready();
        let no_wait = options("Enter", &serde_json::json!({ "noWaitAfter": true }));
        let (result, delivery) = run(&tab, &no_wait, 1000.0).await;
        assert_eq!((result, delivery), (Ok(()), Delivery::Done));
        assert_eq!(tab.count("watch_navigations"), 0);
        assert_eq!(tab.count("settle"), 0);
    }

    /// `ControlOrMeta` follows the platform the tab reports (mac in the fake).
    #[tokio::test(start_paused = true)]
    async fn control_or_meta_uses_the_browsers_platform() {
        let tab = FakeTab::ready();
        let (result, _) = run(
            &tab,
            &options("ControlOrMeta+a", &serde_json::json!({})),
            1000.0,
        )
        .await;
        assert_eq!(result, Ok(()));
        assert_eq!(
            keys(&tab),
            ["rawKeyDown Meta", "rawKeyDown a", "keyUp a", "keyUp Meta"]
        );
    }

    /// The error comes after the focus, as Playwright's does, but no key ever goes down.
    #[tokio::test(start_paused = true)]
    async fn an_unknown_key_ends_the_call_after_the_focus_without_a_key() {
        for key in ["Foo", "Shift+Foo", ""] {
            let tab = FakeTab::ready();
            let (result, delivery) = run(&tab, &options(key, &serde_json::json!({})), 1000.0).await;
            assert_eq!(error_name(&result), Some(ErrorName::Error), "{key:?}");
            assert!(
                matches!(&result, Err(CallError::Playwright(error)) if error.message.contains("Unknown key")),
                "{key:?}: {result:?}"
            );
            assert_eq!(delivery, Delivery::NotSent, "{key:?}");
            assert!(keys(&tab).is_empty(), "{key:?}: {:?}", tab.calls());
            assert_eq!(tab.count("focus true"), 1, "{key:?}");
            assert_eq!(tab.count("input released"), 1, "{key:?}: the turn is free");
        }
    }

    /// A focus that finds the element detached frees the turn and resolves again; the platform
    /// is read once for the call.
    #[tokio::test(start_paused = true)]
    async fn a_detached_element_is_resolved_again_before_any_key() {
        let tab = focusing([Ok(FocusOutcome::ErrorNotconnected), Ok(FocusOutcome::Done)]);
        let (result, delivery) = run(&tab, &options("a", &serde_json::json!({})), 1000.0).await;
        assert_eq!((result, delivery), (Ok(()), Delivery::Done));
        assert_eq!(tab.count("resolve"), 2);
        assert_eq!(tab.count("focus true"), 2);
        assert_eq!(tab.count("platform"), 1);
        assert_eq!(keys(&tab), ["keyDown a", "keyUp a"]);
        let calls = tab.calls();
        let released = calls
            .iter()
            .position(|call| call == "input released")
            .expect("the first turn is released");
        let second = calls
            .iter()
            .rposition(|call| call == "resolve")
            .expect("a second resolve");
        assert!(released < second, "{calls:?}");
    }

    /// A key write that may have reached the page ends the call: delivery Unknown, no second
    /// attempt, and the key it pressed is still released.
    #[tokio::test(start_paused = true)]
    async fn a_failed_key_write_ends_the_call_without_a_retry() {
        let tab = FakeTab {
            press: Step::always(TRIP, Writes::Fail),
            ..FakeTab::ready()
        };
        let (result, delivery) =
            run(&tab, &options("Shift+A", &serde_json::json!({})), 1000.0).await;
        assert_eq!(error_name(&result), Some(ErrorName::Error));
        assert_eq!(delivery, Delivery::Unknown);
        assert_eq!(tab.count("resolve"), 1, "never retried");
        assert_eq!(keys(&tab), ["rawKeyDown Shift", "keyUp Shift"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_key_write_reads_as_the_tab_closing() {
        let tab = FakeTab {
            press: Step::always(TRIP, Writes::Nothing),
            ..FakeTab::ready()
        };
        let (result, delivery) = run(&tab, &options("a", &serde_json::json!({})), 1000.0).await;
        assert_eq!(error_name(&result), Some(ErrorName::TargetClosedError));
        assert_eq!(delivery, Delivery::NotSent);
        assert_eq!(tab.count("resolve"), 1);
    }

    /// A deadline in the middle of `delay` ends the call with a timeout, and the writer still
    /// releases both keys; the record's delivery is the writer's truth once it is done.
    #[tokio::test(start_paused = true)]
    async fn a_deadline_mid_delay_still_releases_every_key() {
        let tab = FakeTab::ready();
        let progress = Progress::for_test(100.0);
        let slow = options("Shift+a", &serde_json::json!({ "delay": 500 }));
        let result = press(&tab, &slow, &progress).await;
        assert_eq!(error_name(&result), Some(ErrorName::TimeoutError));
        time::sleep(Duration::from_secs(1)).await;
        assert_eq!(
            keys(&tab),
            ["rawKeyDown Shift", "keyDown a", "keyUp a", "keyUp Shift"]
        );
        assert_eq!(
            progress.delivery().expect("an input call").delivery(),
            Delivery::Done,
            "every key the writer sent was answered"
        );
        assert_eq!(tab.count("input released"), 1);
    }

    /// An element that never appears times out without taking a turn or sending a key.
    #[tokio::test(start_paused = true)]
    async fn a_missing_element_times_out_without_input() {
        let tab = FakeTab {
            resolve: Step::always(TRIP, Ok(None)),
            ..FakeTab::ready()
        };
        let (result, delivery) = run(&tab, &options("a", &serde_json::json!({})), 500.0).await;
        assert_eq!(error_name(&result), Some(ErrorName::TimeoutError));
        assert_eq!(delivery, Delivery::NotSent);
        assert_eq!(tab.count("input"), 0);
        assert!(keys(&tab).is_empty());
    }

    /// Every wait for the element comes before the turn: a press whose turn is slow to come
    /// focuses only once it has it.
    #[tokio::test(start_paused = true)]
    async fn the_turn_is_taken_after_the_element_is_found() {
        let tab = FakeTab {
            forms: FormSteps {
                input_wait: Duration::from_millis(50),
                ..FormSteps::ready()
            },
            ..FakeTab::ready()
        };
        let (result, _) = run(&tab, &options("a", &serde_json::json!({})), 1000.0).await;
        assert_eq!(result, Ok(()));
        let calls = tab.calls();
        let at = |name: &str| calls.iter().position(|call| call == name);
        assert!(
            at("resolve") < at("input") && at("input") < at("focus true"),
            "{calls:?}"
        );
    }

    /// A press whose deadline passes while it waits for the tab's input turn gives up the wait
    /// and sends nothing.
    #[tokio::test(start_paused = true)]
    async fn a_press_that_times_out_waiting_for_its_turn_sends_nothing() {
        let tab = FakeTab {
            forms: FormSteps {
                input_wait: Duration::from_secs(5),
                ..FormSteps::ready()
            },
            ..FakeTab::ready()
        };
        let (result, delivery) = run(&tab, &options("a", &serde_json::json!({})), 500.0).await;
        assert_eq!(error_name(&result), Some(ErrorName::TimeoutError));
        assert_eq!(delivery, Delivery::NotSent);
        assert_eq!(tab.count("focus true"), 0);
        assert!(keys(&tab).is_empty());
    }

    #[test]
    fn a_truthy_delay_sleeps_and_a_negative_one_sleeps_zero() {
        let delay = |value: serde_json::Value| {
            options("a", &serde_json::json!({ "delay": value })).key_delay()
        };
        assert_eq!(delay(serde_json::json!(0)), None);
        assert_eq!(
            delay(serde_json::json!(50)),
            Some(Duration::from_millis(50))
        );
        assert_eq!(delay(serde_json::json!(-5)), Some(Duration::ZERO));
        assert_eq!(options("a", &serde_json::json!({})).key_delay(), None);
    }
}
