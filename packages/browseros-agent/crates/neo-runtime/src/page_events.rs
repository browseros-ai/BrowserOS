//! Neo: what one tab's watcher turns into Playwright's events for the run's script, and the
//! tab's console and page-error history. The watcher (`page.rs` `watch_events`) hands every
//! event of the tab's session to [`PageEvents::apply`], in socket order, after applying it to the
//! page's navigation state:
//!
//! ```text
//!  Runtime.consoleAPICalled, Log.entryAdded ─▶ history ─┬▶ console    on the context, naming the page
//!  Runtime.exceptionThrown ───────────────────▶ history ─┘▶ pageError
//!  Page.javascriptDialogOpening ─▶ dialog record ─▶ dialog (listener) | default answer (none)
//!  Page.javascriptDialogClosed ──▶ dialogClosed
//!  Page.fileChooserOpened, the tab's or an out-of-process frame's ─▶ fileChooser (listener)
//!  detach, Inspector.detached / targetCrashed ─▶ close | crash        on the page
//!  the main frame's DOMContentLoaded / load ──▶ domcontentloaded | load
//!  Page.frameAttached / frameDetached / frameNavigated / navigatedWithinDocument
//!                                       ─▶ frame_events.rs ─▶ frameAttached | frameDetached | frameNavigated
//! ```
//!
//! `Page.setLifecycleEventsEnabled` replays the current document's lifecycle when the watcher
//! starts. That replay is history, except on a page the run's discovery just reported, whose
//! document is new to the run: the page events go live once the watcher has applied it.
//!
//! Console messages and page errors exist only once `Runtime.enable` has been sent, which Neo
//! does only for a run that listens to them or reads their history ([`ConsoleLog`]). The enable
//! replays what V8 kept for the tab's live documents; those messages predate the run's interest
//! and go to the history alone, never to a listener, like messages Playwright's server saw
//! before its client subscribed.
//!
//! An out-of-process frame reports its console on its own CDP session. The tab's watcher takes
//! those events too, once the run has attached that frame's session: the page's console turns
//! on there before the call that attached it goes on ([`PageEvents::child_attached`]), and the
//! session's own replay is history only, as the tab's.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use browseros_cdp::{CdpClient, CdpEvent, SessionId};
use neo_runtime_spec::serve::SerializedError;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;
use tokio::time;
use tracing::debug;

use crate::bridge;
use crate::console::{
    self, ConsoleApiCalled, ConsoleRecord, ExceptionThrown, History, LogEntryAdded, PageErrorRecord,
};
use crate::dialogs::{self, Dialogs};
use crate::events::{CONTEXT_GUID, Emitted, EventHub, Needs};
use crate::frame_events::FrameModel;
use crate::frames::{self, Host};
use crate::network::{Happening, NetworkLog};
use crate::network_events;
use crate::page::{Applied, DialogAnswer};
use crate::run::PageId;
use crate::tabs::{PageEntry, Tabs};

/// Neo: how long an out-of-process frame's first call waits for that frame's console to turn on.
/// A frame frozen by its page's dialog may never answer; the call then goes on without it.
const CHILD_ENABLE_WAIT: Duration = Duration::from_secs(5);

/// The producer side of one tab's session: cheap to clone, shared by the tab's `PageSession`
/// (which serves `consoleMessages` and dialog answers from it) and its watcher.
#[derive(Clone)]
pub(crate) struct PageEvents {
    hub: EventHub,
    page_guid: String,
    target: String,
    cdp: CdpClient,
    session: SessionId,
    /// The out-of-process frame sessions of this tab the run attached, whose console events the
    /// tab's watcher takes as the tab's, with the frame each hosts.
    children: Arc<Mutex<HashMap<SessionId, Option<String>>>>,
    /// The arguments of the console messages the run's listeners received, kept until a script
    /// reads them (`ConsoleMessage.args()`).
    pub(crate) args: Arc<ConsoleArgs>,
    /// The file choosers the page opened while the run listened, until `FileChooser.element()`
    /// makes their input the run's handle.
    pub(crate) choosers: Arc<Choosers>,
    /// Whether the tab's session intercepts its file choosers now. Every switch takes this lock
    /// and switches to the run's latest `filechooser` subscription, so the last one to run
    /// leaves the latest, whatever order their tasks ran in ([`PageEvents::reconcile_interception`]).
    intercepting: Arc<AsyncMutex<bool>>,
    /// Each page-world execution context of the tab's sessions, by session and id, with the
    /// frame it runs in, from `Runtime.executionContextCreated` (on once the run listens to the
    /// console): the frame a console message's arguments live in.
    contexts: Arc<Mutex<HashMap<(SessionId, i64), String>>>,
    pub(crate) console: Arc<ConsoleLog>,
    pub(crate) dialogs: Arc<Dialogs>,
    /// The tab's frames, as the run's frame events see them.
    frames: Arc<Mutex<FrameModel>>,
    /// Whether the main frame's lifecycle reaches listeners: once the watcher has applied the
    /// lifecycle replay of its start ([`Self::lifecycle_replayed`]), or from the start on a page
    /// the run's discovery reported.
    lifecycle_live: Arc<AtomicBool>,
}

