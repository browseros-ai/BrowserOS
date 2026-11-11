//! Temporary screenshot rendering in every frame's isolated world. The guard owns the tab's
//! screenshot queue until styles, caret and animations have been restored, including when a
//! timeout or host cancellation drops the capture future.

use std::mem;
use std::sync::Arc;
use std::time::Duration;

use browseros_core::{CoreError, ProtocolSession};
use futures_util::future;
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::OwnedMutexGuard;
use tokio::time;

use crate::op_scope::OpScope;
use crate::page::{FRAME_BUDGET, OpPage, frame_tree, world_call};
use crate::resolve::{self, HandleSite};
use crate::target::Target;
use crate::world::SealedWorld;
use crate::{ErrorCode, PlaywrightError};

const PREPARE: &str = include_str!("../../assets/screenshot-prepare.js");
const RESTORE: &str = "function() { window.__pwCleanupScreenshot?.(); }";

/// One screenshot's utility-world mutations and exclusive place in the tab's capture queue.
/// Worlds and background overrides are remembered before sending mutations, because cancellation
/// can lose a CDP reply after the renderer already changed.
#[must_use = "the guard holds the screenshot queue through rendering cleanup"]
pub(super) struct ScreenshotPreparation {
    worlds: Vec<SealedWorld>,
    background: Option<ProtocolSession>,
    queue: Option<OwnedMutexGuard<()>>,
}

impl ScreenshotPreparation {
    /// Waits for earlier screenshots to restore their temporary document state.
    pub(super) async fn acquire(page: &OpPage<'_>) -> Self {
        Self {
            worlds: Vec::new(),
            background: None,
            queue: Some(Arc::clone(page.state().screenshots()).lock_owned().await),
        }
    }

    /// Applies Playwright's utility-world algorithm independently in every frame. Detached or
    /// stalled children are skipped, as upstream's safe non-stalling evaluation skips them;
    /// font readiness is independent of the requested DOM mutations.
    pub(super) async fn prepare(&mut self, page: &OpPage<'_>, scope: &OpScope<'_>, options: Value) {
        let main = page.main_endpoint().frame;
        let mut ids = vec![main.clone()];
        if let Ok(Ok(tree)) = time::timeout(
            FRAME_BUDGET,
            frame_tree::read(page.browser(), page.core_page(), &page.root_session()),
        )
        .await
        {
            ids.extend(tree.ids().filter(|id| **id != main).cloned());
        }
        let worlds = future::join_all(ids.iter().map(|id| async move {
            time::timeout(FRAME_BUDGET, world_call::frame_world(page, scope, Some(id)))
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
                time::timeout(FRAME_BUDGET, world.call_for_screenshot(PREPARE, &options)).await;
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

    /// Change the admitted page's root background only for transparent encodings. Register
    /// before sending: a cancelled CDP reply may still have changed the renderer.
    pub(super) async fn omit_background(
        &mut self,
        session: ProtocolSession,
    ) -> Result<(), CoreError> {
        self.background = Some(session.clone());
        session
            .send::<_, Value>(
                "Emulation.setDefaultBackgroundColorOverride",
                json!({
                    "color": { "r": 0, "g": 0, "b": 0, "a": 0 },
                }),
            )
            .await?;
        Ok(())
    }

    /// Restores before returning bytes, so the caller can immediately inspect animation and
    /// caret state. If this await is cancelled, Drop retries the idempotent cleanup.
    pub(super) async fn restore(mut self) {
        restore(&self.worlds, self.background.as_ref()).await;
        self.worlds.clear();
        self.background = None;
    }
}

impl Drop for ScreenshotPreparation {
    fn drop(&mut self) {
        if self.worlds.is_empty() && self.background.is_none() {
            return;
        }
        let Ok(runtime) = Handle::try_current() else {
            return;
        };
        let worlds = mem::take(&mut self.worlds);
        let background = self.background.take();
        let queue = self.queue.take();
        drop(runtime.spawn(async move {
            // A later screenshot must not replace temporary rendering state while this
            // restoration is still pending on a renderer.
            let _queue = queue;
            restore(&worlds, background.as_ref()).await;
        }));
    }
}

async fn restore(worlds: &[SealedWorld], background: Option<&ProtocolSession>) {
    // A stalled frame must not postpone the root session's background reset.
    future::join(restore_frames(worlds), async {
        if let Some(session) = background {
            let _cleared = time::timeout(
                Duration::from_secs(5),
                session.send::<_, Value>("Emulation.setDefaultBackgroundColorOverride", json!({})),
            )
            .await;
        }
    })
    .await;
}

async fn restore_frames(worlds: &[SealedWorld]) {
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
    use serde_json::json;
    use tokio::sync::Mutex;
    use tokio::{task, time};

    use super::ScreenshotPreparation;
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
            background: None,
            queue: Some(Arc::clone(&queue).lock_owned().await),
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

    /// A lost acknowledgement does not mean the renderer kept its original background.
    #[tokio::test(start_paused = true)]
    async fn cancellation_resets_a_background_whose_set_reply_was_lost()
    -> Result<(), Box<dyn Error>> {
        let connection =
            TestConnection::new([("Emulation.setDefaultBackgroundColorOverride", cdp_pending())]);
        let session = ProtocolSession::for_session(
            Arc::clone(&connection) as Arc<dyn CdpConnection>,
            SessionId::from("background"),
        );
        let queue = Arc::new(Mutex::new(()));
        let mut preparation = ScreenshotPreparation {
            worlds: Vec::new(),
            background: None,
            queue: Some(Arc::clone(&queue).lock_owned().await),
        };
        assert!(
            time::timeout(
                Duration::from_millis(1),
                preparation.omit_background(session)
            )
            .await
            .is_err()
        );
        drop(preparation);
        for _ in 0..10 {
            task::yield_now().await;
        }
        let calls = connection.calls()?;
        assert_eq!(
            calls.first().ok_or("background was not set")?.params["color"]["a"],
            0
        );
        assert!(
            calls.iter().any(
                |call| call.method == "Emulation.setDefaultBackgroundColorOverride"
                    && call.params == json!({})
            ),
            "cancellation must clear an override even when its set reply was lost"
        );
        assert!(queue.try_lock().is_ok(), "the queue releases after reset");
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
                ProbeAnswer::empty_sweep()
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
