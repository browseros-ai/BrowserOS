//! Playwright: `server/frames.ts` `Frame.dragAndDrop`
//! Neo: `Locator.dragTo` and `Page/Frame.dragAndDrop`, served as Playwright's server serves
//! `Frame.dragAndDrop`: two pointer actions (`click.rs`) in one progress, so one timeout bounds
//! both.
//!
//! ```text
//!  drag_and_drop(tab, options, progress)
//!   source: loop A + loop B, "move and down" (visible and stable; the `mouse` interceptor)
//!     a move to the source point, then a left press
//!   the cell: Done → Unknown until the drop is answered too
//!   target: loop A + loop B, "move and up" (visible and stable; the `drag` interceptor),
//!     without the action pre-checks: no navigation wait and no locator handler while the
//!     button is held (`PointerAction::skips_action_pre_checks`)
//!     a move in `steps` to the target point (each step meets Chromium's drag manager:
//!     the first one starts the drag the page allows, the rest are `dragOver`), then the
//!     release, a `drop` when a drag runs
//! ```
//!
//! A drag stopped between its halves leaves the left button held, as Playwright's does; the
//! run's end releases it (cancelling a drag that runs).
//!
//! The call's beforeAction hook (hooks feature 4) is asked once, about the source element: the
//! target half runs with the source's button held, and its selector is in the hook's `args`. A
//! hook's replacement arguments keep both selectors, and both halves run with them.

use std::sync::Arc;

use neo_runtime_spec::serve::{FrameDragAndDropParams, FrameDragAndDropParamsScroll, Point};

use crate::chromium_bridge::keyboard_input::KeyboardTab;
use crate::playwright_methods::click::{
    self, ActionResult, ElementAction, PointerAction, PointerOptions, Stop, Tab,
};
use crate::progress::{CallError, DeliveryCell, PlaywrightError, Progress};

/// Neo: dispatch gives every input method a delivery cell; a call without one is a broken route.
const NO_DELIVERY_CELL: &str = "neo-runtime: Frame.dragAndDrop has no delivery cell";

/// `Frame.dragAndDrop`'s options, as its two pointer actions.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DragOptions {
    pub(crate) source: PointerOptions,
    pub(crate) target: PointerOptions,
}

/// Neo: decoding `Frame.dragAndDrop`'s params into its two pointer actions.
impl DragOptions {
    /// Reads `Frame.dragAndDrop`'s params. `steps` above `click::MAX_STEPS` is refused before
    /// any work, as every pointer action refuses it.
    ///
    /// # Errors
    ///
    /// The refusal of `steps`.
    pub(crate) fn from_params(params: FrameDragAndDropParams) -> Result<Self, PlaywrightError> {
        let steps = click::steps_of(params.steps)?;
        let half = |selector: String, position: Option<&Point>, action| PointerOptions {
            selector,
            strict: params.strict.unwrap_or(false),
            force: params.force.unwrap_or(false),
            scroll: params.scroll != Some(FrameDragAndDropParamsScroll::None),
            wait_after: false,
            position: position.map(click::point),
            action,
            trial: params.trial.unwrap_or(false),
            modifiers: None,
            steps,
            platform: None,
        };
        Ok(Self {
            // Playwright: `server/frames.ts` `Frame.dragAndDrop`: `}, { ...options, position: options.sourcePosition }, frameVisible);`
            source: PointerOptions {
                // Its move is Playwright's default single step.
                steps: 1,
                ..half(
                    params.source,
                    params.source_position.as_ref(),
                    PointerAction::MoveAndDown,
                )
            },
            // Playwright: `server/frames.ts` `Frame.dragAndDrop`: `}, { ...options, position: options.targetPosition }, frameVisible);`
            target: half(
                params.target,
                params.target_position.as_ref(),
                PointerAction::MoveAndUp,
            ),
        })
    }
}