/// CDP's `Page.fileChooserOpened` params.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileChooserOpened {
    frame_id: String,
    mode: String,
    #[serde(default)]
    backend_node_id: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DialogOpening {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    default_prompt: Option<String>,
}

impl PageEvents {
    /// `discovered`: the run's discovery reported the tab as new, so its first lifecycle replay
    /// is the page's own new life and reaches listeners.
    pub(crate) fn new(
        hub: EventHub,
        target: &str,
        cdp: CdpClient,
        session: SessionId,
        discovered: bool,
    ) -> Self {
        Self {
            hub,
            page_guid: bridge::page_guid(&PageId::new(target.to_string())),
            target: target.to_string(),
            cdp,
            session,
            children: Arc::default(),
            args: Arc::default(),
            choosers: Arc::default(),
            intercepting: Arc::default(),
            contexts: Arc::default(),
            console: Arc::default(),
            dialogs: Arc::default(),
            frames: Arc::new(Mutex::new(FrameModel::new(target))),
            lifecycle_live: Arc::new(AtomicBool::new(discovered)),
        }
    }

    /// The watcher's start read the tab's frame tree (`None` when the read failed): the frame
    /// model takes the frames no event has reported yet.
    pub(crate) fn seeded(&self, tree: Option<&Value>) {
        let Some(tree) = tree.and_then(|tree| frames::local_tree(tree).ok()) else {
            return;
        };
        if let Ok(mut model) = self.frames.lock() {
            model.seed(&tree);
        }
    }

    /// The watcher has applied the lifecycle replay of its start: the main frame's lifecycle
    /// events from now on are live.
    pub(crate) fn lifecycle_replayed(&self) {
        self.lifecycle_live.store(true, Ordering::Release);
    }

    /// An out-of-process frame of this tab got a session: from now on its console events are
    /// the tab's, and when the run already wants the tab's console, it turns on there before the
    /// call that attached the frame goes on, so that call's messages are live, not replayed.
    pub(crate) async fn child_attached(&self, host: &Arc<Host>) {
        if let Ok(mut children) = self.children.lock() {
            children.insert(
                host.session_id().clone(),
                host.target_id().map(str::to_string),
            );
        }
        if self.console.requested(&self.session) {
            self.enable_child(host).await;
        }
        // Under the switches' lock, so a switch-off running meanwhile cannot miss this frame.
        let _switching = self.intercepting.lock().await;
        if self.hub.subscribed(&self.page_guid, "filechooser") {
            self.intercept_child(host.session_id(), true).await;
        }
    }

    /// Switches the tab's file chooser interception, and that of each out-of-process frame of it
    /// the run attached, to the run's latest `filechooser` subscription, if it changed. Takes the
    /// switches' lock for the whole switch. The tab's session must have `Page.enable` on
    /// (`PageSession::prepare`); a failure is logged and the next switch tries again.
    pub(crate) async fn reconcile_interception(&self) {
        let mut intercepting = self.intercepting.lock().await;
        let wanted = self.hub.subscribed(&self.page_guid, "filechooser");
        if *intercepting == wanted {
            return;
        }
        let switched = time::timeout(
            CHILD_ENABLE_WAIT,
            self.cdp.send(
                "Page.setInterceptFileChooserDialog",
                json!({ "enabled": wanted }),
                Some(&self.session),
            ),
        )
        .await;
        match switched {
            Ok(Ok(_)) => *intercepting = wanted,
            Ok(Err(error)) => {
                debug!(%error, "the page's file chooser interception did not switch");
            }
            Err(_) => debug!("the page's file chooser interception did not switch in time"),
        }
        self.intercept_children(wanted).await;
    }

    /// Switches the file chooser interception of every out-of-process frame session of this tab
    /// the run attached, under the switches' lock ([`PageEvents::reconcile_interception`]).
    async fn intercept_children(&self, enabled: bool) {
        let sessions: Vec<SessionId> = self
            .children
            .lock()
            .map(|children| children.keys().cloned().collect())
            .unwrap_or_default();
        for session in sessions {
            self.intercept_child(&session, enabled).await;
        }
    }

