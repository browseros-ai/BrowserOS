//! Neo: a page's emulation for one run: the viewport and media Playwright's server keeps per
//! page (`Page._emulatedSize`, `Page._emulatedMedia`), the options a context the run created
//! gives each of its pages, and `requestGC`. The state lives on the page's run session
//! ([`PageSession::emulation`]); every override goes out on that session, and media also on each
//! out-of-process frame session of the page the run attaches.
//!
//! ```text
//!  page.setViewportSize(s)   size = {viewport: s, screen: s} → Emulation.setDeviceMetricsOverride
//!  page.emulateMedia(o)      media ⊕= o → Emulation.setEmulatedMedia   main + child sessions
//!  context page attached     prepare(): the created context's options (browser_calls.rs)
//!  OOPIF session attached    child_attached(): the context's per-frame options and media
//! ```
//!
//! Lifetime: the run explicitly clears viewport and media overrides on the tabs it leaves
//! open, awaiting a bounded cleanup before its outcome (`Tabs::release_run_scoped`). A detach
//! also clears them, but runs in the background and cannot serve as that completion barrier.
//! A failed update restores the previous state and re-sends it in the background, as
//! Playwright does, so the page never keeps half of a rejected change.

use std::collections::HashSet;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use browseros_cdp::{CdpClient, CdpError, CdpEvent, SessionId};
use futures_util::future;
use neo_runtime_spec::serve::{
    PageEmulateMediaParams, PageEmulateMediaParamsColorScheme, PageEmulateMediaParamsContrast,
    PageEmulateMediaParamsForcedColors, PageEmulateMediaParamsMedia,
    PageEmulateMediaParamsReducedMotion,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::time;
use tokio_util::sync::{CancellationToken, DropGuard};
use tracing::debug;

use crate::dispatch::PAGE_END_WAIT;
use crate::frames::Host;
use crate::page::PageSession;
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::tabs;

/// Playwright: `server/chromium/crPage.ts` `FrameSession._updateViewport`: `insets = { width: 24, height: 88 };`
const HEADFUL_INSETS: (i64, i64) = (24, 88);
/// Playwright: `server/chromium/crPage.ts` `FrameSession._updateViewport`: `insets = { width: 16, height: 88 };`
const HEADFUL_WINDOWS_INSETS: (i64, i64) = (16, 88);
/// Playwright: `server/chromium/crPage.ts` `FrameSession._updateViewport`: `insets = { width: 8, height: 85 };`
const HEADFUL_LINUX_INSETS: (i64, i64) = (8, 85);
/// Playwright: `server/chromium/crPage.ts` `FrameSession._updateViewport`: `insets = { width: 2, height: 80 };`
const HEADFUL_MAC_INSETS: (i64, i64) = (2, 80);

/// Playwright: `server/page.ts` `Page.emulatedMedia`: `colorScheme: this._emulatedMedia.colorScheme !== undefined ? this._emulatedMedia.colorScheme : contextOptions.colorScheme ?? 'light',`
const DEFAULT_COLOR_SCHEME: &str = "light";
/// Playwright: `server/page.ts` `Page.emulatedMedia`: `reducedMotion: this._emulatedMedia.reducedMotion !== undefined ? this._emulatedMedia.reducedMotion : contextOptions.reducedMotion ?? 'no-preference',`
const DEFAULT_REDUCED_MOTION: &str = "no-preference";
/// Playwright: `server/page.ts` `Page.emulatedMedia`: `forcedColors: this._emulatedMedia.forcedColors !== undefined ? this._emulatedMedia.forcedColors : contextOptions.forcedColors ?? 'none',`
const DEFAULT_FORCED_COLORS: &str = "none";
/// Playwright: `server/page.ts` `Page.emulatedMedia`: `contrast: this._emulatedMedia.contrast !== undefined ? this._emulatedMedia.contrast : contextOptions.contrast ?? 'no-preference',`
const DEFAULT_CONTRAST: &str = "no-preference";
/// Playwright: `server/page.ts` `Page.emulatedMedia`: `media: this._emulatedMedia.media || 'no-override',`
const NO_OVERRIDE: &str = "no-override";

/// Deviation: `server/page.ts` `Page.emulatedMedia`: `colorScheme: this._emulatedMedia.colorScheme !== undefined ? this._emulatedMedia.colorScheme : contextOptions.colorScheme ?? 'light',`
/// Playwright's context, connected or created, emulates a light color scheme (and no reduced
/// motion, forced colors or contrast) on every page from its first moment. The run's own context
/// is the user's browser, which Neo leaves as the user set it: a feature the script never
/// emulated keeps the browser's own value, as in a Playwright context created with
/// `colorScheme: null` (and the others `null`).
const ATTACHED_CONTEXT_MEDIA: &str = NO_OVERRIDE;

/// Neo: observe a pending native resize without busy-polling the browser's layout engine.
const RESTORE_POLL: Duration = Duration::from_millis(10);

/// A viewport or screen size, Playwright's `types.Size`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Size {
    pub(crate) width: i64,
    pub(crate) height: i64,
}

/// The media features a page emulates, each Playwright's literal (`'dark'`, `'no-override'`, …),
/// or `None` where nothing set it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct EmulatedMedia {
    pub(crate) media: Option<&'static str>,
    pub(crate) color_scheme: Option<&'static str>,
    pub(crate) reduced_motion: Option<&'static str>,
    pub(crate) forced_colors: Option<&'static str>,
    pub(crate) contrast: Option<&'static str>,
}

/// The options of a context the run created (`browser.newContext`) that its pages emulate,
/// validated by `browser_calls.rs`. Playwright's `types.BrowserContextOptions`, cut to the
/// options Neo serves.
#[derive(Clone, Debug, Default, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Playwright's context options are independent booleans"
)]
pub(crate) struct ContextEmulation {
    /// `None` with `viewport: null` (`noDefaultViewport`).
    pub(crate) viewport: Option<Size>,
    pub(crate) screen: Option<Size>,
    pub(crate) device_scale_factor: Option<f64>,
    pub(crate) is_mobile: bool,
    pub(crate) has_touch: bool,
    pub(crate) user_agent: Option<String>,
    pub(crate) locale: Option<String>,
    pub(crate) timezone_id: Option<String>,
    pub(crate) media: EmulatedMedia,
    pub(crate) java_script_enabled: Option<bool>,
    pub(crate) bypass_csp: bool,
    pub(crate) ignore_https_errors: bool,
    /// Whether the browser shows windows (its user agent has no `Headless`), which sizes the
    /// window around a viewport.
    pub(crate) headful: bool,
}

