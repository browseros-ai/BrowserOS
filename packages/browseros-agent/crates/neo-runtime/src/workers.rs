//! Playwright: `server/chromium/crPage.ts` `FrameSession._initializeWorker`
//! Playwright: `server/page.ts` `Page.addWorker`
//! Playwright: `server/page.ts` `Page.removeWorker`
//!
//! A tab's dedicated workers, as Playwright's page keeps them. The tab's session auto-attaches
//! them once the run asks (its first `worker` listener or `page.workers()`); each becomes a record
//! `worker@<T>:<n>` with its URL, the SDK's `Worker`. A worker attaches held at its start: its
//! runtime is enabled (`worker_world.rs`), then it is resumed, in Playwright's order. Its execution
//! context and `Inspector.workerScriptLoaded` make it ready to evaluate; its session's detach, or
//! its tab's close, closes it.
//!
//! ```text
//!  Target.attachedToTarget{worker}     tab session → record, `worker` event to listeners
//!    Runtime.enable                    worker session (worker_world.rs)
//!    Runtime.runIfWaitingForDebugger   worker session
//!  Runtime.executionContextCreated     worker session → its context
//!  Inspector.workerScriptLoaded        worker session → ready: a WorkerWorld
//!  Target.detachedFromTarget{worker}   tab session → closed, `workerClose` to the SDK
//! ```
//!
//! Deviation: `server/chromium/crPage.ts` `FrameSession._initialize`: `this._client.send('Target.setAutoAttach', { autoAttach: true, waitForDebuggerOnStart: true, flatten: true }),` Playwright's page auto-attaches its workers from its start; Neo's does from the run's first `worker` listener or `page.workers()` (med-pw-eval DECIDED), and then sees the workers already running, so a worker that started and ended before then is never seen.
//! Deviation: `server/chromium/crPage.ts` `FrameSession._initializeWorker`: `session._sendMayFail('Target.setAutoAttach', { autoAttach: true, waitForDebuggerOnStart: true, flatten: true });` Playwright also attaches the workers a worker or an out-of-process frame starts; Neo tracks the tab's own session's dedicated workers only.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use browseros_cdp::{CdpClient, CdpError, CdpEvent, SessionId};
use serde_json::{Value, json};
use tokio::sync::{OnceCell, watch};
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::auto_attach::{self, AttachKind};
use crate::click::PageError;
use crate::events::EventHub;
use crate::worker_world::{self, WorkerWorld};

/// Neo: the guid prefix of a dedicated worker, `worker@<targetId>:<n>`.
pub(crate) const WORKER_PREFIX: &str = "worker@";

/// One tab's dedicated workers.
pub(crate) struct Workers {
    hub: EventHub,
    page_guid: String,
    target: String,
    cdp: CdpClient,
    /// The tab's own session, whose auto-attach reports the workers.
    tab: SessionId,
    /// Set once the tab's session auto-attaches workers.
    tracking: OnceCell<()>,
    next: AtomicU64,
    /// The open workers, in attach order (`page.workers()`).
    live: Mutex<Vec<Arc<Worker>>>,
}

/// One dedicated worker.
pub(crate) struct Worker {
    guid: String,
    url: String,
    session: SessionId,
    cdp: CdpClient,
    /// Cancelled when the worker closes: every call on it then ends closed.
    closed: CancellationToken,
    ready: Mutex<Readiness>,
    world: watch::Sender<Option<Arc<WorkerWorld>>>,
}

/// What a worker still needs before it evaluates.
#[derive(Default)]
struct Readiness {
    context: Option<i64>,
    loaded: bool,
}

impl Workers {
    pub(crate) fn new(
        hub: EventHub,
        page_guid: String,
        target: String,
        cdp: CdpClient,
        tab: SessionId,
    ) -> Self {
        Self {
            hub,
            page_guid,
            target,
            cdp,
            tab,
            tracking: OnceCell::new(),
            next: AtomicU64::new(1),
            live: Mutex::default(),
        }
    }

