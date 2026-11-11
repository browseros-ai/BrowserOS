//! Temporary screenshot rendering in every frame's isolated world. The guard owns the tab's
//! screenshot queue until styles, caret and animations have been restored, including when a
//! timeout or host cancellation drops the capture future.

use std::mem;
use std::sync::Arc;
use std::time::Duration;

use browseros_core::screenshot::ScreenshotCapture;
use futures_util::future;
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::OwnedMutexGuard;
use tokio::time;

use crate::op_scope::OpScope;
use crate::page::screenshot_masks::{MaskLocator, ScreenshotMasks};
use crate::page::{OpPage, frame_tree, world_call};
use crate::resolve::{self, HandleSite};
use crate::target::Target;
use crate::world::SealedWorld;
use crate::{ErrorCode, PlaywrightError};

/// How long preparation waits on one frame (its world, its script) before skipping it as
/// stalled, as upstream's non-stalling evaluation skips a frame that does not answer.
const FRAME_STALL: Duration = Duration::from_secs(1);

const PREPARE: &str = include_str!("../../assets/screenshot-prepare.js");
const RESTORE: &str = "function() { window.__pwCleanupScreenshot?.(); }";

/// One screenshot's utility-world mutations and exclusive place in the tab's capture queue.
/// Worlds are remembered before sending preparation, because cancellation can lose the CDP
/// reply after its script already changed the document.
#[must_use = "the guard holds the screenshot queue through rendering cleanup"]
pub(super) struct ScreenshotPreparation {
    worlds: Vec<SealedWorld>,
    queue: Option<OwnedMutexGuard<()>>,
    masks: ScreenshotMasks,
    capture: Option<ScreenshotCapture>,
}

impl ScreenshotPreparation {
    /// Waits for earlier screenshots to restore their temporary document state.
    pub(super) async fn acquire(page: &OpPage<'_>) -> Self {
        Self {
            worlds: Vec::new(),
            queue: Some(Arc::clone(page.state().screenshots()).lock_owned().await),
            masks: ScreenshotMasks::default(),
            capture: None,
        }
    }

    /// Hold core's shared queue before masks are painted; audit images must wait for removal.
    pub(super) fn hold_capture(&mut self, capture: ScreenshotCapture) {
        self.capture = Some(capture);
    }

    pub(super) fn capture(&self) -> Option<&ScreenshotCapture> {
        self.capture.as_ref()
    }

    pub(super) async fn mask(
        &mut self,
        page: &OpPage<'_>,
        scope: &OpScope<'_>,
        locators: &[MaskLocator],
        color: &str,
    ) -> Result<(), PlaywrightError> {
        self.masks.apply(page, scope, locators, color).await
    }

    /// Applies Playwright's utility-world algorithm independently in every frame. Detached or
    /// stalled children are skipped, as upstream's safe non-stalling evaluation skips them;
    /// font readiness is independent of the requested DOM mutations.
    pub(super) async fn prepare(&mut self, page: &OpPage<'_>, scope: &OpScope<'_>, options: Value) {
        let main = page.main_endpoint().frame;
        let mut ids = vec![main.clone()];
        if let Ok(Ok(tree)) = time::timeout(
            FRAME_STALL,
            frame_tree::read(page.browser(), page.core_page(), &page.root_session()),
        )
        .await
        {
            ids.extend(tree.ids().filter(|id| **id != main).cloned());
        }
        let worlds = future::join_all(ids.iter().map(|id| async move {
            time::timeout(FRAME_STALL, world_call::frame_world(page, scope, Some(id)))
                .await
                .ok()
                .and_then(Result::ok)
        }))
        .await;
        self.worlds.extend(worlds.into_iter().flatten());
        // Register every world before sending mutations: cancellation can lose a reply even
        // though its script already changed the document.
        future::join_all(self.worlds.iter().map(|world| async {
            let _prepared =
                time::timeout(FRAME_STALL, world.call_for_screenshot(PREPARE, &options)).await;
        }))
        .await;
    }

    /// Font loading must precede element stability checks. An element capture waits in its
    /// own frame, so unrelated pending fonts in the main frame cannot block it.
    pub(super) async fn wait_for_fonts(
        &self,
        page: &OpPage<'_>,
        scope: &OpScope<'_>,
        target: Option<&Target>,
    ) -> Result<(), PlaywrightError> {
        let world = match target {
            None => self
                .worlds
                .iter()
                .find(|world| *world.frame() == page.main_endpoint().frame)
                .cloned(),
            Some(Target::Element(element)) => Some(
                HandleSite::find(page, scope, element)
                    .await?
                    .ok_or_else(|| resolve::handle_detached(scope))?
                    .world()
                    .clone(),
            ),
            Some(Target::Locator(_)) => {
                return Err(scope.fail(ErrorCode::Internal, "screenshot target was not bound"));
            }
        };
        if let Some(world) = world {
            scope.log("waiting for fonts to load...".to_owned());
            // Upstream ignores a wait whose utility context disappears. A pending font promise
            // still waits for the screenshot's timeout, including with caret: 'initial'.
            let _fonts = world
                .call_for_screenshot(
                    "function() { return document.fonts.ready.then(() => {}); }",
                    &json!([]),
                )
                .await;
        }
        Ok(())
    }