/// A page's emulation state for the run. One per page session.
#[derive(Default)]
pub(crate) struct PageEmulation {
    state: Mutex<State>,
    /// Serializes this page's updates, so two calls never interleave their writes and the state
    /// sent last is the state kept.
    updates: AsyncMutex<()>,
    /// The sessions Chromium attached for a created context's out-of-process frames
    /// ([`watch_targets`]), which `emulateMedia` reaches too.
    frames: Mutex<Vec<SessionId>>,
    /// Stops [`watch_targets`] when the page's session, and this emulation with it, goes.
    watching: OnceLock<DropGuard>,
}

/// The paused auto-attach Playwright sets on every frame session and worker.
fn auto_attach() -> Value {
    // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `this._client.send('Target.setAutoAttach', { autoAttach: true, waitForDebuggerOnStart: true, flatten: true }),`
    json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttachedTarget {
    session_id: SessionId,
    target_info: AttachedTargetInfo,
}

#[derive(Deserialize)]
struct AttachedTargetInfo {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DetachedTarget {
    session_id: SessionId,
}

#[derive(Default)]
struct State {
    /// The created context's options, `None` for the run's own context.
    context: Option<Arc<ContextEmulation>>,
    /// Playwright's `Page._emulatedSize`: `(viewport, screen)`.
    size: Option<(Size, Size)>,
    /// Playwright's `Page._emulatedMedia`.
    media: EmulatedMedia,
    /// Playwright's `FrameSession._metricsOverride`: the last override sent.
    metrics: Option<Value>,
    /// Playwright's `FrameSession._windowId`: set only for a created context's page with a
    /// viewport, as Playwright reads it only without `noDefaultViewport`.
    window: Option<i64>,
    /// Written before sending, even when cancellation hides the command's acknowledgement.
    restore_viewport: bool,
    restore_media: bool,
    /// Native layout dimensions before this run first changed the viewport.
    original_viewport: Option<Value>,
    /// Retires setters and late frame attachment before cleanup waits for the update lock.
    released: bool,
}

impl PageEmulation {
    /// The page's viewport for its `Page` record: the size emulated now, else its context's.
    pub(crate) fn viewport(&self) -> Option<Size> {
        let state = self.state.lock().ok()?;
        state.emulated_size().map(|(viewport, _)| viewport)
    }

    /// Applies the options of the created context `context` to the page's main session, before
    /// any call on the page runs (`Tabs::session`): Playwright's `FrameSession._initialize` for
    /// the main frame.
    ///
    /// Deviation: `server/chromium/crPage.ts` `FrameSession._initialize`: `promises.push(this._setDefaultFontFamilies(this._client));`
    /// Neo leaves the headless browser's default font families, so text renders in BrowserOS's
    /// fonts, not the ones Playwright picks per platform.
    pub(crate) async fn prepare(
        self: &Arc<Self>,
        page: &PageSession,
        context: Arc<ContextEmulation>,
    ) -> Result<(), CdpError> {
        let _turn = self.updates.lock().await;
        if let Ok(mut state) = self.state.lock() {
            state.context = Some(Arc::clone(&context));
        }
        // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `const { windowId } = await this._client.send('Browser.getWindowForTarget');`
        if context.viewport.is_some()
            && let Ok(reply) = page.send("Browser.getWindowForTarget", json!({})).await
            && let Some(window) = reply.get("windowId").and_then(Value::as_i64)
            && let Ok(mut state) = self.state.lock()
        {
            state.window = Some(window);
        }
        let mut steps: Vec<future::BoxFuture<'_, Result<(), CdpError>>> = vec![
            // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `promises.push(this._client.send('Emulation.setFocusEmulationEnabled', { enabled: true }));`
            Box::pin(sent(page.send(
                "Emulation.setFocusEmulationEnabled",
                json!({ "enabled": true }),
            ))),
            Box::pin(self.update_viewport(page)),
            Box::pin(self.send_media(page, &[])),
        ];
        steps.extend(frame_options(&context, |method, params| {
            Box::pin(sent(page.send(method, params))) as future::BoxFuture<'_, _>
        }));
        future::try_join_all(steps).await?;
        self.attach_targets(page).await
    }

    /// Playwright's paused auto-attach on the main frame session of a created context's page:
    /// every out-of-process frame or worker the page starts waits for [`watch_targets`], which
    /// applies the context's options to it and only then lets it run.
    async fn attach_targets(self: &Arc<Self>, page: &PageSession) -> Result<(), CdpError> {
        let cdp = page.cdp().clone();
        let parent = page.session_id().clone();
        // Subscribed before auto-attach is on, so no target it pauses goes unseen and unreleased.
        let events = cdp.events();
        let stop = CancellationToken::new();
        if self.watching.set(stop.clone().drop_guard()).is_err() {
            return Ok(());
        }
        tokio::spawn(watch_targets(
            cdp.clone(),
            parent.clone(),
            events,
            Arc::downgrade(self),
            stop,
        ));
        cdp.send("Target.setAutoAttach", auto_attach(), Some(&parent))
            .await
            .map(|_| ())
    }

    /// Playwright's `FrameSession._onAttachedToTarget` for a target Chromium attached to the
    /// session `parent` and paused: an `iframe` gets the context's per-frame options and the
    /// page's media before it runs, a worker its user agent; any other target is let go.
    /// Returns whether its own session is watched for the targets it starts in turn.
    async fn configure_target(
        &self,
        cdp: &CdpClient,
        parent: &SessionId,
        child: &SessionId,
        kind: &str,
    ) -> bool {
        let send = |method: &'static str, params: Value| {
            Box::pin(sent(cdp.send(method, params, Some(child)))) as future::BoxFuture<'_, _>
        };
        let context = self
            .state
            .lock()
            .ok()
            .and_then(|state| state.context.clone());
        let mut steps: Vec<future::BoxFuture<'_, Result<(), CdpError>>> = Vec::new();
        match kind {
            "iframe" => {
                if let Some(context) = &context {
                    steps.extend(frame_options(context, send));
                }
                steps.push(send("Emulation.setEmulatedMedia", self.media_params()));
                steps.push(send("Target.setAutoAttach", auto_attach()));
                if let Ok(mut frames) = self.frames.lock() {
                    frames.push(child.clone());
                }
            }
            "worker" => {
                // Playwright: `server/chromium/crPage.ts` `FrameSession._initializeWorker`: `if (options.userAgent || options.locale)`
                if let Some(context) = context
                    .as_deref()
                    .filter(|context| context.user_agent.is_some() || context.locale.is_some())
                {
                    steps.push(send(
                        "Emulation.setUserAgentOverride",
                        user_agent_params(context),
                    ));
                }
                steps.push(send("Target.setAutoAttach", auto_attach()));
            }
            _ => {
                // Playwright: `server/chromium/crPage.ts` `FrameSession._onAttachedToTarget`: `parentSession._sendMayFail('Target.detachFromTarget', { sessionId: event.sessionId });`
                let detached = cdp
                    .send(
                        "Target.detachFromTarget",
                        json!({ "sessionId": child }),
                        Some(parent),
                    )
                    .await;
                if let Err(error) = detached {
                    debug!(%error, "letting go of an attached target failed");
                }
                return false;
            }
        }
        for result in future::join_all(steps).await {
            if let Err(error) = result {
                debug!(%error, "configuring an attached target failed");
            }
        }
        // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `promises.push(this._client.send('Runtime.runIfWaitingForDebugger'));`
        if let Err(error) = send("Runtime.runIfWaitingForDebugger", json!({})).await {
            debug!(%error, "releasing an attached target failed");
        }
        true
    }

    /// Forgets an out-of-process frame session Chromium detached.
    fn forget_frame(&self, session: &SessionId) {
        if let Ok(mut frames) = self.frames.lock() {
            frames.retain(|frame| frame != session);
        }
    }

    /// What an out-of-process frame's session gets once the run attaches it: its context's
    /// per-frame options and the page's media (Playwright's `FrameSession._initialize` for a
    /// child frame). A run's own context sends media only once the script emulated some.
    ///
    /// Deviation: `server/chromium/crPage.ts` `FrameSession._initialize`: `this._client.send('Target.setAutoAttach', { autoAttach: true, waitForDebuggerOnStart: true, flatten: true }),`
    /// Playwright attaches every out-of-process frame before it runs and emulates there from its
    /// first moment; so does Neo for a context the run created ([`watch_targets`]). The run's own
    /// context is the user's browser, whose frames Neo never holds paused: Neo attaches one when a
    /// call first reaches it, so a frame no call reached keeps the browser's own media.
    pub(crate) async fn child_attached(&self, host: &Host) {
        let _turn = self.updates.lock().await;
        let (context, media_set) = match self.state.lock() {
            Ok(state) if state.released => return,
            Ok(state) => (
                state.context.clone(),
                state.media != EmulatedMedia::default(),
            ),
            Err(_) => return,
        };
        let mut steps: Vec<future::BoxFuture<'_, Result<(), CdpError>>> = Vec::new();
        if let Some(context) = &context {
            steps.extend(frame_options(context, |method, params| {
                Box::pin(sent(host.send(method, params))) as future::BoxFuture<'_, _>
            }));
        }
        if context.is_some() || media_set {
            let params = self.media_params();
            steps.push(Box::pin(sent(
                host.send("Emulation.setEmulatedMedia", params),
            )));
        }
        for result in future::join_all(steps).await {
            if let Err(error) = result {
                debug!(%error, "emulating in an out-of-process frame failed");
            }
        }
    }

    /// Playwright's `Page.setViewportSize`: the page emulates `size` as both its viewport and
    /// its screen.
    pub(crate) async fn set_viewport_size(
        &self,
        page: &Arc<PageSession>,
        size: Size,
        progress: &Progress,
    ) -> Result<(), CallError> {
        let _turn = progress.race(self.updates.lock()).await?;
        self.ensure_live()?;
        // Playwright: `server/page.ts` `Page.setViewportSize`: `this._setEmulatedSize({ viewport: { ...viewportSize }, screen: { ...viewportSize } });`
        let old = self.swap(|state| state.size.replace((size, size)))?;
        let updated = progress.race(self.update_viewport(page)).await;
        if !matches!(updated, Ok(Ok(()))) {
            // Playwright: `server/page.ts` `Page.setViewportSize`: `this._emulatedSize = oldEmulatedSize;`
            self.swap(|state| {
                state.size = old;
            })?;
            Self::resync(page, Resync::Viewport);
        }
        finish(updated)
    }

    /// Playwright's `Page.emulateMedia`: each option given replaces what the page emulates for
    /// it; `'no-override'` (the client's `null`) stops emulating it.
    pub(crate) async fn emulate_media(
        &self,
        page: &Arc<PageSession>,
        params: &PageEmulateMediaParams,
        progress: &Progress,
    ) -> Result<(), CallError> {
        let _turn = progress.race(self.updates.lock()).await?;
        self.ensure_live()?;
        let old = self.swap(|state| {
            let old = state.media;
            // Playwright: `server/page.ts` `Page.emulateMedia`: `if (options.media !== undefined)`
            if let Some(media) = params.media {
                state.media.media = Some(media_literal(media));
            }
            if let Some(color_scheme) = params.color_scheme {
                state.media.color_scheme = Some(color_scheme_literal(color_scheme));
            }
            if let Some(reduced_motion) = params.reduced_motion {
                state.media.reduced_motion = Some(reduced_motion_literal(reduced_motion));
            }
            if let Some(forced_colors) = params.forced_colors {
                state.media.forced_colors = Some(forced_colors_literal(forced_colors));
            }
            if let Some(contrast) = params.contrast {
                state.media.contrast = Some(contrast_literal(contrast));
            }
            old
        })?;
        let children = page.frames.child_hosts();
        let updated = progress.race(self.send_media(page, &children)).await;
        if !matches!(updated, Ok(Ok(()))) {
            // Playwright: `server/page.ts` `Page.emulateMedia`: `this._emulatedMedia = oldEmulatedMedia;`
            self.swap(|state| {
                state.media = old;
            })?;
            Self::resync(page, Resync::Media);
        }
        finish(updated)
    }

    /// Runs `change` on the state under its lock.
    fn swap<T>(&self, change: impl FnOnce(&mut State) -> T) -> Result<T, CallError> {
        self.state
            .lock()
            .map(|mut state| change(&mut state))
            .map_err(|_| {
                PlaywrightError::error("neo-runtime: the page's emulation is unreadable").into()
            })
    }

    fn ensure_live(&self) -> Result<(), CallError> {
        if self.swap(|state| state.released)? {
            return Err(PlaywrightError::target_closed().into());
        }
        Ok(())
    }

    /// Neo: an ended run (or disconnected context) puts back the browser's own viewport and
    /// media at cleanup. The caller bounds its wait, but this tracked task keeps
    /// going; a slow reset is not discarded. No later setter, resync or frame attachment
    /// can reapply the retired state. Created contexts are disposed instead.
    pub(crate) async fn release(&self, page: &PageSession) {
        if let Ok(mut state) = self.state.lock() {
            state.released = true;
        }
        let _turn = self.updates.lock().await;
        let (viewport, media) = match self.state.lock() {
            Ok(mut state) => {
                state.size = None;
                state.media = EmulatedMedia::default();
                (state.restore_viewport, state.restore_media)
            }
            Err(_) => return,
        };
        if viewport {
            let previous = layout_size(page).await.ok();
            let restored = match page
                .send("Emulation.clearDeviceMetricsOverride", json!({}))
                .await
            {
                Ok(_) => self.viewport_restored(page, previous).await,
                Err(error) => Err(error),
            };
            match restored {
                Ok(()) | Err(CdpError::SessionGone) => {
                    if let Ok(mut state) = self.state.lock() {
                        state.restore_viewport = false;
                        state.metrics = None;
                    }
                }
                Err(error) => debug!(%error, "restoring the page's viewport failed"),
            }
        }
        if media {
            let children = page.frames.child_hosts();
            match self.send_media(page, &children).await {
                Ok(()) | Err(CdpError::SessionGone) => {
                    if let Ok(mut state) = self.state.lock() {
                        state.restore_media = false;
                    }
                }
                Err(error) => debug!(%error, "restoring the page's media failed"),
            }
        }
    }

    /// Neo: Chromium can acknowledge `clearDeviceMetricsOverride` before its native view's
    /// resize reaches the renderer (a held setter acknowledgement reproduces this). Observe
    /// layout until it is native again, or reflects a window resize since the run began.
    /// Media is reset afterward, so a scrollbar change cannot masquerade as that resize.
    async fn viewport_restored(
        &self,
        page: &PageSession,
        previous: Option<Value>,
    ) -> Result<(), CdpError> {
        let original = self
            .state
            .lock()
            .ok()
            .and_then(|state| state.original_viewport.clone());
        let deadline = time::Instant::now() + PAGE_END_WAIT;
        loop {
            let current = layout_size(page).await?;
            if original.as_ref() == Some(&current) || previous.as_ref() != Some(&current) {
                return Ok(());
            }
            if time::Instant::now() >= deadline {
                return Err(CdpError::Timeout {
                    method: "Page.getLayoutMetrics".to_string(),
                });
            }
            time::sleep(RESTORE_POLL).await;
        }
    }

    /// Playwright's background re-send after a failed update: `this.delegate.updateEmulateMedia().catch(() => {});`
    fn resync(page: &Arc<PageSession>, what: Resync) {
        let owner = Arc::clone(page);
        page.spawn(async move {
            let page = owner;
            let emulation = &page.emulation;
            let _turn = emulation.updates.lock().await;
            if emulation.ensure_live().is_err() {
                return;
            }
            let children = page.frames.child_hosts();
            let resent = match what {
                Resync::Viewport => emulation.update_viewport(&page).await,
                Resync::Media => emulation.send_media(&page, &children).await,
            };
            if let Err(error) = resent {
                debug!(%error, "re-sending the page's emulation failed");
            }
        });
    }

    /// Playwright's `FrameSession._updateViewport` with `preserveWindowBoundaries` unset: the
    /// device metrics override, after the window around it when the page has one, skipped when
    /// it is the override already sent.
    async fn update_viewport(&self, page: &PageSession) -> Result<(), CdpError> {
        let Some((metrics, window)) = self.state.lock().ok().and_then(|mut state| {
            let update = state.metrics_update()?;
            state.restore_viewport = true;
            Some(update)
        }) else {
            return Ok(());
        };
        let needs_original = self
            .state
            .lock()
            .is_ok_and(|state| state.context.is_none() && state.original_viewport.is_none());
        if needs_original {
            let original = layout_size(page).await?;
            if let Ok(mut state) = self.state.lock() {
                state.original_viewport = Some(original);
            }
        }
        if let Some((window, width, height)) = window {
            // Playwright: `server/chromium/crPage.ts` `FrameSession.setWindowBounds`: `return await this._client.send('Browser.setWindowBounds', {`
            page.send(
                "Browser.setWindowBounds",
                json!({ "windowId": window, "bounds": { "width": width, "height": height } }),
            )
            .await?;
        }
        page.send("Emulation.setDeviceMetricsOverride", metrics.clone())
            .await?;
        if let Ok(mut state) = self.state.lock() {
            state.metrics = Some(metrics);
        }
        Ok(())
    }

    /// Playwright's `CRPage.updateEmulateMedia`: the page's media on its main session and on each
    /// out-of-process frame session in `children`, where a closed session is skipped.
    async fn send_media(&self, page: &PageSession, children: &[Arc<Host>]) -> Result<(), CdpError> {
        if let Ok(mut state) = self.state.lock() {
            state.restore_media = true;
        }
        let params = self.media_params();
        let main = page.send("Emulation.setEmulatedMedia", params.clone());
        let frames = children.iter().map(|host| {
            let params = params.clone();
            async move {
                match host.send("Emulation.setEmulatedMedia", params).await {
                    // Playwright: `server/chromium/crPage.ts` `CRPage._forAllFrameSessions`: `if (isSessionClosedError(e))`
                    Err(CdpError::SessionGone) => Ok(Value::Null),
                    Err(_) if host.gone().is_cancelled() => Ok(Value::Null),
                    other => other,
                }
            }
        });
        let attached = self
            .frames
            .lock()
            .map(|frames| frames.clone())
            .unwrap_or_default();
        let auto = attached.iter().map(|session| {
            let params = params.clone();
            async move {
                match page
                    .cdp()
                    .send("Emulation.setEmulatedMedia", params, Some(session))
                    .await
                {
                    Err(CdpError::SessionGone) => Ok(Value::Null),
                    other => other,
                }
            }
        });
        let (main, frames, auto) = tokio::join!(
            main,
            future::try_join_all(frames),
            future::try_join_all(auto)
        );
        main?;
        frames?;
        auto.map(|_| ())
    }

    /// Playwright's `FrameSession._updateEmulateMedia` params for the media emulated now.
    fn media_params(&self) -> Value {
        let media = self
            .state
            .lock()
            .map(|state| state.emulated_media())
            .unwrap_or_default();
        media_params(&media)
    }
}