    /// Turns on the tab's worker auto-attach, once: the workers already running attach at once,
    /// each reported before the setting's reply.
    ///
    /// # Errors
    ///
    /// The tab session's answer, when it refused the setting or is gone.
    pub(crate) async fn track(&self) -> Result<(), CdpError> {
        self.tracking
            .get_or_try_init(|| async {
                auto_attach::want(&self.cdp, &self.tab, AttachKind::Worker).await
            })
            .await
            .map(|_| ())
    }

    /// Applies one CDP event the tab's watcher read, from any session.
    pub(crate) fn apply(&self, event: &CdpEvent) {
        let Some(session) = event.session_id.as_ref() else {
            return;
        };
        if session == &self.tab {
            match event.method.as_str() {
                "Target.attachedToTarget" => self.attached(&event.params),
                "Target.detachedFromTarget" => {
                    if let Some(child) = event.params["sessionId"].as_str() {
                        self.detached(&SessionId::from(child));
                    }
                }
                _ => {}
            }
            return;
        }
        let Some(worker) = self.by_session(session) else {
            return;
        };
        match event.method.as_str() {
            "Runtime.executionContextCreated" => {
                if let Some(id) = event.params["context"]["id"].as_i64() {
                    worker.context_created(id);
                }
            }
            // Playwright: `server/chromium/crPage.ts` `FrameSession._initializeWorker`: `session.on('Inspector.workerScriptLoaded', () => worker.workerScriptLoaded());`
            "Inspector.workerScriptLoaded" => worker.script_loaded(),
            _ => {}
        }
    }

    /// The worker `guid` names, while it is open.
    pub(crate) fn get(&self, guid: &str) -> Option<Arc<Worker>> {
        self.live
            .lock()
            .ok()?
            .iter()
            .find(|worker| worker.guid == guid)
            .cloned()
    }

    /// The open workers' records, in attach order: Playwright's `page.workers()`.
    pub(crate) fn records(&self) -> Vec<Value> {
        self.live
            .lock()
            .map(|live| live.iter().map(|worker| worker.record()).collect())
            .unwrap_or_default()
    }

    /// The tab closed or crashed: every worker closes with it.
    /// Playwright: `server/page.ts` `Page.clearWorkers`
    pub(crate) fn close_all(&self) {
        let workers = self
            .live
            .lock()
            .map(|mut live| std::mem::take(&mut *live))
            .unwrap_or_default();
        for worker in workers {
            self.close(&worker);
        }
    }

    /// Playwright: `server/chromium/crPage.ts` `FrameSession._initializeWorker`
    fn attached(&self, params: &Value) {
        if params["targetInfo"]["type"].as_str() != Some("worker") {
            return;
        }
        let Some(session) = params["sessionId"].as_str().map(SessionId::from) else {
            return;
        };
        let url = params["targetInfo"]["url"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let index = self.next.fetch_add(1, Ordering::Relaxed);
        let worker = Arc::new(Worker::new(
            format!("{WORKER_PREFIX}{}:{index}", self.target),
            url,
            session.clone(),
            self.cdp.clone(),
        ));
        // A worker already running when tracking began attaches without being held: its script
        // loaded before Neo looked, so no `workerScriptLoaded` comes.
        if !params["waitingForDebugger"].as_bool().unwrap_or(false) {
            worker.script_loaded();
        }
        if let Ok(mut live) = self.live.lock() {
            live.push(Arc::clone(&worker));
        }
        let cdp = self.cdp.clone();
        tokio::spawn(async move {
            // Playwright: `server/chromium/crPage.ts` `FrameSession._initializeWorker`: `session._sendMayFail('Runtime.runIfWaitingForDebugger');`
            if let Err(error) = worker_world::enable(&cdp, &session).await {
                debug!(%error, "enabling a worker's runtime failed");
            }
            if let Err(error) = cdp
                .send("Runtime.runIfWaitingForDebugger", json!({}), Some(&session))
                .await
            {
                debug!(%error, "resuming a worker failed");
            }
        });
        // Playwright: `server/page.ts` `Page.addWorker`: `this.emit(Page.Events.Worker, worker);`
        if self.hub.subscribed(&self.page_guid, "worker") {
            self.hub.emit(
                &self.page_guid,
                "worker",
                &json!({ "worker": worker.record() }),
            );
        }
    }

    /// Playwright: `server/page.ts` `Page.removeWorker`
    fn detached(&self, session: &SessionId) {
        let worker = self.live.lock().ok().and_then(|mut live| {
            let index = live.iter().position(|worker| &worker.session == session)?;
            Some(live.remove(index))
        });
        if let Some(worker) = worker {
            self.close(&worker);
        }
    }

    /// Playwright: `server/page.ts` `Worker.didClose`: `this.emit(Worker.Events.Close, this);`
    fn close(&self, worker: &Worker) {
        worker.closed.cancel();
        if self.hub.subscribed(&self.page_guid, "workerClose") {
            self.hub.emit(
                &self.page_guid,
                "workerClose",
                &json!({ "worker": { "guid": worker.guid } }),
            );
        }
    }

    fn by_session(&self, session: &SessionId) -> Option<Arc<Worker>> {
        self.live
            .lock()
            .ok()?
            .iter()
            .find(|worker| &worker.session == session)
            .cloned()
    }
}

impl Worker {
    fn new(guid: String, url: String, session: SessionId, cdp: CdpClient) -> Self {
        Self {
            guid,
            url,
            session,
            cdp,
            closed: CancellationToken::new(),
            ready: Mutex::default(),
            world: watch::channel(None).0,
        }
    }