    /// Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `promises.push(this._updateFileChooserInterception(true));`
    ///
    /// One out-of-process frame's interception: Chromium reports a frame's file chooser only to
    /// a session intercepting that frame's own, and a session intercepts only once its `Page`
    /// domain is on (measured). Bounded by [`CHILD_ENABLE_WAIT`]; a failure is logged.
    async fn intercept_child(&self, session: &SessionId, enabled: bool) {
        let switch = async {
            if enabled {
                self.cdp
                    .send("Page.enable", json!({}), Some(session))
                    .await?;
            }
            self.cdp
                .send(
                    "Page.setInterceptFileChooserDialog",
                    json!({ "enabled": enabled }),
                    Some(session),
                )
                .await
        };
        match time::timeout(CHILD_ENABLE_WAIT, switch).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                debug!(%session, %error, "a frame's file chooser interception did not switch");
            }
            Err(_) => {
                debug!(%session, "a frame's file chooser interception did not switch in time");
            }
        }
    }

    /// `Runtime.enable` and `Log.enable` on an out-of-process frame's session, its replay applied
    /// as history before the session goes live, as `PageSession::enable_runtime` does for the
    /// tab's own session. Bounded by [`CHILD_ENABLE_WAIT`]; a failure is logged.
    pub(crate) async fn enable_child(&self, host: &Arc<Host>) {
        let session = host.session_id().clone();
        if self.console.requested(&session) {
            return;
        }
        self.console.request(&session);
        let enable = async {
            host.send("Runtime.enable", json!({})).await?;
            host.send("Log.enable", json!({})).await
        };
        match time::timeout(CHILD_ENABLE_WAIT, enable).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => debug!(%session, %error, "a frame's console did not turn on"),
            Err(_) => debug!(%session, "a frame's console did not turn on in time"),
        }
        self.hub.replay_barrier().await;
        self.console.go_live(&session);
    }

    pub(crate) fn hub(&self) -> &EventHub {
        &self.hub
    }

    /// The run's network events for what the tab's network log just applied
    /// (`network_events.rs`).
    pub(crate) fn network(&self, log: &NetworkLog, happenings: &[Happening]) {
        if !happenings.is_empty() {
            network_events::emit(&self.hub, &self.page_guid, log, happenings);
        }
    }

    /// The tab's target id.
    pub(crate) fn target(&self) -> &str {
        &self.target
    }

    /// Whether `frame` is one of the tab's frames: its main frame, a frame its model knows, or
    /// an out-of-process frame whose session the run attached.
    pub(crate) fn has_frame(&self, frame: &str) -> bool {
        frame == self.target
            || self.frames.lock().is_ok_and(|model| model.contains(frame))
            || self.children.lock().is_ok_and(|children| {
                children
                    .values()
                    .any(|hosted| hosted.as_deref() == Some(frame))
            })
    }

    /// Applies one event the watcher already applied to the page's state (`applied`), and
    /// returns the dialog answer the watcher still owes: `applied.dialog` unless a listener now
    /// owns the dialog.
    pub(crate) fn apply(&self, event: &CdpEvent, applied: &Applied) -> Option<DialogAnswer> {
        let Some(session) = event.session_id.as_ref() else {
            return applied.dialog;
        };
        self.track_context(event, session);
        if session != &self.session {
            // An out-of-process frame of this tab: its console, frames and file choosers.
            // Chromium reports its dialogs on the tab's own session.
            let hosted = self
                .children
                .lock()
                .ok()
                .and_then(|children| children.get(session).cloned());
            if let Some(hosted) = hosted {
                if event.method == "Page.fileChooserOpened" {
                    self.file_chooser(&event.params, hosted);
                }
                self.console_event(event, session);
                self.frame_event(event);
            }
            return applied.dialog;
        }
        let mut answer = applied.dialog;
        match event.method.as_str() {
            "Page.javascriptDialogOpening" => answer = self.dialog_opening(&event.params, answer),
            "Page.javascriptDialogClosed" => self.dialog_closed(),
            "Page.fileChooserOpened" => self.file_chooser(&event.params, None),
            _ => self.console_event(event, session),
        }
        if applied.new_document {
            self.console.mark_navigation();
        }
        self.frame_event(event);
        self.lifecycle(applied);
        answer
    }

    /// Playwright: `server/chromium/crPage.ts` `FrameSession._onExecutionContextCreated`: `const frame = contextPayload.auxData ? this._page.frameManager.frame(contextPayload.auxData.frameId) : null;`
    ///
    /// The page-world execution contexts of the tab's own session and of the out-of-process
    /// frame sessions the watcher takes, each with its frame.
    fn track_context(&self, event: &CdpEvent, session: &SessionId) {
        let Ok(mut contexts) = self.contexts.lock() else {
            return;
        };
        match event.method.as_str() {
            "Runtime.executionContextCreated" => {
                let context = &event.params["context"];
                let page_world = context["auxData"]["isDefault"].as_bool() == Some(true);
                if let (Some(id), Some(frame), true) = (
                    context["id"].as_i64(),
                    context["auxData"]["frameId"].as_str(),
                    page_world,
                ) {
                    contexts.insert((session.clone(), id), frame.to_string());
                }
            }
            "Runtime.executionContextDestroyed" => {
                if let Some(id) = event.params["executionContextId"].as_i64() {
                    contexts.remove(&(session.clone(), id));
                }
            }
            "Runtime.executionContextsCleared" => {
                contexts.retain(|(of, _), _| of != session);
            }
            _ => {}
        }
    }

    /// A frame change of the tab, for the page's and the context's frame listeners.
    fn frame_event(&self, event: &CdpEvent) {
        if !matches!(
            event.method.as_str(),
            "Page.frameAttached"
                | "Page.frameDetached"
                | "Page.frameNavigated"
                | "Page.navigatedWithinDocument"
        ) {
            return;
        }
        let Ok(changes) = self.frames.lock().map(|mut model| model.apply(event)) else {
            return;
        };
        for change in changes {
            if self
                .hub
                .wants(&self.page_guid, change.listener, change.listener)
            {
                self.hub
                    .emit(&self.page_guid, change.method, &change.params);
            }
        }
    }

    /// Playwright: `client/frame.ts` `Frame.constructor`: `if (!this._parentFrame && event.add === 'load' && this._page) {`
    /// The main frame's `domcontentloaded` and `load`; the context's `pageload` comes with `load`.
    fn lifecycle(&self, applied: &Applied) {
        if !self.lifecycle_live.load(Ordering::Acquire) {
            return;
        }
        if applied.fired.dom_content_loaded
            && self.hub.subscribed(&self.page_guid, "domcontentloaded")
        {
            self.hub
                .emit(&self.page_guid, "domcontentloaded", &json!({}));
        }
        if applied.fired.load && self.hub.wants(&self.page_guid, "load", "pageload") {
            self.hub.emit(&self.page_guid, "load", &json!({}));
        }
    }

    /// The session ended: the tab closed or crashed. Emits `close` or `crash` on the page; the
    /// SDK's `close` also runs the context's `pageclose` listeners.
    pub(crate) fn closing(&self, event: Option<&CdpEvent>) {
        let crashed = event.is_some_and(|event| event.method == "Inspector.targetCrashed");
        let wanted = if crashed {
            self.hub.subscribed(&self.page_guid, "crash")
        } else {
            self.hub.wants(&self.page_guid, "close", "pageclose")
        };
        if wanted {
            let name = if crashed { "crash" } else { "close" };
            self.hub.emit(&self.page_guid, name, &json!({}));
        }
    }

    /// The watcher lost events: a dialog may have closed unseen, and it dismisses any.
    pub(crate) fn lagged(&self) {
        self.dialogs.forget();
    }

    /// A console message, log entry or page error of `session`, the tab's or a frame's.
    fn console_event(&self, event: &CdpEvent, session: &SessionId) {
        match event.method.as_str() {
            "Runtime.consoleAPICalled" => self.console_api(&event.params, session),
            "Log.entryAdded" => self.log_entry(&event.params, session),
            "Runtime.exceptionThrown" => self.exception(&event.params, session),
            _ => {}
        }
    }

    fn console_api(&self, params: &Value, session: &SessionId) {
        let Ok(event) = serde_json::from_value::<ConsoleApiCalled>(params.clone()) else {
            return;
        };
        let Some(record) = console::from_console_api(&event) else {
            return;
        };
        let args = params["args"].as_array().cloned().unwrap_or_default();
        // Playwright: `server/chromium/crPage.ts` `FrameSession._onConsoleAPI`: `const context = this._contextIdToContext.get(event.executionContextId);`
        let frame = params["executionContextId"].as_i64().and_then(|id| {
            self.contexts
                .lock()
                .ok()
                .and_then(|contexts| contexts.get(&(session.clone(), id)).cloned())
        });
        self.message(record, event.timestamp, session, args, frame);
    }

    fn log_entry(&self, params: &Value, session: &SessionId) {
        let Ok(LogEntryAdded { entry }) = serde_json::from_value::<LogEntryAdded>(params.clone())
        else {
            return;
        };
        // Playwright: `server/chromium/crPage.ts` `FrameSession._onLogEntryAdded`: `args.map(arg => releaseObject(this._client, arg.objectId!));`
        for object in console::log_entry_object_ids(&entry) {
            let cdp = self.cdp.clone();
            let session = session.clone();
            tokio::spawn(async move {
                let released = cdp
                    .send(
                        "Runtime.releaseObject",
                        json!({ "objectId": object }),
                        Some(&session),
                    )
                    .await;
                if let Err(error) = released {
                    debug!(%session, %error, "releasing a log entry's argument failed");
                }
            });
        }
        // Playwright: `server/chromium/crPage.ts` `FrameSession._onLogEntryAdded`: `this._page.addConsoleMessage(null, level, [], location, text, event.entry.timestamp);`
        if let Some(record) = console::from_log_entry(&entry) {
            self.message(record, entry.timestamp, session, Vec::new(), None);
        }
    }

    /// A console message, or a log entry with no `args`. A listened message with arguments
    /// names them by an id (`argsId`), which `ConsoleMessage.args()` reads (`Tabs::console_args`).
    /// `frame`: the frame the message's arguments live in, when the watcher knows its context.
    fn message(
        &self,
        record: ConsoleRecord,
        timestamp: f64,
        session: &SessionId,
        args: Vec<Value>,
        frame: Option<String>,
    ) {
        let params = self
            .hub
            .wants(&self.page_guid, "console", "console")
            .then(|| serde_json::to_value(&record).ok())
            .flatten();
        let live = self.console.push_message(record, timestamp, session);
        if let Some(mut params) = params.filter(|_| live) {
            params["page"] = json!({ "guid": self.page_guid });
            if !args.is_empty() {
                let host = if session == &self.session {
                    None
                } else {
                    self.children
                        .lock()
                        .ok()
                        .and_then(|children| children.get(session).cloned().flatten())
                };
                let id = self.args.keep(frame, host, args);
                params["argsId"] = json!(format!("console@{}:{id}", self.target));
            }
            self.hub.emit(CONTEXT_GUID, "console", &params);
        }
    }

    fn exception(&self, params: &Value, session: &SessionId) {
        let Ok(event) = serde_json::from_value::<ExceptionThrown>(params.clone()) else {
            return;
        };
        let record = console::from_exception(&event);
        // Playwright: `server/dispatchers/browserContextDispatcher.ts` `BrowserContextDispatcher.constructor`: `this._dispatchEvent('pageError', {`
        let params = self
            .hub
            .wants(&self.page_guid, "pageerror", "weberror")
            .then(|| {
                json!({
                    "error": record.error,
                    "page": { "guid": self.page_guid },
                    "location": {
                        "url": record.location.url,
                        "line": record.location.line_number,
                        "column": record.location.column_number,
                    },
                })
            });
        let live = self.console.push_error(record, event.timestamp, session);
        if let Some(params) = params.filter(|_| live) {
            self.hub.emit(CONTEXT_GUID, "pageError", &params);
        }
    }

    fn dialog_opening(&self, params: &Value, answer: Option<DialogAnswer>) -> Option<DialogAnswer> {
        let Ok(opening) = serde_json::from_value::<DialogOpening>(params.clone()) else {
            return answer;
        };
        let Some((index, record)) = self.dialogs.opened(
            &self.target,
            &self.page_guid,
            &opening.kind,
            &opening.message,
            opening.default_prompt.as_deref().unwrap_or(""),
        ) else {
            return answer;
        };
        if self.hub.wants(&self.page_guid, "dialog", "dialog")
            && self
                .hub
                .emit(CONTEXT_GUID, "dialog", &json!({ "dialog": record }))
                == Emitted::Queued
        {
            // The script owns it now: its listener answers, or the SDK gives the default when
            // the listener is gone by the time the event runs.
            self.dialogs.emitted(index);
            return None;
        }
        self.dialogs.answered_by_default(index);
        Some(answer.unwrap_or_else(|| dialogs::default_answer(&opening.kind)))
    }

    /// Playwright: `server/page.ts` `Page._onFileChooserOpened`: `if (!this.listenerCount(Page.Events.FileChooser)) {`
    /// A file chooser the page opened while the run intercepts them: Playwright's `fileChooser`
    /// event on the page when the run listens, else nothing (the chooser stays closed). `host`:
    /// the out-of-process frame whose session reported it, `None` for the tab's own.
    fn file_chooser(&self, params: &Value, host: Option<String>) {
        let Ok(opened) = serde_json::from_value::<FileChooserOpened>(params.clone()) else {
            return;
        };
        let Some(backend) = opened.backend_node_id else {
            return;
        };
        if !self.hub.subscribed(&self.page_guid, "filechooser") {
            return;
        }
        let id = self.choosers.keep(opened.frame_id, backend, host);
        self.hub.emit(
            &self.page_guid,
            "fileChooser",
            &json!({
                "chooser": format!("filechooser@{}:{id}", self.target),
                "isMultiple": opened.mode == "selectMultiple",
            }),
        );
    }

    fn dialog_closed(&self) {
        let Some(record) = self.dialogs.closed() else {
            return;
        };
        if self
            .hub
            .wants(&self.page_guid, "dialogclosed", "dialogclosed")
        {
            self.hub
                .emit(CONTEXT_GUID, "dialogClosed", &json!({ "dialog": record }));
        }
    }

    /// Frees the objects the tab's console events hold in the page (V8's `console` object
    /// group, on the tab's session and each frame session's), once nothing listens to them and
    /// every argument a listener received was read (a read made the run's handles of them).
    pub(crate) fn release_console_objects(&self) {
        // Arguments a listener received and the script has not read yet live in this group.
        if self.args.unread() {
            return;
        }
        let mut sessions = vec![self.session.clone()];
        if let Ok(children) = self.children.lock() {
            sessions.extend(children.keys().cloned());
        }
        for session in sessions {
            if !self.console.requested(&session) {
                continue;
            }
            let cdp = self.cdp.clone();
            tokio::spawn(async move {
                let released = cdp
                    .send(
                        "Runtime.releaseObjectGroup",
                        json!({ "objectGroup": "console" }),
                        Some(&session),
                    )
                    .await;
                if let Err(error) = released {
                    debug!(%session, %error, "releasing the console's objects failed");
                }
            });
        }
    }

    /// Whether no listener of the run wants this page's console messages or page errors.
    pub(crate) fn console_unwanted(&self) -> bool {
        !self.hub.wants(&self.page_guid, "console", "console")
            && !self.hub.wants(&self.page_guid, "pageerror", "weberror")
    }
}