/// Neo: native CDP layout reads avoid adding a page script to viewport cleanup.
async fn layout_size(page: &PageSession) -> Result<Value, CdpError> {
    let layout = page.send("Page.getLayoutMetrics", json!({})).await?;
    Ok(json!([
        layout["cssLayoutViewport"]["clientWidth"],
        layout["cssLayoutViewport"]["clientHeight"]
    ]))
}

/// What a failed update re-sends.
#[derive(Clone, Copy)]
enum Resync {
    Viewport,
    Media,
}

impl State {
    /// Playwright's `Page.emulatedSize`: what the page emulates, else its context's viewport.
    fn emulated_size(&self) -> Option<(Size, Size)> {
        self.size.or_else(|| {
            let context = self.context.as_ref()?;
            let viewport = context.viewport?;
            // Playwright: `server/page.ts` `Page.emulatedSize`: `return contextOptions.viewport ? { viewport: contextOptions.viewport, screen: contextOptions.screen || contextOptions.viewport } : undefined;`
            Some((viewport, context.screen.unwrap_or(viewport)))
        })
    }

    /// Playwright's `Page.emulatedMedia`, every feature resolved against the page's context.
    fn emulated_media(&self) -> EmulatedMedia {
        let context = self.context.as_ref().map(|context| context.media);
        let default = |set: Option<&'static str>, context: Option<&'static str>, playwright| {
            set.or_else(|| match self.context {
                Some(_) => Some(context.unwrap_or(playwright)),
                None => Some(ATTACHED_CONTEXT_MEDIA),
            })
        };
        EmulatedMedia {
            media: Some(self.media.media.unwrap_or(NO_OVERRIDE)),
            color_scheme: default(
                self.media.color_scheme,
                context.and_then(|media| media.color_scheme),
                DEFAULT_COLOR_SCHEME,
            ),
            reduced_motion: default(
                self.media.reduced_motion,
                context.and_then(|media| media.reduced_motion),
                DEFAULT_REDUCED_MOTION,
            ),
            forced_colors: default(
                self.media.forced_colors,
                context.and_then(|media| media.forced_colors),
                DEFAULT_FORCED_COLORS,
            ),
            contrast: default(
                self.media.contrast,
                context.and_then(|media| media.contrast),
                DEFAULT_CONTRAST,
            ),
        }
    }