    pub(crate) fn guid(&self) -> &str {
        &self.guid
    }

    /// The worker's session, whose objects its handles name.
    pub(crate) fn session(&self) -> &SessionId {
        &self.session
    }

    pub(crate) fn cdp(&self) -> &CdpClient {
        &self.cdp
    }

    /// Cancelled when the worker closes.
    pub(crate) fn closed(&self) -> &CancellationToken {
        &self.closed
    }

    /// Playwright's `WorkerInitializer` record, `{guid, initializer: {url}}`.
    pub(crate) fn record(&self) -> Value {
        json!({ "guid": self.guid, "initializer": { "url": self.url } })
    }

    /// The worker's execution context with the `UtilityScript`'s door, once it is ready.
    /// Playwright: `server/page.ts` `Worker._executionContext`: `return await progress.race(this.openScope.race(this._executionContextPromise));`
    ///
    /// # Errors
    ///
    /// `Closed` when the worker closes first.
    pub(crate) async fn world(&self) -> Result<Arc<WorkerWorld>, PageError> {
        let mut ready = self.world.subscribe();
        loop {
            if self.closed.is_cancelled() {
                return Err(PageError::Closed);
            }
            if let Some(world) = ready.borrow_and_update().clone() {
                return Ok(world);
            }
            tokio::select! {
                biased;
                () = self.closed.cancelled() => return Err(PageError::Closed),
                changed = ready.changed() => {
                    if changed.is_err() {
                        return Err(PageError::Closed);
                    }
                }
            }
        }
    }

    /// The worker's evaluation door as it is now, closed or not: what its handles' calls use,
    /// as Playwright's `JSHandle` keeps its execution context.
    pub(crate) fn current_world(&self) -> Option<Arc<WorkerWorld>> {
        self.world.borrow().clone()
    }

    /// Playwright: `server/page.ts` `Worker.createExecutionContext`
    fn context_created(&self, context: i64) {
        if let Ok(mut ready) = self.ready.lock() {
            ready.context = Some(context);
        }
        self.when_ready();
    }

    /// Playwright: `server/page.ts` `Worker.workerScriptLoaded`
    fn script_loaded(&self) {
        if let Ok(mut ready) = self.ready.lock() {
            ready.loaded = true;
        }
        self.when_ready();
    }

    fn when_ready(&self) {
        let context = self
            .ready
            .lock()
            .ok()
            .and_then(|ready| ready.loaded.then_some(ready.context).flatten());
        if let Some(context) = context
            && self.world.borrow().is_none()
        {
            self.world.send_replace(Some(Arc::new(WorkerWorld::new(
                self.cdp.clone(),
                self.session.clone(),
                context,
                self.closed.clone(),
            ))));
        }
    }
}