/// One tab's console messages and page errors as Playwright's server keeps them
/// (`console::History`), and where each session's `Runtime.enable` replay ends: the tab's own
/// session and each out-of-process frame's.
///
/// A replay comes before its enable's reply; [`ConsoleLog::go_live`] runs once the watcher has
/// applied everything up to that reply. Until then a message counts as live only when V8 stamped
/// it at or after the moment the enable was requested: it was logged after the subscription that
/// asked for the enable (or the history read), so a listener must see it.
#[derive(Default)]
pub(crate) struct ConsoleLog(Mutex<ConsoleState>);

#[derive(Default)]
struct ConsoleState {
    history: History,
    /// Per session, where its `Runtime.enable` replay stands.
    replays: HashMap<SessionId, Replay>,
}

/// One session's `Runtime.enable`.
#[derive(Clone, Copy)]
struct Replay {
    /// Epoch milliseconds when the enable was requested, as V8 stamps messages.
    requested_at: f64,
    live: bool,
}

impl ConsoleState {
    fn live(&self, session: &SessionId, timestamp: f64) -> bool {
        self.replays
            .get(session)
            .is_some_and(|replay| replay.live || timestamp >= replay.requested_at)
    }
}

impl ConsoleLog {
    /// Records that `Runtime.enable` is about to be sent on `session`.
    pub(crate) fn request(&self, session: &SessionId) {
        if let Ok(mut state) = self.0.lock() {
            state.replays.entry(session.clone()).or_insert(Replay {
                requested_at: now_ms(),
                live: false,
            });
        }
    }