    /// The device metrics override to send, with the window bounds that go first, or `None`
    /// when the page emulates no size or the override is the one already sent.
    #[expect(
        clippy::type_complexity,
        reason = "one tuple of the two writes and their order"
    )]
    fn metrics_update(&self) -> Option<(Value, Option<(i64, i64, i64)>)> {
        let (viewport, screen) = self.emulated_size()?;
        let context = self.context.as_deref();
        let is_mobile = context.is_some_and(|context| context.is_mobile);
        let is_landscape = screen.width > screen.height;
        // Playwright: `server/chromium/crPage.ts` `FrameSession._updateViewport`: `deviceScaleFactor: options.deviceScaleFactor || 1,`
        let device_scale_factor = context
            .and_then(|context| context.device_scale_factor)
            .filter(|factor| *factor != 0.0)
            .unwrap_or(1.0);
        let orientation = if is_mobile && is_landscape {
            json!({ "angle": 90, "type": "landscapePrimary" })
        } else if is_mobile {
            json!({ "angle": 0, "type": "portraitPrimary" })
        } else {
            json!({ "angle": 0, "type": "landscapePrimary" })
        };
        let metrics = json!({
            "mobile": is_mobile,
            "width": viewport.width,
            "height": viewport.height,
            "screenWidth": screen.width,
            "screenHeight": screen.height,
            "deviceScaleFactor": device_scale_factor,
            "screenOrientation": orientation,
        });
        if self.metrics.as_ref() == Some(&metrics) {
            return None;
        }
        let window = self.window.map(|window| {
            let (width, height) = if context.is_some_and(|context| context.headful) {
                headful_insets()
            } else {
                (0, 0)
            };
            (window, viewport.width + width, viewport.height + height)
        });
        Some((metrics, window))
    }
}