    /// Restores before returning bytes, so the caller can immediately inspect animation and
    /// caret state. If this await is cancelled, Drop retries the idempotent cleanup.
    pub(super) async fn restore(mut self) {
        self.masks.restore().await;
        restore(&self.worlds).await;
        self.worlds.clear();
        if let Some(capture) = self.capture.take() {
            capture.finish().await;
        }
    }
}

impl Drop for ScreenshotPreparation {
    fn drop(&mut self) {
        if self.worlds.is_empty() && self.masks.is_empty() && self.capture.is_none() {
            return;
        }
        let Ok(runtime) = Handle::try_current() else {
            return;
        };
        let worlds = mem::take(&mut self.worlds);
        let queue = self.queue.take();
        let mut masks = mem::take(&mut self.masks);
        let capture = self.capture.take();
        drop(runtime.spawn(async move {
            // A second screenshot must not replace the first one's cleanup callback while
            // restoration is still pending on a renderer.
            let _queue = queue;
            masks.restore().await;
            restore(&worlds).await;
            if let Some(capture) = capture {
                capture.finish().await;
            }
        }));
    }
}

async fn restore(worlds: &[SealedWorld]) {
    // Each renderer gets its own bounded attempt. A dead first renderer must not prevent
    // healthy child frames from resuming animations or restoring their caret and styles.
    future::join_all(worlds.iter().map(|world| async {
        let _restored = time::timeout(
            Duration::from_secs(5),
            world.call_for_screenshot(RESTORE, &json!([])),
        )
        .await;
    }))
    .await;
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;
    use std::time::Duration;

    use browseros_core::testing::{TestConnection, cdp_pending};
    use browseros_core::{CdpConnection, ProtocolSession, SessionId};
    use tokio::sync::Mutex;
    use tokio::{task, time};

    use super::{ScreenshotMasks, ScreenshotPreparation};
    use crate::world::SealedWorld;

    #[tokio::test(start_paused = true)]
    async fn cancellation_restores_healthy_frames_while_a_renderer_is_stalled()
    -> Result<(), Box<dyn Error>> {
        let stalled = TestConnection::new([("Runtime.callFunctionOn", cdp_pending())]);
        let healthy = TestConnection::new([]);
        let world = |connection: &Arc<TestConnection>, name: &str| {
            SealedWorld::for_tests(
                ProtocolSession::for_session(
                    Arc::clone(connection) as Arc<dyn CdpConnection>,
                    SessionId::from(name),
                ),
                name,
                0,
                name,
            )
        };
        let queue = Arc::new(Mutex::new(()));
        let preparation = ScreenshotPreparation {
            worlds: vec![world(&stalled, "stalled"), world(&healthy, "healthy")],
            queue: Some(Arc::clone(&queue).lock_owned().await),
            masks: ScreenshotMasks::default(),
            capture: None,
        };
        drop(preparation);
        task::yield_now().await;
        assert!(
            healthy
                .calls()?
                .iter()
                .any(|call| call.method == "Runtime.callFunctionOn"),
            "a stalled parent renderer must not postpone cleanup in a healthy child"
        );
        assert!(
            queue.try_lock().is_err(),
            "the next screenshot waits for cleanup to finish"
        );
        time::advance(Duration::from_secs(6)).await;
        task::yield_now().await;
        assert!(
            queue.try_lock().is_ok(),
            "a dead renderer must not hold the queue forever"
        );
        Ok(())
    }

    /// Caret choice must not select a different font-readiness contract. The fake browser has
    /// no pixel encoder; reaching its font promise is the behavior under test here.
    #[tokio::test]
    async fn initial_caret_still_waits_for_fonts() -> Result<(), Box<dyn Error>> {
        use crate::Op;
        use crate::test_support::{ProbeAnswer, ScriptedRun, ScriptedTab, TabScript};
        use serde_json::{Value, json};
        use std::sync::atomic::{AtomicBool, Ordering};

        #[derive(Default)]
        struct Fonts(AtomicBool);
        impl TabScript for Fonts {
            fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
                ProbeAnswer::unexpected()
            }
            fn main_world(
                &self,
                _object: &str,
                function: &str,
            ) -> Option<Result<Value, &'static str>> {
                if function.contains("document.fonts.ready") {
                    self.0.store(true, Ordering::SeqCst);
                    Some(Ok(json!({})))
                } else {
                    None
                }
            }
        }
        let tab = ScriptedTab::new(Fonts::default());
        let run = ScriptedRun::over(&tab).await?;
        let op = Op::from_name("page.screenshot").ok_or("screenshot is registered")?;
        let _pixels = run
            .playwright
            .invoke(
                &run.env,
                op,
                json!({ "page": run.page, "caret": "initial" }),
            )
            .await;
        assert!(
            tab.script().0.load(Ordering::SeqCst),
            "caret initial must still await font readiness before capture"
        );
        Ok(())
    }
}