/// Playwright: `server/frames.ts` `Frame.dragAndDrop`: `dom.assertDone(await this._retryWithProgressIfNotConnected(progress, source,`
///
/// Serves `Frame.dragAndDrop`. `Ok(())` is Playwright's `{id}` reply; the call log and delivery
/// are in `progress`.
pub(crate) async fn drag_and_drop<T: KeyboardTab>(
    tab: &T,
    options: &DragOptions,
    progress: &Progress,
) -> Result<(), CallError> {
    // `dragAndDrop` is an input method in the spec, so dispatch always gives it a cell.
    let Some(delivery) = progress.delivery() else {
        return Err(PlaywrightError::error(NO_DELIVERY_CELL).into());
    };
    // Said before the source half, so its acknowledgement, even one that comes after the call
    // gave up on it, completes only that half: `Done` waits for the drop.
    delivery.expect_more();
    let source = &options.source;
    let replaced = click::retry_with_progress_if_not_connected(
        tab,
        &source.selector,
        source.strict,
        source.force,
        progress,
        &DragSource { options, delivery },
    )
    .await?;
    let target = replaced
        .as_ref()
        .map_or(&options.target, |replaced| &replaced.target);
    click::pointer_action(tab, target, progress).await
}

/// Neo: a drag's source half, which its call's beforeAction hook sees (hooks feature 4), with
/// the drag's options: the script's, or a hook's replacement, which then drives the target half
/// too.
struct DragSource<'a> {
    options: &'a DragOptions,
    delivery: &'a Arc<DeliveryCell>,
}

