use crate::connection::CdpConnection;
use crate::{CoreError, FrameId, PageId, ProtocolSession, SessionId};
use browseros_cdp::{CdpEvent, target};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex as StdMutex, PoisonError, Weak};
use tokio::sync::{Mutex, broadcast};
use tracing::warn;

/// Page-owned renderer configuration installed before a new iframe is resumed.
/// Registrations are weak: the SDK owns the settings and unregisters by dropping its lease.
pub trait FrameInitializer: Send + Sync {
    /// Whether a renderer that `parent` auto-attaches inherits configuration it must receive
    /// before its first script runs. Core pauses new frames under `parent` only then.
    ///
    /// A pause is not free: Chromium recomputes a frame's auto-attached children whenever any
    /// session, from any client, arms auto-attach on it, detaches a child that is still
    /// navigating, and never releases that child's pause, so its page never loads. Answer
    /// `true` only while there is something to install.
    fn configures(&self, _parent: &SessionId) -> bool {
        true
    }
    /// Inherit page configuration from the session that auto-attached this renderer.
    fn initialize<'a>(
        &'a self,
        session: &'a ProtocolSession,
        parent: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<(), CoreError>>;
    /// Restore retained page settings when the page receives a new root session.
    fn page_attached<'a>(
        &'a self,
        _session: &'a ProtocolSession,
        _page: &'a PageId,
    ) -> BoxFuture<'a, Result<(), CoreError>> {
        Box::pin(async { Ok(()) })
    }
    /// Rediscover descendants after event lag, before unclaimed paused targets are resumed.
    fn recover(&self) -> BoxFuture<'_, Result<(), CoreError>> {
        Box::pin(async { Ok(()) })
    }
    /// Release endpoint-owned state after a detach, refused command, or lag reconciliation.
    fn detached(&self, _session: &SessionId) {}
}

fn frame_auto_attach(wait_for_debugger: bool) -> Value {
    // Keep worker attach events: Chromium can pause even workers excluded by a target filter,
    // hiding the event needed to release them. `on_attached` resumes non-frame targets directly.
    json!({
        "autoAttach": true,
        "waitForDebuggerOnStart": wait_for_debugger,
        "flatten": true,
    })
}

/// One frame's CDP endpoint, as [`FrameRegistry::resolve_frame_target`] found it.
#[derive(Debug, Clone)]
pub struct FrameTarget {
    pub session: ProtocolSession,
    /// `{ "frameId": .. }` for a same-process child, whose session also serves its parent, so
    /// accessibility reads must name the frame. Empty for page and OOPIF sessions.
    pub ax_params: Value,
    /// Root and OOPIF sessions already inspect the intended document. Same-process children share
    /// their nearest parent target session, so cursor acquisition must target the resolved child
    /// Document object instead.
    pub cursor_uses_session_default: bool,
}

/// Which CDP session reaches each frame of each attached page. A page's own session is
/// registered when core attaches the tab. Out-of-process iframes (OOPIFs) arrive through
/// auto-attach events or are attached on demand, and a same-process child shares the session of
/// the parent it was found through. A new connection epoch clears every entry, because flattened
/// session ids belong to one websocket. None of its locks is held across a send on `cdp`,
/// because a send refused as gone takes them to forget its session (see
/// `BrowserSession::forget_session`).
pub struct FrameRegistry {
    cdp: Arc<dyn CdpConnection>,
    initializers: StdMutex<Vec<Weak<dyn FrameInitializer>>>,
    connection_epoch: Mutex<u64>,
    oopif_sessions: Mutex<HashMap<FrameId, SessionId>>,
    same_process_sessions: Mutex<HashMap<FrameId, SessionId>>,
    /// How a parent last resolved each child frame. See [`FrameLink`].
    frame_links: Mutex<HashMap<FrameId, FrameLink>>,
    page_sessions: Mutex<HashMap<PageId, SessionId>>,
}

/// A child frame as its parent resolved it: the parent's endpoint, whose document holds the
/// frame's owner element, and the endpoint the child was read through. Page.getFrameTree omits
/// remote frames, so frame admission asks this parent for the owner instead. Only a hint for
/// where to ask: admission re-verifies the owner on every call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrameLink {
    pub(crate) parent: SessionId,
    pub(crate) endpoint: SessionId,
}

impl FrameRegistry {
    #[must_use]
    pub fn new(cdp: Arc<dyn CdpConnection>) -> Arc<Self> {
        let connection_epoch = cdp.connection_epoch();
        let registry = Arc::new(Self {
            cdp,
            initializers: StdMutex::default(),
            connection_epoch: Mutex::new(connection_epoch),
            oopif_sessions: Mutex::new(HashMap::new()),
            same_process_sessions: Mutex::new(HashMap::new()),
            frame_links: Mutex::new(HashMap::new()),
            page_sessions: Mutex::new(HashMap::new()),
        });
        Self::spawn_event_listener(registry.clone());
        registry
    }