/// Watches the targets Chromium auto-attaches under a created context's page (`parent`) and the
/// frames and workers it attached in turn, configuring and releasing each
/// ([`PageEmulation::configure_target`]). Ends when the page's emulation is gone or `stop` fires.
///
/// Every target it misses stays paused, so it never skips one: a lagged event stream is logged,
/// and Chromium lets a paused target run once the session that paused it detaches, with the run.
async fn watch_targets(
    cdp: CdpClient,
    parent: SessionId,
    mut events: broadcast::Receiver<CdpEvent>,
    emulation: Weak<PageEmulation>,
    stop: CancellationToken,
) {
    let mut watched = HashSet::from([parent]);
    loop {
        let event = tokio::select! {
            biased;
            () = stop.cancelled() => return,
            event = events.recv() => event,
        };
        let event = match event {
            Ok(event) => event,
            Err(RecvError::Lagged(missed)) => {
                debug!(
                    missed,
                    "the auto-attach watcher lagged behind the browser's events"
                );
                continue;
            }
            Err(RecvError::Closed) => return,
        };
        let Some(from) = event
            .session_id
            .clone()
            .filter(|from| watched.contains(from))
        else {
            continue;
        };
        match event.method.as_str() {
            "Target.attachedToTarget" => {
                let Ok(attached) = serde_json::from_value::<AttachedTarget>(event.params) else {
                    continue;
                };
                let Some(emulation) = emulation.upgrade() else {
                    return;
                };
                let child = attached.session_id;
                if emulation
                    .configure_target(&cdp, &from, &child, &attached.target_info.kind)
                    .await
                {
                    watched.insert(child);
                }
            }
            "Target.detachedFromTarget" => {
                if let Ok(detached) = serde_json::from_value::<DetachedTarget>(event.params) {
                    watched.remove(&detached.session_id);
                    if let Some(emulation) = emulation.upgrade() {
                        emulation.forget_frame(&detached.session_id);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Playwright's `FrameSession._updateEmulateMedia`: `'no-override'` is sent as `''`, which
/// stops the override.
fn media_params(media: &EmulatedMedia) -> Value {
    // Playwright: `server/chromium/crPage.ts` `FrameSession._updateEmulateMedia`: `const media = emulatedMedia.media === 'no-override' ? '' : emulatedMedia.media;`
    let value = |literal: Option<&'static str>| match literal {
        None | Some(NO_OVERRIDE) => "",
        Some(literal) => literal,
    };
    json!({
        "media": value(media.media),
        "features": [
            { "name": "prefers-color-scheme", "value": value(media.color_scheme) },
            { "name": "prefers-reduced-motion", "value": value(media.reduced_motion) },
            { "name": "forced-colors", "value": value(media.forced_colors) },
            { "name": "prefers-contrast", "value": value(media.contrast) },
        ],
    })
}

/// The per-frame writes of Playwright's `FrameSession._initialize` for a created context's
/// options, each one `send(method, params)`; every frame session gets them, the main one too.
fn frame_options<'a, F>(
    context: &'a ContextEmulation,
    send: F,
) -> Vec<future::BoxFuture<'a, Result<(), CdpError>>>
where
    F: Fn(&'static str, Value) -> future::BoxFuture<'a, Result<(), CdpError>>,
{
    let mut steps = Vec::new();
    if context.bypass_csp {
        // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `promises.push(this._client.send('Page.setBypassCSP', { enabled: true }));`
        steps.push(send("Page.setBypassCSP", json!({ "enabled": true })));
    }
    if context.ignore_https_errors {
        // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `promises.push(this._client.send('Security.setIgnoreCertificateErrors', { ignore: true }));`
        steps.push(send(
            "Security.setIgnoreCertificateErrors",
            json!({ "ignore": true }),
        ));
    }
    if context.has_touch {
        // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `promises.push(this._client.send('Emulation.setTouchEmulationEnabled', { enabled: true }));`
        steps.push(send(
            "Emulation.setTouchEmulationEnabled",
            json!({ "enabled": true }),
        ));
    }
    if context.java_script_enabled == Some(false) {
        // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `promises.push(this._client.send('Emulation.setScriptExecutionDisabled', { value: true }));`
        steps.push(send(
            "Emulation.setScriptExecutionDisabled",
            json!({ "value": true }),
        ));
    }
    if context.user_agent.is_some() || context.locale.is_some() {
        steps.push(send(
            "Emulation.setUserAgentOverride",
            user_agent_params(context),
        ));
    }
    if let Some(locale) = &context.locale {
        let locale = send("Emulation.setLocaleOverride", json!({ "locale": locale }));
        steps.push(Box::pin(emulate_locale(locale)));
    }
    if let Some(timezone_id) = &context.timezone_id {
        let timezone = send(
            "Emulation.setTimezoneOverride",
            json!({ "timezoneId": timezone_id }),
        );
        steps.push(Box::pin(emulate_timezone(timezone, timezone_id.clone())));
    }
    steps
}

/// Playwright's `emulateLocale`: an override another page of the renderer already set is the
/// same locale, which is not an error.
async fn emulate_locale(sent: future::BoxFuture<'_, Result<(), CdpError>>) -> Result<(), CdpError> {
    match sent.await {
        // Playwright: `server/chromium/crPage.ts` `emulateLocale`: `if (exception.message.includes('Another locale override is already in effect'))`
        Err(CdpError::Protocol { message, .. })
            if message.contains("Another locale override is already in effect") =>
        {
            Ok(())
        }
        other => other,
    }
}

/// Playwright's `emulateTimezone`: an override already in effect is kept, and an unknown time
/// zone names the one given.
async fn emulate_timezone(
    sent: future::BoxFuture<'_, Result<(), CdpError>>,
    timezone_id: String,
) -> Result<(), CdpError> {
    match sent.await {
        // Playwright: `server/chromium/crPage.ts` `emulateTimezone`: `if (exception.message.includes('Timezone override is already in effect'))`
        Err(CdpError::Protocol { message, .. })
            if message.contains("Timezone override is already in effect") =>
        {
            Ok(())
        }
        // Playwright: `server/chromium/crPage.ts` `emulateTimezone`: `Invalid timezone ID: ${timezoneId}`
        Err(CdpError::Protocol { code, message }) if message.contains("Invalid timezone") => {
            Err(CdpError::Protocol {
                code,
                message: format!("Invalid timezone ID: {timezone_id}"),
            })
        }
        other => other,
    }
}

/// Playwright's `FrameSession._updateUserAgent` params.
fn user_agent_params(context: &ContextEmulation) -> Value {
    // Playwright: `server/chromium/crPage.ts` `FrameSession._updateUserAgent`: `userAgent: options.userAgent || '',`
    let mut params = json!({ "userAgent": context.user_agent.clone().unwrap_or_default() });
    if let Some(locale) = &context.locale {
        params["acceptLanguage"] = json!(locale);
    }
    if let Some(metadata) = user_agent_metadata(context) {
        params["userAgentMetadata"] = metadata;
    }
    params
}

/// Playwright's `calculateUserAgentMetadata`: the client hints a user agent string implies.
fn user_agent_metadata(context: &ContextEmulation) -> Option<Value> {
    let ua = context.user_agent.as_deref().filter(|ua| !ua.is_empty())?;
    // Playwright: `server/chromium/crPage.ts` `calculateUserAgentMetadata`: `platform: 'Windows',`
    let mut platform = "Windows";
    let mut platform_version = String::new();
    // Playwright: `server/chromium/crPage.ts` `calculateUserAgentMetadata`: `architecture: 'x86',`
    let mut architecture = "x86";
    if let Some(version) = version_after(ua, "Android ", '.', 3) {
        platform = "Android";
        platform_version = version;
        architecture = "arm";
    } else if let Some(version) = version_after(ua, "iPhone OS ", '_', 2) {
        platform = "iOS";
        platform_version = version;
        architecture = "arm";
    } else if let Some(version) = version_after(ua, "iPad; CPU OS ", '_', 2) {
        platform = "iOS";
        platform_version = version;
        architecture = "arm";
    } else if let Some(version) = version_after(ua, "Mac OS X ", '_', 3) {
        platform = "macOS";
        platform_version = version;
        if !ua.contains("Intel") {
            architecture = "arm";
        }
    } else if let Some(version) = windows_version(ua) {
        platform = "Windows";
        platform_version = version;
    } else if ua.to_lowercase().contains("linux") {
        platform = "Linux";
    }
    if ua.contains("ARM") {
        architecture = "arm";
    }
    Some(json!({
        "mobile": context.is_mobile,
        "model": "",
        "architecture": architecture,
        "platform": platform,
        "platformVersion": platform_version,
    }))
}

/// The first match of Playwright's `<prefix>(\d+(<sep>\d+)?…)` with at most `parts` numbers:
/// `/Android (\d+(\.\d+)?(\.\d+)?)/` is `version_after(ua, "Android ", '.', 3)`.
fn version_after(ua: &str, prefix: &str, separator: char, parts: usize) -> Option<String> {
    ua.match_indices(prefix)
        .find_map(|(at, _)| numbers(&ua[at + prefix.len()..], separator, parts))
}

/// Playwright's `/Windows\D+(\d+(\.\d+)?(\.\d+)?)/`: `Windows`, then one or more non-digits,
/// then the version.
fn windows_version(ua: &str) -> Option<String> {
    ua.match_indices("Windows").find_map(|(at, _)| {
        let rest = &ua[at + "Windows".len()..];
        let skipped = rest.find(|c: char| c.is_ascii_digit())?;
        (skipped > 0).then(|| numbers(&rest[skipped..], '.', 3))?
    })
}

/// `\d+(<sep>\d+)?…` at the start of `text`, with at most `parts` numbers.
fn numbers(text: &str, separator: char, parts: usize) -> Option<String> {
    let digits = |text: &str| {
        text.find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len())
    };
    let first = digits(text);
    if first == 0 {
        return None;
    }
    let mut end = first;
    for _ in 1..parts {
        let rest = &text[end..];
        let Some(after) = rest.strip_prefix(separator) else {
            break;
        };
        let more = digits(after);
        if more == 0 {
            break;
        }
        end += separator.len_utf8() + more;
    }
    Some(text[..end].to_string())
}

/// Playwright's headful window insets for the platform Neo's server runs on.
fn headful_insets() -> (i64, i64) {
    if cfg!(target_os = "windows") {
        HEADFUL_WINDOWS_INSETS
    } else if cfg!(target_os = "linux") {
        HEADFUL_LINUX_INSETS
    } else if cfg!(target_os = "macos") {
        HEADFUL_MAC_INSETS
    } else {
        HEADFUL_INSETS
    }
}

/// Playwright's `Page.requestGC` (`CRPage.requestGC`): a full garbage collection in the page's
/// renderer.
pub(crate) async fn request_gc(page: &PageSession, progress: &Progress) -> Result<(), CallError> {
    // Playwright: `server/chromium/crPage.ts` `CRPage.requestGC`: `await this._mainFrameSession._client.send('HeapProfiler.collectGarbage');`
    finish(
        progress
            .race(page.send("HeapProfiler.collectGarbage", json!({})))
            .await
            .map(|result| result.map(|_| ())),
    )
}

/// A write's reply, dropped: every emulation command replies `{}`.
async fn sent(reply: impl Future<Output = Result<Value, CdpError>>) -> Result<(), CdpError> {
    reply.await.map(|_| ())
}

/// A raced update's outcome as the call's result.
fn finish(updated: Result<Result<(), CdpError>, CallError>) -> Result<(), CallError> {
    match updated {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(tabs::cdp_error(&error).into()),
        Err(error) => Err(error),
    }
}

/// The protocol literal of an `emulateMedia` `media` option.
fn media_literal(media: PageEmulateMediaParamsMedia) -> &'static str {
    match media {
        PageEmulateMediaParamsMedia::Screen => "screen",
        PageEmulateMediaParamsMedia::Print => "print",
        PageEmulateMediaParamsMedia::NoOverride => NO_OVERRIDE,
    }
}

/// The protocol literal of an `emulateMedia` `colorScheme` option.
fn color_scheme_literal(color_scheme: PageEmulateMediaParamsColorScheme) -> &'static str {
    match color_scheme {
        PageEmulateMediaParamsColorScheme::Dark => "dark",
        PageEmulateMediaParamsColorScheme::Light => "light",
        PageEmulateMediaParamsColorScheme::NoPreference => "no-preference",
        PageEmulateMediaParamsColorScheme::NoOverride => NO_OVERRIDE,
    }
}

/// The protocol literal of an `emulateMedia` `reducedMotion` option.
fn reduced_motion_literal(reduced_motion: PageEmulateMediaParamsReducedMotion) -> &'static str {
    match reduced_motion {
        PageEmulateMediaParamsReducedMotion::Reduce => "reduce",
        PageEmulateMediaParamsReducedMotion::NoPreference => "no-preference",
        PageEmulateMediaParamsReducedMotion::NoOverride => NO_OVERRIDE,
    }
}

/// The protocol literal of an `emulateMedia` `forcedColors` option.
fn forced_colors_literal(forced_colors: PageEmulateMediaParamsForcedColors) -> &'static str {
    match forced_colors {
        PageEmulateMediaParamsForcedColors::Active => "active",
        PageEmulateMediaParamsForcedColors::None => "none",
        PageEmulateMediaParamsForcedColors::NoOverride => NO_OVERRIDE,
    }
}