    /// Whether `Runtime.enable` was requested on `session`.
    pub(crate) fn requested(&self, session: &SessionId) -> bool {
        self.0
            .lock()
            .is_ok_and(|state| state.replays.contains_key(session))
    }

    /// `session`'s replay has been applied; every later message of it is live.
    pub(crate) fn go_live(&self, session: &SessionId) {
        if let Ok(mut state) = self.0.lock()
            && let Some(replay) = state.replays.get_mut(session)
        {
            replay.live = true;
        }
    }

    fn push_message(&self, record: ConsoleRecord, timestamp: f64, session: &SessionId) -> bool {
        self.0.lock().is_ok_and(|mut state| {
            state.history.push_message(record);
            state.live(session, timestamp)
        })
    }

    fn push_error(&self, record: PageErrorRecord, timestamp: f64, session: &SessionId) -> bool {
        self.0.lock().is_ok_and(|mut state| {
            state.history.push_error(record);
            state.live(session, timestamp)
        })
    }

    fn mark_navigation(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.history.mark_navigation();
        }
    }

    pub(crate) fn messages(&self, all: bool) -> Vec<ConsoleRecord> {
        self.0
            .lock()
            .map(|state| state.history.messages(all))
            .unwrap_or_default()
    }

    pub(crate) fn errors(&self, all: bool) -> Vec<SerializedError> {
        self.0
            .lock()
            .map(|state| state.history.errors(all))
            .unwrap_or_default()
    }

    pub(crate) fn clear_messages(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.history.clear_messages();
        }
    }

    pub(crate) fn clear_errors(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.history.clear_errors();
        }
    }
}

