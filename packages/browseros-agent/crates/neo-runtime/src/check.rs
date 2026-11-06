//! Check/uncheck verify checked state around click's per-element loop B. Already-matching
//! elements never take an input turn; changing state awaits the hit-target verdict without
//! awaiting navigation. The SDK's setChecked delegates to these same two operations.

use std::sync::Arc;

use neo_runtime_spec::injected::Point;
use neo_runtime_spec::serve::{
    FrameCheckParams, FrameCheckParamsScroll, FrameUncheckParams, FrameUncheckParamsScroll,
};

use crate::click::{
    self, ActionResult, ClickOptions, ElementAction, MouseButton, MouseClick, PageError, Stop,
    WaitAfter,
};
use crate::forms::{CheckedState, FormTab};
use crate::progress::{CallError, DeliveryCell, PlaywrightError, Progress};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CheckOptions {
    pub(crate) checked: bool,
    pub(crate) click: ClickOptions,
}

impl CheckOptions {
    pub(crate) fn from_check(params: FrameCheckParams) -> Self {
        Self {
            checked: true,
            click: ClickOptions {
                selector: params.selector,
                strict: params.strict.unwrap_or(false),
                force: params.force.unwrap_or(false),
                scroll: params.scroll != Some(FrameCheckParamsScroll::None),
                wait_after: true,
                position: params.position.map(|point| Point {
                    x: point.x,
                    y: point.y,
                }),
                mouse: MouseClick {
                    button: MouseButton::Left,
                    click_count: 1,
                    delay: None,
                },
                trial: params.trial.unwrap_or(false),
            },
        }
    }

    pub(crate) fn from_uncheck(params: FrameUncheckParams) -> Self {
        Self {
            checked: false,
            click: ClickOptions {
                selector: params.selector,
                strict: params.strict.unwrap_or(false),
                force: params.force.unwrap_or(false),
                scroll: params.scroll != Some(FrameUncheckParamsScroll::None),
                wait_after: true,
                position: params.position.map(|point| Point {
                    x: point.x,
                    y: point.y,
                }),
                mouse: MouseClick {
                    button: MouseButton::Left,
                    click_count: 1,
                    delay: None,
                },
                trial: params.trial.unwrap_or(false),
            },
        }
    }
}

pub(crate) async fn set_checked<T: FormTab>(
    tab: &T,
    options: &CheckOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    let delivery = progress.delivery().ok_or_else(|| {
        PlaywrightError::error("neo-runtime: a checked-state action has no delivery cell")
    })?;
    click::retry_with_progress_if_not_connected(
        tab,
        &options.click.selector,
        options.click.strict,
        options.click.force,
        progress,
        &CheckAction { options, delivery },
    )
    .await
}

struct CheckAction<'a> {
    options: &'a CheckOptions,
    delivery: &'a Arc<DeliveryCell>,
}

impl<T: FormTab> ElementAction<T> for CheckAction<'_> {
    type Output = ();

    async fn act(
        &self,
        tab: &T,
        element: &T::Element,
        progress: &Progress,
    ) -> Result<ActionResult<()>, Stop> {
        let state = read_state(tab, element, progress).await?;
        if state.matches == self.options.checked {
            return Ok(ActionResult::Done(()));
        }
        if !self.options.checked && state.is_radio == Some(true) {
            return Err(PlaywrightError::error(
                "Cannot uncheck radio button. Radio buttons can only be unchecked by selecting another radio button in the same group.",
            ).into());
        }
        match click::retry_pointer_action(
            tab,
            element,
            &self.options.click,
            WaitAfter::Disabled,
            progress,
            self.delivery,
        )
        .await?
        {
            ActionResult::NotConnected => return Ok(ActionResult::NotConnected),
            ActionResult::Done(()) => {}
        }
        if self.options.click.trial {
            return Ok(ActionResult::Done(()));
        }
        if read_state(tab, element, progress).await?.matches != self.options.checked {
            return Err(
                PlaywrightError::error("Clicking the checkbox did not change its state").into(),
            );
        }
        Ok(ActionResult::Done(()))
    }
}

async fn read_state<T: FormTab>(
    tab: &T,
    element: &T::Element,
    progress: &Progress,
) -> Result<CheckedState, Stop> {
    let state = progress
        .race(tab.checked_state(element))
        .await?
        .map_err(|error| match error {
            // A detached state read is terminal, including the verification after a click;
            // re-resolving there could repeat input that already reached the page.
            PageError::NotConnected => not_attached(),
            other => other.into(),
        })?;
    if state.received.as_deref() == Some("error:notconnected") {
        return Err(not_attached());
    }
    Ok(state)
}

