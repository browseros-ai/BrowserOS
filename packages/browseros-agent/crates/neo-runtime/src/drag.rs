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
//!   target: loop A + loop B, "move and up" (visible and stable; the `drag` interceptor)
//!     a move in `steps` to the target point (each step meets Chromium's drag manager:
//!     the first one starts the drag the page allows, the rest are `dragOver`), then the
//!     release, a `drop` when a drag runs
//! ```
//!
//! A drag stopped between its halves leaves the left button held, as Playwright's does; the
//! run's end releases it (cancelling a drag that runs).

use neo_runtime_spec::serve::{FrameDragAndDropParams, FrameDragAndDropParamsScroll, Point};

use crate::click::{self, PointerAction, PointerOptions};
use crate::keyboard::KeyboardTab;
use crate::progress::{CallError, PlaywrightError, Progress};

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
    click::pointer_action(tab, &options.source, progress).await?;
    if let Some(delivery) = progress.delivery() {
        delivery.expect_more();
    }
    click::pointer_action(tab, &options.target, progress).await
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::click::tests::FakeTab;
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

    /// A drag whose target never appears stops after its press: the cell reads `Unknown`, the
    /// press went out and its release did not.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_drag_stopped_between_its_halves_is_unknown() {
        let tab = FakeTab::ready();
        let options = decode(&json!({ "source": "#a", "target": "#b" })).expect("served");
        let progress = Progress::for_test(30_000.0);
        click::pointer_action(&tab, &options.source, &progress)
            .await
            .expect("the press");
        let delivery = progress.delivery().expect("an input call");
        assert_eq!(delivery.delivery(), Delivery::Done);
        delivery.expect_more();
        assert_eq!(
            delivery.delivery(),
            Delivery::Unknown,
            "half a drag is not done"
        );
    }
}