/// Deviation: `client/consoleMessage.ts` `ConsoleMessage.args`: `return this._event.args.map(JSHandle.from);`
/// Playwright's server makes every argument a handle as the message arrives. Neo keeps the
/// arguments of a page's last this-many listened messages and makes their handles when the script
/// first reads them; an older message's `args()` throws an Error naming this cap.
pub(crate) const MAX_KEPT_ARGS: usize = 1000;

/// The arguments of the console messages a page's listeners received, as CDP sent them, until
/// `ConsoleMessage.args()` makes them the run's handles (`Tabs::console_args`).
#[derive(Default)]
pub(crate) struct ConsoleArgs(Mutex<KeptArgs>);

#[derive(Default)]
struct KeptArgs {
    next: u64,
    kept: VecDeque<Kept>,
}

/// One message's arguments: the out-of-process frame whose session they belong to (`None`: the
/// tab's own session, whose main frame owns them), CDP's `RemoteObject`s, and once read, the
/// handle records made from them.
#[derive(Clone, Debug)]
pub(crate) struct Kept {
    pub(crate) id: u64,
    /// The frame the arguments live in, when the watcher knew the message's context.
    pub(crate) frame: Option<String>,
    /// The out-of-process frame whose session reported the message; `None` for the tab's own.
    pub(crate) host: Option<String>,
    pub(crate) objects: Vec<Value>,
    pub(crate) handles: Option<Value>,
}

impl ConsoleArgs {
    /// Keeps one message's arguments and returns its id, dropping the oldest past the cap.
    fn keep(&self, frame: Option<String>, host: Option<String>, objects: Vec<Value>) -> u64 {
        let Ok(mut state) = self.0.lock() else {
            return 0;
        };
        state.next += 1;
        let id = state.next;
        state.kept.push_back(Kept {
            id,
            frame,
            host,
            objects,
            handles: None,
        });
        while state.kept.len() > MAX_KEPT_ARGS {
            state.kept.pop_front();
        }
        id
    }

    /// Message `id`'s arguments, `None` once the cap dropped them.
    pub(crate) fn get(&self, id: u64) -> Option<Kept> {
        let state = self.0.lock().ok()?;
        state.kept.iter().find(|kept| kept.id == id).cloned()
    }

    /// Records the handles made from message `id`'s arguments: a later `args()` gets them again.
    pub(crate) fn made(&self, id: u64, handles: Value) {
        if let Ok(mut state) = self.0.lock()
            && let Some(kept) = state.kept.iter_mut().find(|kept| kept.id == id)
        {
            kept.handles = Some(handles);
        }
    }

    /// Whether some kept arguments still have no handles: they live in V8's `console` group.
    fn unread(&self) -> bool {
        self.0
            .lock()
            .is_ok_and(|state| state.kept.iter().any(|kept| kept.handles.is_none()))
    }
}

/// Neo: how many file choosers a page keeps for `FileChooser.element()`; an older one's
/// `element()` throws an Error naming this cap.
pub(crate) const MAX_KEPT_CHOOSERS: usize = 100;

/// The file choosers a page opened while the run listened: the frame, the session that reported
/// it and the input's backend node, and once read, the `ElementHandle` record made of it
/// (`Tabs::file_chooser_element`).
#[derive(Default)]
pub(crate) struct Choosers(Mutex<KeptChoosers>);

#[derive(Default)]
struct KeptChoosers {
    next: u64,
    kept: VecDeque<Chooser>,
}