/// The protocol literal of an `emulateMedia` `contrast` option.
fn contrast_literal(contrast: PageEmulateMediaParamsContrast) -> &'static str {
    match contrast {
        PageEmulateMediaParamsContrast::NoPreference => "no-preference",
        PageEmulateMediaParamsContrast::More => "more",
        PageEmulateMediaParamsContrast::NoOverride => NO_OVERRIDE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(user_agent: &str, is_mobile: bool) -> ContextEmulation {
        ContextEmulation {
            user_agent: Some(user_agent.to_string()),
            is_mobile,
            ..ContextEmulation::default()
        }
    }

    #[test]
    fn user_agent_metadata_matches_playwrights_formula() {
        // The user agents of Playwright's device descriptors at the pin.
        let iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 15_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/27.2 Mobile/15E148 Safari/604.1";
        let pixel = "Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/158.0.8127.18 Mobile Safari/537.36";
        let desktop = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/158.0.8127.18 Safari/537.36";
        let mac = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/27.2 Safari/605.1.15";
        let ipad = "Mozilla/5.0 (iPad; CPU OS 12_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/27.2 Mobile/15E148 Safari/604.1";
        let cases = [
            (iphone, true, "iOS", "15_0", "arm"),
            (pixel, true, "Android", "14", "arm"),
            (desktop, false, "Windows", "10.0", "x86"),
            (mac, false, "macOS", "10_15_7", "x86"),
            (ipad, true, "iOS", "12_2", "arm"),
            ("Mozilla/5.0 (X11; Linux x86_64)", false, "Linux", "", "x86"),
            ("Mozilla/5.0 (X11; Linux ARM)", false, "Linux", "", "arm"),
            (
                "Mozilla/5.0 (Macintosh; Mac OS X 14_1)",
                false,
                "macOS",
                "14_1",
                "arm",
            ),
        ];
        for (ua, mobile, platform, version, architecture) in cases {
            assert_eq!(
                user_agent_metadata(&context(ua, mobile)),
                Some(json!({
                    "mobile": mobile,
                    "model": "",
                    "architecture": architecture,
                    "platform": platform,
                    "platformVersion": version,
                })),
                "{ua}"
            );
        }
        assert_eq!(user_agent_metadata(&context("", false)), None);
    }

    #[test]
    fn version_patterns_stop_where_playwrights_regexes_stop() {
        assert_eq!(
            version_after("Android 4.4.2.1", "Android ", '.', 3),
            Some("4.4.2".into())
        );
        assert_eq!(
            version_after("Android x Android 9", "Android ", '.', 3),
            Some("9".into())
        );
        assert_eq!(version_after("Android .5", "Android ", '.', 3), None);
        assert_eq!(
            version_after("iPhone OS 15_0_1", "iPhone OS ", '_', 2),
            Some("15_0".into())
        );
        assert_eq!(
            version_after("Android 7.", "Android ", '.', 3),
            Some("7".into())
        );
        assert_eq!(windows_version("Windows NT 6.1; WOW64"), Some("6.1".into()));
        assert_eq!(windows_version("Windows10"), None);
        assert_eq!(windows_version("Windows Phone 10.0"), Some("10.0".into()));
    }

    #[test]
    fn media_resolves_against_the_pages_context() {
        let attached = State::default();
        assert_eq!(
            media_params(&attached.emulated_media()),
            json!({"media": "", "features": [
                {"name": "prefers-color-scheme", "value": ""},
                {"name": "prefers-reduced-motion", "value": ""},
                {"name": "forced-colors", "value": ""},
                {"name": "prefers-contrast", "value": ""},
            ]}),
            "the run's own context keeps the browser's media"
        );
        let created = State {
            context: Some(Arc::new(ContextEmulation {
                media: EmulatedMedia {
                    reduced_motion: Some("reduce"),
                    ..EmulatedMedia::default()
                },
                ..ContextEmulation::default()
            })),
            media: EmulatedMedia {
                media: Some("print"),
                contrast: Some(NO_OVERRIDE),
                ..EmulatedMedia::default()
            },
            ..State::default()
        };
        assert_eq!(
            media_params(&created.emulated_media()),
            json!({"media": "print", "features": [
                {"name": "prefers-color-scheme", "value": "light"},
                {"name": "prefers-reduced-motion", "value": "reduce"},
                {"name": "forced-colors", "value": "none"},
                {"name": "prefers-contrast", "value": ""},
            ]}),
            "a created context defaults as Playwright's does"
        );
    }

    #[test]
    fn metrics_follow_playwrights_update_viewport() {
        let size = Size {
            width: 390,
            height: 664,
        };
        let screen = Size {
            width: 390,
            height: 844,
        };
        let mut state = State {
            context: Some(Arc::new(ContextEmulation {
                viewport: Some(size),
                screen: Some(screen),
                device_scale_factor: Some(3.0),
                is_mobile: true,
                ..ContextEmulation::default()
            })),
            window: Some(7),
            ..State::default()
        };
        let (metrics, window) = state.metrics_update().unwrap();
        assert_eq!(
            metrics,
            json!({
                "mobile": true,
                "width": 390,
                "height": 664,
                "screenWidth": 390,
                "screenHeight": 844,
                "deviceScaleFactor": 3.0,
                "screenOrientation": {"angle": 0, "type": "portraitPrimary"},
            })
        );
        assert_eq!(
            window,
            Some((7, 390, 664)),
            "headless windows have no insets"
        );
        state.metrics = Some(metrics);
        assert!(
            state.metrics_update().is_none(),
            "the same override is not sent twice"
        );
        assert!(
            State::default().metrics_update().is_none(),
            "the run's own context emulates no size until the script sets one"
        );
        let landscape = State {
            size: Some((
                Size {
                    width: 800,
                    height: 400,
                },
                Size {
                    width: 800,
                    height: 400,
                },
            )),
            ..State::default()
        };
        let (metrics, window) = landscape.metrics_update().unwrap();
        assert_eq!(metrics["mobile"], json!(false));
        assert_eq!(metrics["deviceScaleFactor"], json!(1.0));
        assert_eq!(
            metrics["screenOrientation"],
            json!({"angle": 0, "type": "landscapePrimary"})
        );
        assert_eq!(window, None, "the run's own pages keep their window");
    }
}
