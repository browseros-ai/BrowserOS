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
//!  detach, Inspector.detached / targetCrashed ─▶ close | crash        on the page
//! ```
//!
//! Console messages and page errors exist only once `Runtime.enable` has been sent, which Neo
//! does only for a run that listens to them or reads their history ([`ConsoleLog`]). The enable
//! replays what V8 kept for the tab's live documents; those messages predate the run's interest
//! and go to the history alone, never to a listener, like messages Playwright's server saw
//! before its client subscribed.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use browseros_cdp::{CdpClient, CdpEvent, SessionId};
use neo_runtime_spec::serve::SerializedError;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::debug;

use crate::bridge;
use crate::console::{
    self, ConsoleApiCalled, ConsoleRecord, ExceptionThrown, History, LogEntryAdded, PageErrorRecord,
};
use crate::dialogs::{self, Dialogs};
use crate::events::{CONTEXT_GUID, Emitted, EventHub, Needs};
use crate::page::{Applied, DialogAnswer};
use crate::run::PageId;
use crate::tabs::{PageEntry, Tabs};

/// The producer side of one tab's session: cheap to clone, shared by the tab's `PageSession`
/// (which serves `consoleMessages` and dialog answers from it) and its watcher.
#[derive(Clone)]
pub(crate) struct PageEvents {
    hub: EventHub,
    page_guid: String,
    target: String,
    cdp: CdpClient,
    session: SessionId,
    pub(crate) console: Arc<ConsoleLog>,
    pub(crate) dialogs: Arc<Dialogs>,
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
    pub(crate) fn new(hub: EventHub, target: &str, cdp: CdpClient, session: SessionId) -> Self {
        Self {
            hub,
            page_guid: bridge::page_guid(&PageId::new(target.to_string())),
            target: target.to_string(),
            cdp,
            session,
            console: Arc::default(),
            dialogs: Arc::default(),
        }
    }

    pub(crate) fn hub(&self) -> &EventHub {
        &self.hub
    }

    /// The tab's target id.
    pub(crate) fn target(&self) -> &str {
        &self.target
    }

    /// Applies one event the watcher already applied to the page's state (`applied`), and
    /// returns the dialog answer the watcher still owes: `applied.dialog` unless a listener now
    /// owns the dialog.
    pub(crate) fn apply(&self, event: &CdpEvent, applied: &Applied) -> Option<DialogAnswer> {
        if event.session_id.as_ref() != Some(&self.session) {
            return applied.dialog;
        }
        let mut answer = applied.dialog;
        match event.method.as_str() {
            "Runtime.consoleAPICalled" => self.console_api(&event.params),
            "Log.entryAdded" => self.log_entry(&event.params),
            "Runtime.exceptionThrown" => self.exception(&event.params),
            "Page.javascriptDialogOpening" => answer = self.dialog_opening(&event.params, answer),
            "Page.javascriptDialogClosed" => self.dialog_closed(),
            _ => {}
        }
        if applied.new_document {
            self.console.mark_navigation();
        }
        answer
    }

    /// The session ended: the tab closed or crashed. Emits `close` or `crash` on the page.
    pub(crate) fn closing(&self, event: Option<&CdpEvent>) {
        let crashed = event.is_some_and(|event| event.method == "Inspector.targetCrashed");
        let name = if crashed { "crash" } else { "close" };
        if self.hub.subscribed(&self.page_guid, name) {
            self.hub.emit(&self.page_guid, name, json!({}));
        }
    }

    /// The watcher lost events: a dialog may have closed unseen, and it dismisses any.
    pub(crate) fn lagged(&self) {
        self.dialogs.forget();
    }

    fn console_api(&self, params: &Value) {
        let Ok(event) = serde_json::from_value::<ConsoleApiCalled>(params.clone()) else {
            return;
        };
        let Some(record) = console::from_console_api(&event) else {
            return;
        };
        self.message(record, event.timestamp);
    }