#[derive(Clone, Debug)]
pub(crate) struct Chooser {
    pub(crate) id: u64,
    pub(crate) frame: String,
    /// The out-of-process frame whose session reported it, whose DOM the backend node is of;
    /// `None` for the tab's own session.
    pub(crate) host: Option<String>,
    pub(crate) backend: i64,
    pub(crate) handle: Option<Value>,
}

impl Choosers {
    fn keep(&self, frame: String, backend: i64, host: Option<String>) -> u64 {
        let Ok(mut state) = self.0.lock() else {
            return 0;
        };
        state.next += 1;
        let id = state.next;
        state.kept.push_back(Chooser {
            id,
            frame,
            host,
            backend,
            handle: None,
        });
        while state.kept.len() > MAX_KEPT_CHOOSERS {
            state.kept.pop_front();
        }
        id
    }

    pub(crate) fn get(&self, id: u64) -> Option<Chooser> {
        let state = self.0.lock().ok()?;
        state.kept.iter().find(|chooser| chooser.id == id).cloned()
    }

    pub(crate) fn made(&self, id: u64, handle: Value) {
        if let Ok(mut state) = self.0.lock()
            && let Some(chooser) = state.kept.iter_mut().find(|chooser| chooser.id == id)
        {
            chooser.handle = Some(handle);
        }
    }
}

/// Platform: V8 stamps console messages and exceptions with `Runtime.Timestamp`, milliseconds
/// since the epoch on the browser's clock, which is this machine's for a local BrowserOS.
fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64() * 1000.0)
}

/// Playwright: `server/page.ts` `Page._setFileChooserInterceptedBy`: `if (wasIntercepted !== this.fileChooserIntercepted())`
///
/// Brings the page's file chooser interception (`Page.setInterceptFileChooserDialog`, which its
/// session keeps until it detaches) to the run's latest `filechooser` subscription, on a run task
/// the page's next call waits for: a click right after `page.on('filechooser')` must find the
/// chooser intercepted, and one right after the last listener left must find it not. The SDK
/// subscribes on the first listener and unsubscribes after the last, so this switches as
/// Playwright's does, while the client listens. Each subscription change starts one; they take
/// turns ([`PageEvents::reconcile_interception`]), so a slow switch-on cannot land after the
/// switch-off that followed it.
pub(crate) fn intercept_file_chooser(tabs: &Tabs, entry: Arc<PageEntry>) {
    let enabling = tabs.events().begin_enable(entry.id.as_str());
    let run_tabs = tabs.clone();
    tabs.spawn(async move {
        let _enabling = enabling;
        let session = match run_tabs.session(&entry).await {
            Ok(session) => session,
            Err(error) => {
                debug!(?error, "a file chooser subscription's page has no session");
                return;
            }
        };
        if let Err(error) = session.prepare().await {
            debug!(%error, "a file chooser subscription could not prepare its page");
            return;
        }
        session.events.reconcile_interception().await;
    });
}

/// Switches on what `needs` asks for in `entry`'s session, on a run task, and makes the page's
/// next calls wait for it ([`EventHub::ready`]). Attaches the session if the page has none.
/// A failure is logged: the subscription stays, and the page's calls fail on their own if the
/// page has gone.
pub(crate) fn enable(tabs: &Tabs, entry: Arc<PageEntry>, needs: Needs) {
    let enabling = tabs.events().begin_enable(entry.id.as_str());
    let run_tabs = tabs.clone();
    tabs.spawn(async move {
        let _enabling = enabling;
        let session = match run_tabs.session(&entry).await {
            Ok(session) => session,
            Err(error) => {
                debug!(?error, "a subscription's page has no session");
                return;
            }
        };
        if needs.watcher
            && let Err(error) = session.prepare().await
        {
            debug!(%error, "a subscription could not prepare its page");
            return;
        }
        if needs.runtime
            && let Err(error) = session.enable_runtime().await
        {
            debug!(?error, "a subscription could not enable the page's console");
        }
        if needs.network
            && let Err(error) = session.enable_network().await
        {
            debug!(?error, "a subscription could not enable the page's network");
        }
    });
}

#[cfg(test)]
mod tests {
    use neo_runtime_spec::serve::PageConsoleMessagesResultMessagesItemLocation;

    use super::*;

    fn record(text: &str) -> ConsoleRecord {
        ConsoleRecord {
            r#type: "log".to_string(),
            text: text.to_string(),
            args: Vec::new(),
            location: PageConsoleMessagesResultMessagesItemLocation {
                url: String::new(),
                line_number: 0,
                column_number: 0,
            },
            timestamp: 0.0,
        }
    }