    /// Register renderer setup owned by a higher layer without extending that owner's lifetime.
    pub fn register_initializer(&self, initializer: &Arc<dyn FrameInitializer>) {
        self.initializers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::downgrade(initializer));
    }

    fn initializers(&self) -> Vec<Arc<dyn FrameInitializer>> {
        let mut slots = self
            .initializers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let live: Vec<_> = slots.iter().filter_map(Weak::upgrade).collect();
        slots.retain(|slot| slot.strong_count() > 0);
        live
    }

    /// Whether frames `parent` auto-attaches start paused: only when some initializer has
    /// configuration for them ([`FrameInitializer::configures`]).
    fn pauses_children_of(
        initializers: &[Arc<dyn FrameInitializer>],
        parent: Option<&SessionId>,
    ) -> bool {
        parent.is_some_and(|parent| {
            initializers
                .iter()
                .any(|initializer| initializer.configures(parent))
        })
    }

    /// Arm recursive attachment before a page with persistent settings can create a renderer.
    pub async fn enable_initialization(&self, session: &ProtocolSession) -> Result<(), CoreError> {
        session
            .send_value("Target.setAutoAttach", frame_auto_attach(true))
            .await?;
        Ok(())
    }

    pub async fn register_page(
        &self,
        page_session: ProtocolSession,
        page_id: PageId,
        session_id: SessionId,
    ) -> Result<(), CoreError> {
        self.clear_sessions_from_prior_connection().await;
        {
            // Serialize guarded registration with epoch-driven cache clearing. Otherwise an
            // old attachment hook could await here, then repopulate the new epoch's cache.
            let cached_epoch = self.connection_epoch.lock().await;
            let mut sessions = self.page_sessions.lock().await;
            if let Some(guard) = page_session.epoch_guard() {
                guard.check(self.cdp.as_ref())?;
                if *cached_epoch != guard.expected {
                    return Err(guard.invalidated.clone().into());
                }
            }
            sessions.insert(page_id.clone(), session_id.clone());
        }
        let initialized = page_session.at_connection_epoch();
        let initializers = self.initializers();
        for initializer in &initializers {
            initializer.page_attached(&initialized, &page_id).await?;
        }
        // Asked after `page_attached`, which restores a page's retained configuration.
        let pause = Self::pauses_children_of(&initializers, Some(&session_id));
        let setup = page_session
            .send::<_, Value>("Target.setAutoAttach", frame_auto_attach(pause))
            .await;
        if let Err(error @ CoreError::Target(_)) = setup {
            return Err(error);
        }
        Ok(())
    }

    /// The endpoint for the main frame (`None`) or one of `page_id`'s child frames. A known OOPIF
    /// answers on its own session. A `cross_process` child (its owner element has no
    /// `contentDocument` in the parent) that has not attached yet is attached by its target id.
    /// Any other child shares `same_process_parent`'s session. Without a parent it reuses the
    /// session it inherited before, else the page's. Passing the parent also records the
    /// parent-to-child link that frame admission climbs later. The page must already be
    /// attached. `BrowserView::page_session` attaches it.
    pub async fn resolve_frame_target(
        &self,
        page_id: PageId,
        frame_id: Option<FrameId>,
        same_process_parent: Option<&ProtocolSession>,
        cross_process: bool,
    ) -> Result<FrameTarget, CoreError> {
        self.resolve_frame_target_with_epoch(
            page_id,
            frame_id,
            same_process_parent,
            cross_process,
            None,
        )
        .await
    }

    /// On-demand child attachment is part of the caller's dispatch, not a new unbound task.
    /// Keep its epoch through native attachment and domain setup before exposing the endpoint.
    pub(crate) async fn resolve_frame_target_with_epoch(
        &self,
        page_id: PageId,
        frame_id: Option<FrameId>,
        same_process_parent: Option<&ProtocolSession>,
        cross_process: bool,
        guard: Option<crate::connection::EpochGuard>,
    ) -> Result<FrameTarget, CoreError> {
        if let Some(guard) = &guard {
            guard.check(self.cdp.as_ref())?;
        }
        self.clear_sessions_from_prior_connection().await;
        let page_session_id = self
            .page_sessions
            .lock()
            .await
            .get(&page_id)
            .cloned()
            .ok_or_else(|| CoreError::Message(format!("Page {page_id} has no attached session")))?;
        let Some(frame_id) = frame_id else {
            return Ok(FrameTarget {
                session: ProtocolSession::for_session(self.cdp.clone(), page_session_id)
                    .with_epoch_guard(guard),
                ax_params: json!({}),
                cursor_uses_session_default: true,
            });
        };
        // Boxed for the same reason as `PageAccess::frame_session`: many callers embed this.
        let target = Box::pin(self.resolve_child_target(
            page_session_id,
            frame_id.clone(),
            same_process_parent,
            cross_process,
            guard,
        ))
        .await?;
        if let (Some(parent), Some(endpoint)) = (
            same_process_parent.and_then(ProtocolSession::session_id),
            target.session.session_id(),
        ) {
            let link = FrameLink {
                parent: parent.clone(),
                endpoint: endpoint.clone(),
            };
            self.frame_links.lock().await.insert(frame_id, link);
        }
        Ok(target)
    }

    async fn resolve_child_target(
        &self,
        page_session_id: SessionId,
        frame_id: FrameId,
        same_process_parent: Option<&ProtocolSession>,
        cross_process: bool,
        guard: Option<crate::connection::EpochGuard>,
    ) -> Result<FrameTarget, CoreError> {
        if let Some(oopif) = self.oopif_sessions.lock().await.get(&frame_id).cloned() {
            return Ok(FrameTarget {
                session: ProtocolSession::for_session(self.cdp.clone(), oopif)
                    .with_epoch_guard(guard),
                ax_params: json!({}),
                cursor_uses_session_default: true,
            });
        }
        // A cross-process child (no contentDocument on the parent) that is not in the cache yet may
        // be an OOPIF whose attach event has not landed; nested cross-origin frames attach through
        // an async cascade a capture can outrun. Attach it by its target id so it is driven on its
        // own session rather than mis-queried on the parent's. A same-process child never reaches
        // this, so it keeps inheriting its parent session. A concurrent event may register it in
        // the meantime, so re-check the cache after attaching.
        if cross_process {
            if let Some(session_id) = self
                .attach_oopif_on_demand(
                    &frame_id,
                    same_process_parent
                        .and_then(ProtocolSession::session_id)
                        .cloned()
                        .or_else(|| Some(page_session_id.clone())),
                    guard.clone(),
                )
                .await?
            {
                return Ok(FrameTarget {
                    session: ProtocolSession::for_session(self.cdp.clone(), session_id)
                        .with_epoch_guard(guard),
                    ax_params: json!({}),
                    cursor_uses_session_default: true,
                });
            }
            if let Some(oopif) = self.oopif_sessions.lock().await.get(&frame_id).cloned() {
                return Ok(FrameTarget {
                    session: ProtocolSession::for_session(self.cdp.clone(), oopif)
                        .with_epoch_guard(guard),
                    ax_params: json!({}),
                    cursor_uses_session_default: true,
                });
            }
        }
        // A non-target frame shares its nearest parent target's CDP session, which may itself be
        // an OOPIF. Cache that inheritance because later ref resolution knows the frame id but no
        // longer has the capture-time parent target.
        let inherited_session = same_process_parent
            .and_then(ProtocolSession::session_id)
            .cloned();
        let session_id = if let Some(inherited_session) = inherited_session {
            self.same_process_sessions
                .lock()
                .await
                .insert(frame_id.clone(), inherited_session.clone());
            inherited_session
        } else {
            self.same_process_sessions
                .lock()
                .await
                .get(&frame_id)
                .cloned()
                .unwrap_or(page_session_id)
        };
        Ok(FrameTarget {
            session: ProtocolSession::for_session(self.cdp.clone(), session_id)
                .with_epoch_guard(guard),
            ax_params: json!({ "frameId": frame_id.0 }),
            cursor_uses_session_default: false,
        })
    }

    /// Scoped frame endpoints verify both their owner element and this live endpoint mapping.
    /// Frame IDs alone survive navigations and cannot authorize a retained OOPIF session.
    pub(crate) async fn frame_session_matches(&self, frame: &FrameId, session: &SessionId) -> bool {
        self.clear_sessions_from_prior_connection().await;
        self.oopif_sessions
            .lock()
            .await
            .get(frame)
            .is_some_and(|id| id == session)
            || self
                .same_process_sessions
                .lock()
                .await
                .get(frame)
                .is_some_and(|id| id == session)
    }

    /// How a parent last resolved `frame`. See [`FrameLink`].
    pub(crate) async fn frame_link(&self, frame: &FrameId) -> Option<FrameLink> {
        self.clear_sessions_from_prior_connection().await;
        self.frame_links.lock().await.get(frame).cloned()
    }

    /// The OOPIF whose own target `session` is attached to.
    pub(crate) async fn oopif_for_session(&self, session: &SessionId) -> Option<FrameId> {
        self.clear_sessions_from_prior_connection().await;
        self.oopif_sessions
            .lock()
            .await
            .iter()
            .find(|(_, id)| *id == session)
            .map(|(frame, _)| frame.clone())
    }

    async fn clear_sessions_from_prior_connection(&self) {
        let current_epoch = self.cdp.connection_epoch();
        let mut cached_epoch = self.connection_epoch.lock().await;
        if *cached_epoch == current_epoch {
            return;
        }
        // Flattened target session ids are scoped to one CDP websocket. A reconnect can reuse
        // frame ids while every cached page, OOPIF, and inherited session id is already invalid.
        self.oopif_sessions.lock().await.clear();
        self.same_process_sessions.lock().await.clear();
        self.frame_links.lock().await.clear();
        self.page_sessions.lock().await.clear();
        *cached_epoch = current_epoch;
    }

    fn spawn_event_listener(registry: Arc<Self>) {
        let mut events = registry.cdp.events();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => registry.handle_event(event).await,
                    // A frame-heavy page can attach many targets at once and overflow the CDP
                    // event buffer. Keep listening instead of dying for the rest of the session: a
                    // missed attach is recovered on demand when a frame is resolved, and a missed
                    // detach is reconciled against the live target list so a stale session cannot
                    // shadow that recovery.
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!("browser frame listener lagged by {skipped} CDP event(s)");
                        registry.reconcile_oopifs_after_lag().await;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    async fn handle_event(&self, event: CdpEvent) {
        match event.method.as_str() {
            "Target.attachedToTarget" => {
                let parsed = serde_json::from_value::<target::AttachedToTargetEvent>(event.params);
                if let Ok(params) = parsed {
                    self.on_attached(params, event.session_id).await;
                }
            }
            "Target.detachedFromTarget" => {
                let parsed =
                    serde_json::from_value::<target::DetachedFromTargetEvent>(event.params);
                if let Ok(params) = parsed {
                    self.on_detached(&SessionId::from(params.session_id)).await;
                }
            }
            _ => {}
        }
    }

    async fn on_attached(&self, params: target::AttachedToTargetEvent, parent: Option<SessionId>) {
        self.clear_sessions_from_prior_connection().await;
        if params.target_info.r#type != "iframe" {
            // Media belongs to frame renderers. Auto-attach also pauses workers, which must
            // start immediately rather than waiting for an initializer that never owns them.
            if params.waiting_for_debugger {
                let session = ProtocolSession::for_session(
                    self.cdp.clone(),
                    SessionId::from(params.session_id),
                )
                .at_connection_epoch();
                let _resumed = session
                    .send_value("Runtime.runIfWaitingForDebugger", json!({}))
                    .await;
            }
            return;
        }
        self.register_oopif_session(
            FrameId(params.target_info.target_id),
            SessionId::from(params.session_id),
            params.waiting_for_debugger,
            parent,
        )
        .await;
    }

    /// Record an OOPIF's session and prepare it for capture: enable the domains the snapshot
    /// uses and re-arm auto-attach so this OOPIF's own cross-origin children attach too. Shared
    /// by the attach event and the on-demand attach below.
    async fn register_oopif_session(
        &self,
        frame_id: FrameId,
        session_id: SessionId,
        waiting_for_debugger: bool,
        parent: Option<SessionId>,
    ) {
        let _ = self
            .register_oopif_session_with_epoch(
                frame_id,
                session_id,
                waiting_for_debugger,
                parent,
                None,
            )
            .await;
    }

    async fn register_oopif_session_with_epoch(
        &self,
        frame_id: FrameId,
        session_id: SessionId,
        waiting_for_debugger: bool,
        parent: Option<SessionId>,
        guard: Option<crate::connection::EpochGuard>,
    ) -> Result<(), CoreError> {
        let initializers = self.initializers();
        let guard = guard.or_else(|| {
            if initializers.is_empty() {
                return None;
            }
            ProtocolSession::for_session(self.cdp.clone(), session_id.clone())
                .cancellation_endpoint()
                .epoch_guard()
                .cloned()
        });
        {
            let cached_epoch = self.connection_epoch.lock().await;
            let mut sessions = self.oopif_sessions.lock().await;
            if let Some(guard) = &guard {
                guard.check(self.cdp.as_ref())?;
                if *cached_epoch != guard.expected {
                    return Err(guard.invalidated.clone().into());
                }
            }
            sessions.insert(frame_id, session_id.clone());
        }
        let session = ProtocolSession::for_session(self.cdp.clone(), session_id)
            .with_epoch_guard(guard.clone());
        if waiting_for_debugger && initializers.is_empty() {
            let _ = session
                .send::<_, Value>("Runtime.runIfWaitingForDebugger", json!({}))
                .await;
        }
        let _ = session.send::<_, Value>("DOM.enable", json!({})).await;
        let _ = session
            .send::<_, Value>("Accessibility.enable", json!({}))
            .await;
        // This frame inherits `parent`'s configuration, so its own children need it too.
        let pause = Self::pauses_children_of(&initializers, parent.as_ref());
        let _ = session
            .send::<_, Value>("Target.setAutoAttach", frame_auto_attach(pause))
            .await;
        if !initializers.is_empty() {
            // Recursive auto-attach is armed before initialization and resume, so nested
            // OOPIF startup scripts cannot outrun their page's persistent configuration.
            let prepared = async {
                for initializer in &initializers {
                    initializer.initialize(&session, parent.as_ref()).await?;
                }
                Ok::<(), CoreError>(())
            }
            .await;
            // Always release a paused renderer, even if setup failed or the owner disappeared.
            let resumed = session
                .send_value("Runtime.runIfWaitingForDebugger", json!({}))
                .await;
            prepared?;
            resumed?;
        }
        if let Some(guard) = &guard {
            guard.check(self.cdp.as_ref())?;
        }
        Ok(())
    }

    /// A frame missing from `oopif_sessions` may still be an OOPIF whose `attachedToTarget` event
    /// has not arrived yet: nested cross-origin frames attach through an async cascade that a
    /// synchronous capture can outrun. For an OOPIF the frame id equals its target id, so attaching
    /// by that id both proves it is a separate target and yields its session. A same-process frame
    /// has no such target and the attach fails, so the caller falls back to the parent session.
    /// Without this a not-yet-attached OOPIF is queried on its parent's session, which cannot
    /// resolve a cross-process frame id and returns an empty subtree.
    async fn attach_oopif_on_demand(
        &self,
        frame_id: &FrameId,
        parent: Option<SessionId>,
        guard: Option<crate::connection::EpochGuard>,
    ) -> Result<Option<SessionId>, CoreError> {
        let root = ProtocolSession::root(self.cdp.clone()).with_epoch_guard(guard.clone());
        let attached = root
            .send::<_, target::AttachToTargetResult>(
                "Target.attachToTarget",
                json!({ "targetId": frame_id.0, "flatten": true }),
            )
            .await;
        let attached = match attached {
            Ok(attached) => attached,
            Err(error @ CoreError::Target(_)) => return Err(error),
            Err(_) => return Ok(None),
        };
        let session_id = SessionId::from(attached.session_id);
        self.register_oopif_session_with_epoch(
            frame_id.clone(),
            session_id.clone(),
            false,
            parent,
            guard,
        )
        .await?;
        Ok(Some(session_id))
    }

    /// Forgets a session that died, whether a detach event or a refused command said so. It
    /// does what [`Self::on_detached`] does and also releases the page attached under it, by
    /// value, so a page already re-attached under a new session keeps that one. See
    /// `BrowserSession::forget_session` for the lock rule.
    pub(crate) async fn forget_session(&self, session_id: &SessionId) {
        self.on_detached(session_id).await;
        self.page_sessions
            .lock()
            .await
            .retain(|_, existing| existing != session_id);
    }

    async fn on_detached(&self, session_id: &SessionId) {
        self.clear_sessions_from_prior_connection().await;
        self.oopif_sessions
            .lock()
            .await
            .retain(|_, existing| existing != session_id);
        self.same_process_sessions
            .lock()
            .await
            .retain(|_, existing| existing != session_id);
        self.frame_links
            .lock()
            .await
            .retain(|_, link| link.parent != *session_id && link.endpoint != *session_id);
        // Higher layers own their endpoint caches. Notify after releasing registry locks so
        // the same lifecycle event releases both core routing and inherited page settings.
        for initializer in self.initializers() {
            initializer.detached(session_id);
        }
    }

    /// A lagged event stream may have dropped a `detachedFromTarget`, leaving an OOPIF session that
    /// is cached but dead. That cached entry is checked before the on-demand attach path, so it
    /// would shadow recovery and keep the frame unreadable. For an OOPIF the frame id is its target
    /// id, so drop every cached OOPIF whose target no longer exists; the next capture re-attaches
    /// the live one on demand.
    async fn reconcile_oopifs_after_lag(&self) {
        let root = ProtocolSession::root(self.cdp.clone());
        let Ok(result) = root.send::<_, Value>("Target.getTargets", json!({})).await else {
            return;
        };
        let live: HashSet<String> = result
            .get("targetInfos")
            .and_then(Value::as_array)
            .map(|infos| {
                infos
                    .iter()
                    .filter_map(|info| {
                        info.get("targetId")
                            .and_then(Value::as_str)
                            .map(String::from)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let dead: Vec<_> = self
            .oopif_sessions
            .lock()
            .await
            .iter()
            .filter(|(frame_id, _)| !live.contains(&frame_id.0))
            .map(|(_, session)| session.clone())
            .collect();
        for session in dead {
            self.on_detached(&session).await;
        }
        // A dropped worker attach event must not leave startup suspended forever. These
        // short-lived recovery sessions only release the debugger wait; frame initialization
        // and its inherited page settings still run through the separate path below.
        for info in result
            .get("targetInfos")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if matches!(
                info.get("type").and_then(Value::as_str),
                Some("worker" | "shared_worker" | "service_worker")
            ) && let Some(id) = info.get("targetId").and_then(Value::as_str)
            {
                self.resume_worker_after_lag(id).await;
            }
        }
        let initializers = self.initializers();
        for initializer in &initializers {
            let _recovered = initializer.recover().await;
        }
        if !initializers.is_empty() {
            let targets = result
                .get("targetInfos")
                .and_then(Value::as_array)
                .into_iter()
                .flatten();
            for target in targets {
                if target.get("type").and_then(Value::as_str) != Some("iframe") {
                    continue;
                }
                let Some(id) = target.get("targetId").and_then(Value::as_str) else {
                    continue;
                };
                let frame = FrameId(id.to_owned());
                if !self.oopif_sessions.lock().await.contains_key(&frame) {
                    let _attached = self.attach_oopif_on_demand(&frame, None, None).await;
                }
            }
        }
    }

    async fn resume_worker_after_lag(&self, target_id: &str) {
        let root = ProtocolSession::root(self.cdp.clone()).at_connection_epoch();
        let Ok(attached) = root
            .send::<_, target::AttachToTargetResult>(
                "Target.attachToTarget",
                json!({"targetId":target_id,"flatten":true}),
            )
            .await
        else {
            return;
        };
        let id = SessionId::from(attached.session_id);
        let session = ProtocolSession::for_session(self.cdp.clone(), id.clone())
            .with_epoch_guard(root.epoch_guard().cloned());
        let _resumed = session
            .send_value("Runtime.runIfWaitingForDebugger", json!({}))
            .await;
        let _detached = root
            .send_value("Target.detachFromTarget", json!({"sessionId":id}))
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameInitializer, FrameRegistry};
    use crate::connection::CdpConnection;
    use crate::{FrameId, PageId, ProtocolSession, SessionId};
    use browseros_cdp::{CdpError, CdpEvent, TargetedEvents};
    use futures_util::future::BoxFuture;
    use serde_json::{Value, json};
    use std::error::Error;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex as StdMutex, PoisonError};
    use tokio::sync::{broadcast, mpsc};

    struct NoopConnection {
        epoch: AtomicU64,
        sent: StdMutex<Vec<String>>,
        targeted: TargetedEvents,
    }

    impl Default for NoopConnection {
        fn default() -> Self {
            Self {
                epoch: AtomicU64::new(1),
                sent: StdMutex::default(),
                targeted: TargetedEvents::default(),
            }
        }
    }

    impl CdpConnection for NoopConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            _params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            self.sent
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(method.to_owned());
            Box::pin(async { Ok(json!({})) })
        }

        fn send_at_epoch<'a>(
            &'a self,
            expected: u64,
            method: &'a str,
            params: Value,
            session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                let actual = self.connection_epoch();
                if actual != expected {
                    return Err(CdpError::ConnectionEpochChanged { expected, actual });
                }
                self.send(method, params, session).await
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async { Ok("{}".to_string()) })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            let (_tx, rx) = broadcast::channel(1);
            rx
        }

        fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
            self.targeted.subscribe(method)
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            self.epoch.load(Ordering::SeqCst)
        }
    }

    /// Attaches to exactly one target id (standing in for an OOPIF whose attach event has not
    /// arrived); any other attachToTarget fails, standing in for a same-process frame with no
    /// separate target.
    struct AttachConnection {
        oopif_target: &'static str,
        attach_session: &'static str,
        targeted: TargetedEvents,
    }

    impl CdpConnection for AttachConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                if method == "Target.attachToTarget" {
                    let target_id = params
                        .get("targetId")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if target_id == self.oopif_target {
                        return Ok(json!({ "sessionId": self.attach_session }));
                    }
                    return Err(CdpError::Protocol {
                        code: -32000,
                        message: format!("No target with given id: {target_id}"),
                    });
                }
                Ok(json!({}))
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async { Ok("{}".to_string()) })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            let (_tx, rx) = broadcast::channel(1);
            rx
        }

        fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
            self.targeted.subscribe(method)
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            1
        }
    }

    #[tokio::test]
    async fn resolve_attaches_an_unregistered_oopif_on_demand() -> Result<(), crate::CoreError> {
        let cdp = Arc::new(AttachConnection {
            oopif_target: "deep-oopif",
            attach_session: "deep-oopif-session",
            targeted: TargetedEvents::default(),
        });
        let registry = FrameRegistry::new(cdp.clone());
        let page_id = PageId(1);
        registry
            .page_sessions
            .lock()
            .await
            .insert(page_id.clone(), SessionId::from("page-session".to_string()));
        let shell =
            ProtocolSession::for_session(cdp.clone(), SessionId::from("shell-session".to_string()));

        let resolved = registry
            .resolve_frame_target(
                page_id,
                Some(FrameId("deep-oopif".to_string())),
                Some(&shell),
                true,
            )
            .await?;

        assert!(resolved.session.same_session(&ProtocolSession::for_session(
            cdp,
            SessionId::from("deep-oopif-session".to_string())
        )));
        assert!(!resolved.session.same_session(&shell));
        assert_eq!(resolved.ax_params, json!({}));
        assert!(resolved.cursor_uses_session_default);
        Ok(())
    }

    #[tokio::test]
    async fn resolve_falls_back_to_same_process_when_no_target_exists()
    -> Result<(), crate::CoreError> {
        let cdp = Arc::new(AttachConnection {
            oopif_target: "unused",
            attach_session: "unused",
            targeted: TargetedEvents::default(),
        });
        let registry = FrameRegistry::new(cdp.clone());
        let page_id = PageId(1);
        registry
            .page_sessions
            .lock()
            .await
            .insert(page_id.clone(), SessionId::from("page-session".to_string()));
        let parent =
            ProtocolSession::for_session(cdp.clone(), SessionId::from("shell-session".to_string()));

        let resolved = registry
            .resolve_frame_target(
                page_id,
                Some(FrameId("plain-child".to_string())),
                Some(&parent),
                true,
            )
            .await?;

        assert!(resolved.session.same_session(&parent));
        assert_eq!(resolved.ax_params, json!({ "frameId": "plain-child" }));
        assert!(!resolved.cursor_uses_session_default);
        Ok(())
    }

    #[tokio::test]
    async fn same_process_child_inherits_and_caches_its_oopif_parent_session()
    -> Result<(), crate::CoreError> {
        let cdp = Arc::new(NoopConnection::default());
        let registry = FrameRegistry::new(cdp.clone());
        let page_id = PageId(1);
        registry
            .page_sessions
            .lock()
            .await
            .insert(page_id.clone(), SessionId::from("page-session".to_string()));
        registry.oopif_sessions.lock().await.insert(
            FrameId("oopif".to_string()),
            SessionId::from("oopif-session".to_string()),
        );
        let oopif = registry
            .resolve_frame_target(
                page_id.clone(),
                Some(FrameId("oopif".to_string())),
                None,
                false,
            )
            .await?;
        let nested_id = FrameId("nested".to_string());

        let nested = registry
            .resolve_frame_target(
                page_id.clone(),
                Some(nested_id.clone()),
                Some(&oopif.session),
                false,
            )
            .await?;
        assert!(nested.session.same_session(&oopif.session));
        assert_eq!(nested.ax_params, json!({"frameId": "nested"}));
        assert!(!nested.cursor_uses_session_default);

        let cached = registry
            .resolve_frame_target(page_id, Some(nested_id), None, false)
            .await?;
        assert!(cached.session.same_session(&oopif.session));
        assert!(!cached.session.same_session(&ProtocolSession::for_session(
            cdp,
            SessionId::from("page-session".to_string())
        )));
        Ok(())
    }

    #[tokio::test]
    async fn reconnect_discards_inherited_sessions_from_the_prior_connection()
    -> Result<(), crate::CoreError> {
        let cdp = Arc::new(NoopConnection::default());
        let registry = FrameRegistry::new(cdp.clone());
        let page_id = PageId(1);
        let child_id = FrameId("child".to_string());
        let first_page_session = ProtocolSession::for_session(
            cdp.clone(),
            SessionId::from("page-session-1".to_string()),
        );
        registry
            .register_page(
                first_page_session,
                page_id.clone(),
                SessionId::from("page-session-1".to_string()),
            )
            .await?;
        let first_parent = ProtocolSession::for_session(
            cdp.clone(),
            SessionId::from("oopif-session-1".to_string()),
        );
        let first_child = registry
            .resolve_frame_target(
                page_id.clone(),
                Some(child_id.clone()),
                Some(&first_parent),
                false,
            )
            .await?;
        assert!(first_child.session.same_session(&first_parent));

        cdp.epoch.store(2, Ordering::SeqCst);
        let second_page_session = ProtocolSession::for_session(
            cdp.clone(),
            SessionId::from("page-session-2".to_string()),
        );
        registry
            .register_page(
                second_page_session.clone(),
                page_id.clone(),
                SessionId::from("page-session-2".to_string()),
            )
            .await?;

        let reconnected_child = registry
            .resolve_frame_target(page_id, Some(child_id), None, false)
            .await?;
        assert!(reconnected_child.session.same_session(&second_page_session));
        assert!(!reconnected_child.session.same_session(&first_parent));
        Ok(())
    }

    /// Reports a fixed set of live targets so reconcile can tell a dropped detach from a live frame.
    struct GetTargetsConnection {
        live_targets: Vec<&'static str>,
        targeted: TargetedEvents,
    }

    impl CdpConnection for GetTargetsConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            _params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                if method == "Target.getTargets" {
                    let infos: Vec<Value> = self
                        .live_targets
                        .iter()
                        .map(|id| json!({ "targetId": id, "type": "iframe" }))
                        .collect();
                    return Ok(json!({ "targetInfos": infos }));
                }
                Ok(json!({}))
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async { Ok("{}".to_string()) })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            let (_tx, rx) = broadcast::channel(1);
            rx
        }

        fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
            self.targeted.subscribe(method)
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            1
        }
    }

    #[tokio::test]
    async fn lag_reconcile_drops_oopifs_whose_target_is_gone() {
        let cdp = Arc::new(GetTargetsConnection {
            live_targets: vec!["live-oopif"],
            targeted: TargetedEvents::default(),
        });
        let registry = FrameRegistry::new(cdp);
        {
            let mut sessions = registry.oopif_sessions.lock().await;
            sessions.insert(
                FrameId("live-oopif".to_string()),
                SessionId::from("live-session".to_string()),
            );
            sessions.insert(
                FrameId("dead-oopif".to_string()),
                SessionId::from("dead-session".to_string()),
            );
        }

        registry.reconcile_oopifs_after_lag().await;

        let sessions = registry.oopif_sessions.lock().await;
        assert!(sessions.contains_key(&FrameId("live-oopif".to_string())));
        assert!(!sessions.contains_key(&FrameId("dead-oopif".to_string())));
    }

    #[tokio::test]
    async fn lag_recovery_releases_workers_and_detaches_its_temporary_session()
    -> Result<(), Box<dyn Error>> {
        let connection = crate::testing::TestConnection::new([
            (
                "Target.getTargets",
                json!({"targetInfos":[{"targetId":"missed-worker","type":"worker"}]}),
            ),
            ("Runtime.runIfWaitingForDebugger", json!({})),
            ("Target.detachFromTarget", json!({})),
        ]);
        let registry = FrameRegistry::new(connection.clone());
        registry.reconcile_oopifs_after_lag().await;
        let calls = connection.calls()?;
        let resumed = calls
            .iter()
            .find(|call| call.method == "Runtime.runIfWaitingForDebugger")
            .ok_or("a missed worker attachment must still release startup")?;
        let detached = calls
            .iter()
            .find(|call| call.method == "Target.detachFromTarget")
            .ok_or("recovery must release its temporary attachment")?;
        assert!(resumed.session.is_some());
        assert_eq!(
            detached.params.get("sessionId"),
            resumed.session.as_ref().map(|id| json!(id)).as_ref()
        );
        Ok(())
    }

    /// Records the configuration write and can replace the socket at the async handoff.
    struct TestInitializer {
        connection: Arc<NoopConnection>,
        invalidate: bool,
    }

    impl FrameInitializer for TestInitializer {
        fn initialize<'a>(
            &'a self,
            session: &'a ProtocolSession,
            parent: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<(), crate::CoreError>> {
            Box::pin(async move {
                assert_eq!(
                    parent,
                    Some(&SessionId::from("root")),
                    "initialization inherits the attaching renderer"
                );
                if self.invalidate {
                    self.connection.epoch.fetch_add(1, Ordering::SeqCst);
                }
                session
                    .send_value("Emulation.setEmulatedMedia", json!({"media":"print"}))
                    .await?;
                Ok(())
            })
        }
    }

    fn attached_child() -> CdpEvent {
        CdpEvent {
            method: "Target.attachedToTarget".to_owned(),
            session_id: Some(SessionId::from("root")),
            params: json!({"sessionId":"child","waitingForDebugger":true,"targetInfo":{
                "targetId":"child-frame","type":"iframe","title":"","url":"about:blank","attached":true,"canAccessOpener":false
            }}),
        }
    }

    #[tokio::test]
    async fn inherited_renderer_configuration_precedes_resume() -> Result<(), Box<dyn Error>> {
        let cdp = Arc::new(NoopConnection::default());
        let registry = FrameRegistry::new(cdp.clone());
        let lease: Arc<dyn FrameInitializer> = Arc::new(TestInitializer {
            connection: Arc::clone(&cdp),
            invalidate: false,
        });
        registry.register_initializer(&lease);
        registry
            .register_page(
                ProtocolSession::for_session(cdp.clone(), SessionId::from("root")),
                PageId::from(1u32),
                SessionId::from("root"),
            )
            .await?;
        cdp.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        let event = attached_child();
        let _: browseros_cdp::target::AttachedToTargetEvent =
            serde_json::from_value(event.params.clone())?;
        registry.handle_event(event).await;
        let sent = cdp.sent.lock().unwrap_or_else(PoisonError::into_inner);
        let position = |method| sent.iter().position(|sent| sent == method);
        assert!(
            position("Target.setAutoAttach") < position("Emulation.setEmulatedMedia"),
            "recursive attachment is armed before configuration: {sent:?}"
        );
        assert!(
            position("Emulation.setEmulatedMedia") < position("Runtime.runIfWaitingForDebugger"),
            "startup scripts only run after inherited preferences: {sent:?}"
        );
        assert!(
            position("Emulation.setEmulatedMedia").is_some(),
            "the live lease must initialize the renderer"
        );
        drop(sent);
        drop(lease);
        assert!(
            registry.initializers().is_empty(),
            "dropping the SDK removes its weak registration"
        );
        Ok(())
    }

    /// Has configuration only for renderers attached under the session named `0`.
    struct ConfiguresOnly(&'static str);

    impl FrameInitializer for ConfiguresOnly {
        fn configures(&self, parent: &SessionId) -> bool {
            parent.as_str() == self.0
        }

        fn initialize<'a>(
            &'a self,
            _session: &'a ProtocolSession,
            _parent: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<(), crate::CoreError>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn only_frames_with_configuration_to_install_start_paused() -> Result<(), Box<dyn Error>>
    {
        let connection = crate::testing::TestConnection::new([]);
        for method in [
            "Target.setAutoAttach",
            "DOM.enable",
            "Accessibility.enable",
            "Runtime.runIfWaitingForDebugger",
        ] {
            connection.always(method, json!({}))?;
        }
        let registry = FrameRegistry::new(connection.clone());
        let lease: Arc<dyn FrameInitializer> = Arc::new(ConfiguresOnly("emulated"));
        registry.register_initializer(&lease);
        for (page, session) in [(1u32, "plain"), (2, "emulated")] {
            registry
                .register_page(
                    ProtocolSession::for_session(connection.clone(), SessionId::from(session)),
                    PageId::from(page),
                    SessionId::from(session),
                )
                .await?;
        }
        for (parent, child) in [("plain", "plain-child"), ("emulated", "emulated-child")] {
            registry
                .handle_event(CdpEvent {
                    method: "Target.attachedToTarget".to_owned(),
                    session_id: Some(SessionId::from(parent)),
                    params: json!({"sessionId": child, "waitingForDebugger": false, "targetInfo": {
                        "targetId": child, "type": "iframe", "title": "", "url": "about:blank",
                        "attached": true, "canAccessOpener": false
                    }}),
                })
                .await;
        }
        let armed: Vec<_> = connection
            .calls()?
            .into_iter()
            .filter(|call| call.method == "Target.setAutoAttach")
            .map(|call| {
                let session = call.session.map(|id| id.as_str().to_owned());
                (session, call.params["waitForDebuggerOnStart"].clone())
            })
            .collect();
        let expected = [
            ("plain", false),
            ("emulated", true),
            ("plain-child", false),
            ("emulated-child", true),
        ]
        .map(|(session, paused)| (Some(session.to_owned()), json!(paused)));
        // A paused frame Chromium detaches mid-navigation is never resumed, so a page with
        // nothing to install must never pause one.
        assert_eq!(armed, expected);
        drop(lease);
        Ok(())
    }

    #[tokio::test]
    async fn an_initialization_from_an_old_socket_never_publishes_on_its_replacement()
    -> Result<(), Box<dyn Error>> {
        let cdp = Arc::new(NoopConnection::default());
        let registry = FrameRegistry::new(cdp.clone());
        let lease: Arc<dyn FrameInitializer> = Arc::new(TestInitializer {
            connection: Arc::clone(&cdp),
            invalidate: true,
        });
        registry.register_initializer(&lease);
        registry
            .register_page(
                ProtocolSession::for_session(cdp.clone(), SessionId::from("root")),
                PageId::from(1u32),
                SessionId::from("root"),
            )
            .await?;
        cdp.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        let event = attached_child();
        let _: browseros_cdp::target::AttachedToTargetEvent =
            serde_json::from_value(event.params.clone())?;
        registry.handle_event(event).await;
        assert_eq!(
            cdp.connection_epoch(),
            2,
            "the initializer must actually replace the socket"
        );
        let sent = cdp.sent.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(
            !sent
                .iter()
                .any(|method| method == "Emulation.setEmulatedMedia"
                    || method == "Runtime.runIfWaitingForDebugger"),
            "old-session setup and resume must both fail before transport publication: {sent:?}"
        );
        Ok(())
    }
}