fn not_attached() -> Stop {
    PlaywrightError::error("Element is not attached to the DOM").into()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use neo_runtime_spec::injected::ElementState;

    use super::*;
    use crate::click::tests::{ArmAnswer, FakeTab, Step, Writes};
    use crate::click::{PageError, StatesCheck, Verdict};
    use crate::forms::CheckedState;
    use crate::progress::ErrorName;
    use crate::run::Delivery;

    const TRIP: Duration = Duration::from_millis(1);

    fn options(checked: bool) -> CheckOptions {
        if checked {
            CheckOptions::from_check(
                serde_json::from_value(serde_json::json!({"selector":"input", "strict":true}))
                    .expect("check params"),
            )
        } else {
            CheckOptions::from_uncheck(
                serde_json::from_value(serde_json::json!({"selector":"input", "strict":true}))
                    .expect("uncheck params"),
            )
        }
    }

    fn state(matches: bool, is_radio: bool) -> CheckedState {
        CheckedState {
            matches,
            received: Some(if matches { "checked" } else { "unchecked" }.into()),
            is_radio: Some(is_radio),
        }
    }

    fn error_category(result: &Result<(), CallError>, name: ErrorName, category: &str) {
        assert!(
            matches!(result, Err(CallError::Playwright(error)) if error.name == name && error.message.contains(category)),
            "{result:?}"
        );
    }

    fn presses(tab: &FakeTab) -> usize {
        tab.calls()
            .iter()
            .filter(|call| call.starts_with("press "))
            .count()
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_check_and_uncheck_click_once_and_verify_the_final_state() {
        for checked in [false, true] {
            let mut tab = FakeTab::ready();
            tab.forms.checked = Step::new(
                TRIP,
                [Ok(state(!checked, false)), Ok(state(checked, false))],
            );
            let progress = Progress::for_test(1000.0);
            assert_eq!(
                set_checked(&tab, &options(checked), &progress).await,
                Ok(())
            );
            assert_eq!(
                progress.delivery().expect("input").delivery(),
                Delivery::Done
            );
            assert_eq!(presses(&tab), 1, "one state-changing click");
            assert!(
                tab.count("verdict") > 0,
                "the hit-target verdict must be read"
            );
            assert_eq!(tab.count("settle"), 0, "check does not await navigation");
            assert_eq!(tab.count("watch_navigations"), 0);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_an_already_matching_checkbox_or_unchecked_radio_is_a_noop() {
        for (checked, radio) in [(false, false), (true, false), (false, true)] {
            let mut tab = FakeTab::ready();
            tab.forms.checked = Step::always(TRIP, Ok(state(checked, radio)));
            let progress = Progress::for_test(1000.0);
            assert_eq!(
                set_checked(&tab, &options(checked), &progress).await,
                Ok(())
            );
            assert_eq!(
                progress.delivery().expect("input").delivery(),
                Delivery::NotSent
            );
            assert_eq!(tab.count("input"), 0);
            assert_eq!(presses(&tab), 0);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_unchecking_a_checked_radio_fails_before_input() {
        let mut tab = FakeTab::ready();
        tab.forms.checked = Step::always(TRIP, Ok(state(true, true)));
        let progress = Progress::for_test(1000.0);
        error_category(
            &set_checked(&tab, &options(false), &progress).await,
            ErrorName::Error,
            "radio",
        );
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::NotSent
        );
        assert_eq!(presses(&tab), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_detached_and_wrong_element_reads_fail_before_input() {
        for reply in [
            Ok(CheckedState {
                matches: false,
                received: Some("error:notconnected".into()),
                is_radio: None,
            }),
            Err(PageError::NotConnected),
            Err(PageError::Threw("Not a checkbox or radio button".into())),
        ] {
            let category = if matches!(&reply, Err(PageError::Threw(_))) {
                "checkbox"
            } else {
                "attached"
            };
            let mut tab = FakeTab::ready();
            tab.forms.checked = Step::always(TRIP, reply);
            let progress = Progress::for_test(1000.0);
            error_category(
                &set_checked(&tab, &options(true), &progress).await,
                ErrorName::Error,
                category,
            );
            assert_eq!(
                progress.delivery().expect("input").delivery(),
                Delivery::NotSent
            );
            assert_eq!(tab.count("input"), 0);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_click_that_did_not_change_state_fails_without_another_click() {
        let tab = FakeTab::ready();
        let progress = Progress::for_test(1000.0);
        error_category(
            &set_checked(&tab, &options(true), &progress).await,
            ErrorName::Error,
            "did not change",
        );
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::Done
        );
        assert_eq!(presses(&tab), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_trial_reads_the_verdict_but_does_not_verify_or_deliver_input() {
        let mut tab = FakeTab::ready();
        tab.forms.checked = Step::new(
            TRIP,
            [
                Ok(state(false, false)),
                Err(PageError::Threw("trial must not reread".into())),
            ],
        );
        let mut options = options(true);
        options.click.trial = true;
        let progress = Progress::for_test(1000.0);
        assert_eq!(set_checked(&tab, &options, &progress).await, Ok(()));
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::NotSent
        );
        assert!(tab.count("verdict") > 0);
        assert_eq!(tab.count("settle"), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_pointer_detachment_refinds_the_element_before_clicking() {
        let mut tab = FakeTab::ready();
        tab.forms.checked = Step::new(
            TRIP,
            [
                Ok(state(false, false)),
                Ok(state(false, false)),
                Ok(state(true, false)),
            ],
        );
        tab.arm = Step::new(TRIP, [Ok(ArmAnswer::NotConnected), Ok(ArmAnswer::Armed)]);
        let progress = Progress::for_test(1000.0);
        assert_eq!(set_checked(&tab, &options(true), &progress).await, Ok(()));
        assert!(tab.count("resolve") > 1);
        assert_eq!(presses(&tab), 1);
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::Done
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_verified_block_can_retry_without_waiting_for_navigation() {
        let mut tab = FakeTab::ready();
        tab.forms.checked = Step::new(TRIP, [Ok(state(false, false)), Ok(state(true, false))]);
        tab.verdict = Step::new(TRIP, [Verdict::Blocked("overlay".into()), Verdict::Done]);
        let progress = Progress::for_test(1000.0);
        assert_eq!(set_checked(&tab, &options(true), &progress).await, Ok(()));
        assert!(presses(&tab) > 1);
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::Done
        );
        assert_eq!(tab.count("settle"), 0);
        assert_eq!(tab.count("input"), tab.count("input released"));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_disabled_checkbox_waits_then_times_out_without_input() {
        let mut tab = FakeTab::ready();
        tab.check_states = Step::always(TRIP, Ok(StatesCheck::Missing(ElementState::Enabled)));
        let progress = Progress::for_test(500.0);
        error_category(
            &set_checked(&tab, &options(true), &progress).await,
            ErrorName::TimeoutError,
            "",
        );
        assert!(tab.count("check_states") > 1);
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::NotSent
        );
        assert_eq!(presses(&tab), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_force_reuses_the_forced_click_path_and_still_verifies_state() {
        let mut tab = FakeTab::ready();
        tab.forms.checked = Step::new(TRIP, [Ok(state(false, false)), Ok(state(true, false))]);
        tab.check_states = Step::always(
            TRIP,
            Err(PageError::Threw("force must skip state checks".into())),
        );
        let mut options = options(true);
        options.click.force = true;
        let progress = Progress::for_test(1000.0);
        assert_eq!(set_checked(&tab, &options, &progress).await, Ok(()));
        assert_eq!(tab.count("check_states"), 0);
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::Done
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_uncertain_mouse_writes_fail_without_retrying_or_verifying_state() {
        let mut tab = FakeTab::ready();
        tab.press = Step::always(TRIP, Writes::Fail);
        let progress = Progress::for_test(1000.0);
        error_category(
            &set_checked(&tab, &options(true), &progress).await,
            ErrorName::Error,
            "",
        );
        assert_eq!(presses(&tab), 1);
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::Unknown
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_failed_final_read_never_causes_another_click() {
        let mut tab = FakeTab::ready();
        tab.forms.checked = Step::new(
            TRIP,
            [Ok(state(false, false)), Err(PageError::NotConnected)],
        );
        let progress = Progress::for_test(1000.0);
        error_category(
            &set_checked(&tab, &options(true), &progress).await,
            ErrorName::Error,
            "attached",
        );
        assert_eq!(presses(&tab), 1);
        assert_eq!(
            progress.delivery().expect("input").delivery(),
            Delivery::Done
        );
    }
}