    #[test]
    fn tests_that_a_replay_is_history_until_its_session_goes_live() {
        let log = ConsoleLog::default();
        let tab = SessionId::from("tab");
        let frame = SessionId::from("frame");
        assert!(
            !log.push_message(record("never enabled"), 1.0, &tab),
            "no enable: history only"
        );
        log.request(&tab);
        let requested_at = log
            .0
            .lock()
            .map(|state| state.replays[&tab].requested_at)
            .unwrap();
        assert!(
            !log.push_message(record("before"), requested_at - 1.0, &tab),
            "logged before the request: a replay"
        );
        assert!(
            log.push_message(record("after"), requested_at, &tab),
            "logged after the request, replayed or not: live"
        );
        assert!(
            !log.push_message(record("frame"), requested_at + 1.0, &frame),
            "another session has its own replay"
        );
        log.go_live(&tab);
        assert!(
            log.push_message(record("late stamp"), 0.0, &tab),
            "after the replay barrier everything is live, whatever its stamp"
        );
        assert_eq!(
            log.messages(true)
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>(),
            ["never enabled", "before", "after", "frame", "late stamp"],
            "the history keeps every message, live or not"
        );
    }
    #[tokio::test]
    async fn tests_that_interception_ends_at_the_latest_subscription_whatever_order_switches_run() {
        use futures_util::future;

        use crate::fake_browser::{FakeBrowser, Replies};

        let browser = FakeBrowser::start(Replies::default()).await;
        let hub = EventHub::default();
        let events = PageEvents::new(
            hub.clone(),
            "T",
            browser.cdp().await,
            SessionId::from("s"),
            false,
        );
        let switches = || -> Vec<Value> {
            browser
                .exchanges()
                .into_iter()
                .filter(|exchange| exchange.method() == "Page.setInterceptFileChooserDialog")
                .map(|exchange| exchange.command["params"]["enabled"].clone())
                .collect()
        };
        // A listener came and went (an aborted wait) before either switch ran: neither turns it on.
        hub.subscribe("page@T", "filechooser", true);
        hub.subscribe("page@T", "filechooser", false);
        future::join(
            events.reconcile_interception(),
            events.reconcile_interception(),
        )
        .await;
        assert_eq!(switches(), Vec::<Value>::new(), "nothing to switch");
        hub.subscribe("page@T", "filechooser", true);
        events.reconcile_interception().await;
        // The off follows a late on: both switches read the latest subscription, off.
        hub.subscribe("page@T", "filechooser", false);
        future::join(
            events.reconcile_interception(),
            events.reconcile_interception(),
        )
        .await;
        assert_eq!(
            switches(),
            [json!(true), json!(false)],
            "on, then off, once each"
        );
        assert!(!*events.intercepting.lock().await);
    }

    #[tokio::test]
    async fn tests_that_a_console_messages_context_names_the_frame_it_logged_in() {
        use crate::fake_browser::{FakeBrowser, Replies};

        let browser = FakeBrowser::start(Replies::default()).await;
        let events = PageEvents::new(
            EventHub::default(),
            "T",
            browser.cdp().await,
            SessionId::from("s"),
            false,
        );
        let context = |method: &str, params: Value| CdpEvent {
            method: method.to_string(),
            params,
            session_id: Some(SessionId::from("s")),
        };
        events.track_context(
            &context(
                "Runtime.executionContextCreated",
                json!({ "context": { "id": 7, "auxData": { "frameId": "CHILD", "isDefault": true } } }),
            ),
            &SessionId::from("s"),
        );
        events.track_context(
            &context(
                "Runtime.executionContextCreated",
                json!({ "context": { "id": 8, "auxData": { "frameId": "CHILD", "isDefault": false } } }),
            ),
            &SessionId::from("s"),
        );
        let frame_of = |id: i64| {
            events
                .contexts
                .lock()
                .ok()
                .and_then(|contexts| contexts.get(&(SessionId::from("s"), id)).cloned())
        };
        assert_eq!(frame_of(7).as_deref(), Some("CHILD"), "a page world");
        assert_eq!(frame_of(8), None, "an isolated world is no console's");
        events.track_context(
            &context("Runtime.executionContextsCleared", json!({})),
            &SessionId::from("s"),
        );
        assert_eq!(frame_of(7), None, "cleared with its session's contexts");
    }

    #[tokio::test]
    async fn tests_that_the_lifecycle_replay_is_history_except_on_a_discovered_page() {
        use crate::fake_browser::{FakeBrowser, Replies};
        use crate::page::Fired;

        let browser = FakeBrowser::start(Replies::default()).await;
        let cdp = browser.cdp().await;
        let loaded = Applied {
            fired: Fired {
                dom_content_loaded: true,
                load: true,
            },
            ..Applied::default()
        };
        let mut seen = Vec::new();
        for discovered in [false, true] {
            let hub = EventHub::default();
            hub.subscribe("page@T", "load", true);
            hub.subscribe("page@T", "domcontentloaded", true);
            let events = PageEvents::new(
                hub.clone(),
                "T",
                cdp.clone(),
                SessionId::from("s"),
                discovered,
            );
            events.lifecycle(&loaded);
            let replayed = serde_json::from_str::<Vec<Value>>(&hub.drain(None)).unwrap();
            events.lifecycle_replayed();
            events.lifecycle(&loaded);
            let live = serde_json::from_str::<Vec<Value>>(&hub.drain(None)).unwrap();
            let methods = |events: &[Value]| -> Vec<String> {
                events
                    .iter()
                    .map(|event| event["method"].as_str().unwrap().to_string())
                    .collect()
            };
            seen.push((discovered, methods(&replayed), methods(&live)));
        }
        let both = vec!["domcontentloaded".to_string(), "load".to_string()];
        assert_eq!(
            seen,
            [
                (false, Vec::new(), both.clone()),
                (true, both.clone(), both)
            ],
            "a known page's replay is history; a new page's is its own"
        );
    }
}