impl<T: Tab> ElementAction<T> for DragSource<'_> {
    /// The replacement the source half ran with, if a hook replaced the drag's arguments.
    type Output = Option<DragOptions>;
    type Options = DragOptions;

    async fn act(
        &self,
        tab: &T,
        element: &T::Element,
        replaced: Option<&DragOptions>,
        progress: &Progress,
    ) -> Result<ActionResult<Option<DragOptions>>, Stop> {
        let options = replaced.unwrap_or(self.options);
        Ok(
            match click::pointer_act(tab, element, &options.source, progress, self.delivery).await?
            {
                ActionResult::Done(()) => ActionResult::Done(replaced.cloned()),
                ActionResult::NotConnected => ActionResult::NotConnected,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::playwright_methods::click::tests::FakeTab;
    use crate::run::Delivery;
    use neo_runtime_spec::injected::Point;

    fn decode(params: &Value) -> Result<DragOptions, PlaywrightError> {
        DragOptions::from_params(serde_json::from_value(params.clone()).expect("params"))
    }

    #[test]
    fn tests_that_drag_and_drop_decodes_its_two_halves() {
        let options = decode(&json!({
            "source": "#a", "target": "#b", "strict": true, "steps": 4, "force": true,
            "trial": true, "scroll": "none",
            "sourcePosition": { "x": 1, "y": 2 }, "targetPosition": { "x": 3, "y": 4 },
        }))
        .expect("served");
        let (source, target) = (&options.source, &options.target);
        assert_eq!(source.action, PointerAction::MoveAndDown);
        assert_eq!(target.action, PointerAction::MoveAndUp);
        assert_eq!(
            (source.selector.as_str(), target.selector.as_str()),
            ("#a", "#b")
        );
        assert_eq!(source.steps, 1, "the press's move is one step");
        assert_eq!(target.steps, 4, "`steps` is the release's move");
        assert_eq!(source.position, Some(Point { x: 1.0, y: 2.0 }));
        assert_eq!(target.position, Some(Point { x: 3.0, y: 4.0 }));
        for half in [source, target] {
            assert!(half.strict && half.force && half.trial && !half.scroll && !half.wait_after);
            assert_eq!(half.modifiers, None);
        }
        assert_eq!(
            decode(&json!({ "source": "#a", "target": "#b", "steps": 1001 })),
            Err(PlaywrightError::error(
                "Neo does not support steps above 1000"
            ))
        );
    }

    /// The two halves run in one progress: the source's move and press, then the target's move
    /// and release, each a pointer action of its own; the call is `Done` only once both are.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_drag_presses_on_the_source_and_releases_on_the_target() {
        let tab = FakeTab::ready();
        let options =
            decode(&json!({ "source": "#a", "target": "#b", "steps": 2 })).expect("served");
        let progress = Progress::for_test(30_000.0);
        let result = drag_and_drop(&tab, &options, &progress).await;
        assert_eq!(result, Ok(()));
        assert_eq!(
            progress.delivery().expect("an input call").delivery(),
            Delivery::Done
        );
        let plans = tab.plans.lock().expect("plans").clone();
        let kinds: Vec<Vec<String>> = plans
            .iter()
            .map(|plan| {
                plan.commands
                    .iter()
                    .filter(|command| command.method.is_some())
                    .map(|command| {
                        let event: Value =
                            serde_json::from_str(&command.params).expect("params are JSON");
                        event["type"].as_str().unwrap_or_default().to_string()
                    })
                    .collect()
            })
            .collect();
        assert_eq!(
            kinds,
            [
                vec!["mouseMoved", "mousePressed"],
                vec!["mouseMoved", "mouseMoved", "mouseReleased"],
            ]
        );
        let log = progress.folded_log().join("\n");
        assert!(
            log.contains("move and down") && log.contains("move and up"),
            "{log}"
        );
        let mouse = tab.devices.lock().expect("devices").mouse.clone();
        assert!(mouse.buttons.is_empty(), "released on the target");
    }

    /// Playwright's target half skips `performActionPreChecks` in loops A and B: with the
    /// source's button held, no locator handler runs and no pending navigation is waited for,
    /// since either would move the mouse or wait in the middle of the drag. The source half runs
    /// both, in loop A and in loop B.
    #[tokio::test(start_paused = true)]
    async fn tests_that_only_the_source_half_runs_the_action_pre_checks() {
        let tab = FakeTab::ready();
        let options = decode(&json!({ "source": "#a", "target": "#b" })).expect("served");
        let progress = Progress::for_test(30_000.0);
        let delivery = progress.delivery().expect("an input call");
        delivery.expect_more();
        *tab.pending.lock().expect("pending") = Some("http://localhost/first".to_string());
        click::pointer_action(&tab, &options.source, &progress)
            .await
            .expect("the press");
        assert_eq!(
            tab.count("navigation_settled"),
            1,
            "the source waits out a pending navigation: {:?}",
            tab.calls()
        );
        assert_eq!(
            *tab.checkpoints.lock().expect("checkpoints"),
            [0, 0],
            "loop A's and loop B's checkpoints, both before the press"
        );

        *tab.pending.lock().expect("pending") = Some("http://localhost/second".to_string());
        click::pointer_action(&tab, &options.target, &progress)
            .await
            .expect("the drop");
        assert_eq!(
            *tab.checkpoints.lock().expect("checkpoints"),
            [0, 0],
            "no handler runs with the button held"
        );
        assert_eq!(
            tab.count("navigation_settled"),
            1,
            "the target waits for no navigation: {:?}",
            tab.calls()
        );
        assert_eq!(
            tab.plans.lock().expect("plans").len(),
            2,
            "pressed, then released"
        );
        assert_eq!(delivery.delivery(), Delivery::Done);
    }

    /// A drag whose target never appears stops after its press: the cell reads `Unknown`, the
    /// press went out and its release did not.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_drag_stopped_between_its_halves_is_unknown() {
        let tab = FakeTab::ready();
        let options = decode(&json!({ "source": "#a", "target": "#b" })).expect("served");
        let progress = Progress::for_test(30_000.0);
        let delivery = progress.delivery().expect("an input call");
        delivery.expect_more();
        click::pointer_action(&tab, &options.source, &progress)
            .await
            .expect("the press");
        assert_eq!(
            delivery.delivery(),
            Delivery::Unknown,
            "half a drag is not done"
        );
    }
}