    fn log_entry(&self, params: &Value) {
        let Ok(LogEntryAdded { entry }) = serde_json::from_value::<LogEntryAdded>(params.clone())
        else {
            return;
        };
        // Playwright: `server/chromium/crPage.ts` `FrameSession._onLogEntryAdded`: `args.map(arg => releaseObject(this._client, arg.objectId!));`
        for object in console::log_entry_object_ids(&entry) {
            let cdp = self.cdp.clone();
            let session = self.session.clone();
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
        if let Some(record) = console::from_log_entry(&entry) {
            self.message(record, entry.timestamp);
        }
    }

    fn message(&self, record: ConsoleRecord, timestamp: f64) {
        let live = self.console.push_message(record.clone(), timestamp);
        if !live || !self.hub.wants(&self.page_guid, "console", "console") {
            return;
        }
        let Ok(mut params) = serde_json::to_value(&record) else {
            return;
        };
        params["page"] = json!({ "guid": self.page_guid });
        self.hub.emit(CONTEXT_GUID, "console", params);
    }

    fn exception(&self, params: &Value) {
        let Ok(event) = serde_json::from_value::<ExceptionThrown>(params.clone()) else {
            return;
        };
        let record = console::from_exception(&event);
        let live = self.console.push_error(record.clone(), event.timestamp);
        if !live || !self.hub.wants(&self.page_guid, "pageerror", "weberror") {
            return;
        }
        // Playwright: `server/dispatchers/browserContextDispatcher.ts` `BrowserContextDispatcher.constructor`: `this._dispatchEvent('pageError', {`
        self.hub.emit(
            CONTEXT_GUID,
            "pageError",
            json!({
                "error": record.error,
                "page": { "guid": self.page_guid },
                "location": {
                    "url": record.location.url,
                    "line": record.location.line_number,
                    "column": record.location.column_number,
                },
            }),
        );
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
                .emit(CONTEXT_GUID, "dialog", json!({ "dialog": record }))
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

    fn dialog_closed(&self) {
        let Some(record) = self.dialogs.closed() else {
            return;
        };
        if self.hub.wants(&self.page_guid, "dialogclosed", "dialogclosed") {
            self.hub
                .emit(CONTEXT_GUID, "dialogClosed", json!({ "dialog": record }));
        }
    }

    /// Frees the objects the session's console events hold in the page (V8's `console` object
    /// group), once nothing listens to them: Neo hands no console argument out as a handle yet.
    pub(crate) fn release_console_objects(&self) {
        if !self.console.requested() {
            return;
        }
        let cdp = self.cdp.clone();
        let session = self.session.clone();
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

    /// Whether no listener of the run wants this page's console messages or page errors.
    pub(crate) fn console_unwanted(&self) -> bool {
        !self.hub.wants(&self.page_guid, "console", "console")
            && !self.hub.wants(&self.page_guid, "pageerror", "weberror")
    }
}

/// One tab's console messages and page errors as Playwright's server keeps them
/// (`console::History`), and where `Runtime.enable`'s replay ends.
///
/// The replay comes before the enable's reply; [`ConsoleLog::go_live`] runs once the watcher has
/// applied everything up to that reply. Until then a message counts as live only when V8 stamped
/// it at or after the moment the enable was requested: it was logged after the subscription that
/// asked for the enable (or the history read), so a listener must see it.
#[derive(Default)]
pub(crate) struct ConsoleLog(Mutex<ConsoleState>);

#[derive(Default)]
struct ConsoleState {
    history: History,
    /// Epoch milliseconds when `Runtime.enable` was requested, as V8 stamps messages.
    requested_at: Option<f64>,
    live: bool,
}

impl ConsoleState {
    fn live(&self, timestamp: f64) -> bool {
        self.live || self.requested_at.is_some_and(|at| timestamp >= at)
    }
}

impl ConsoleLog {
    /// Records that `Runtime.enable` is about to be sent.
    pub(crate) fn request(&self) {
        if let Ok(mut state) = self.0.lock()
            && state.requested_at.is_none()
        {
            state.requested_at = Some(now_ms());
        }
    }

    fn requested(&self) -> bool {
        self.0
            .lock()
            .is_ok_and(|state| state.requested_at.is_some())
    }

    /// The enable's replay has been applied; every later message is live.
    pub(crate) fn go_live(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.live = true;
        }
    }

    fn push_message(&self, record: ConsoleRecord, timestamp: f64) -> bool {
        self.0.lock().is_ok_and(|mut state| {
            state.history.push_message(record);
            state.live(timestamp)
        })
    }

    fn push_error(&self, record: PageErrorRecord, timestamp: f64) -> bool {
        self.0.lock().is_ok_and(|mut state| {
            state.history.push_error(record);
            state.live(timestamp)
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

/// Platform: V8 stamps console messages and exceptions with `Runtime.Timestamp`, milliseconds
/// since the epoch on the browser's clock, which is this machine's for a local BrowserOS.
fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64() * 1000.0)
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
    });
}
