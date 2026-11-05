//! A scripted browser for the camera and the tab checks: the tabs a test opens, and how captures
//! answer. It speaks the CDP the capture path sends (`Browser.getTabs`, `Browser.getTabInfo`,
//! `Target.attachToTarget`, `Page.captureScreenshot`), what a sweep of a one-frame tab sends
//! (`Page.getFrameTree`, the sealed world, the seal's field values), the control journal's script,
//! the fit's window calls, with one window whose state `setWindowBounds` changes, and
//! `Browser.activateTab`, which brings a tab to the front; every other command answers `{}`.
//!
//! It is not core's scripted `TestConnection`: captures that a test holds and releases need a
//! reply that waits, which a canned reply cannot do.

use crate::support::BoxError;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use browseros_cdp::{CdpError, CdpEvent, SessionId};
use browseros_core::sensitive_fields::{SEALED_WORLD, seal_source};
use browseros_core::{BrowserSession, BrowserSessionHooks, CdpConnection};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::{broadcast, watch};

/// What every capture returns: a JPEG's start and end markers around a little data.
pub(crate) const JPEG: &[u8] = b"\xff\xd8\xff\xe0 scripted page \xff\xd9";

/// Session ids are unique across every scripted browser in the process: core serializes
/// captures with a process-wide lock per session id, so two tests' `session-1` would wait on
/// each other's captures.
static SESSIONS: AtomicUsize = AtomicUsize::new(0);

/// How captures answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Captures {
    /// At once.
    Answer,
    /// Once the test releases them.
    Held,
    /// Never.
    Hang,
}

pub(crate) struct ScriptedBrowser {
    tabs: Mutex<Vec<i64>>,
    /// The tab in front. Until a tab is brought there, every tab reports itself active.
    front: Mutex<Option<i64>>,
    /// What every tab's password, card and code fields hold, as the seal reports it to a sweep.
    fields: Mutex<Vec<String>>,
    /// URLs a test set; any other tab is on `https://example.test/<tab>`.
    urls: Mutex<HashMap<i64, String>>,
    /// `Browser.getTabs` fails, as when the browser is gone.
    unreachable: AtomicBool,
    captures: watch::Sender<Captures>,
    started: watch::Sender<usize>,
    events: broadcast::Sender<CdpEvent>,
    /// The one window's state; openbox maximizes a normal window.
    window: Mutex<&'static str>,
}

impl ScriptedBrowser {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            tabs: Mutex::default(),
            front: Mutex::default(),
            fields: Mutex::default(),
            urls: Mutex::default(),
            unreachable: AtomicBool::new(false),
            captures: watch::channel(Captures::Answer).0,
            started: watch::channel(0).0,
            events: broadcast::channel(16).0,
            window: Mutex::new("maximized"),
        })
    }

    pub(crate) fn set_window(&self, state: &'static str) {
        *self.window.lock().unwrap_or_else(PoisonError::into_inner) = state;
    }

    /// The window's state now.
    pub(crate) fn window(&self) -> &'static str {
        *self.window.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A session over this browser, as the app's browser link would hand the service.
    pub(crate) fn session(self: &Arc<Self>) -> Arc<BrowserSession> {
        BrowserSession::new(
            Arc::clone(self) as Arc<dyn CdpConnection>,
            BrowserSessionHooks::default(),
        )
    }

    pub(crate) fn open_tab(&self, tab: i64) {
        self.lock_tabs().push(tab);
    }

    pub(crate) fn close_tab(&self, tab: i64) {
        self.lock_tabs().retain(|open| *open != tab);
    }

    /// Brings `tab` to the front, as the agent's or the owner's `Browser.activateTab` does.
    pub(crate) fn bring_to_front(&self, tab: i64) {
        *self.front.lock().unwrap_or_else(PoisonError::into_inner) = Some(tab);
    }

    /// The owner typed these into sensitive fields.
    pub(crate) fn type_into_fields(&self, values: &[&str]) {
        *self.fields.lock().unwrap_or_else(PoisonError::into_inner) =
            values.iter().map(|value| (*value).to_owned()).collect();
    }

    /// The tab navigated: the browser reports `url` for it from now on.
    pub(crate) fn set_url(&self, tab: i64, url: &str) {
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(tab, url.to_string());
    }

    /// The browser stops answering `Browser.getTabs` from now on.
    pub(crate) fn become_unreachable(&self) {
        self.unreachable.store(true, Ordering::SeqCst);
    }

    /// Captures wait from now until [`Self::release_captures`].
    pub(crate) fn hold_captures(&self) {
        self.captures.send_replace(Captures::Held);
    }

    pub(crate) fn release_captures(&self) {
        self.captures.send_replace(Captures::Answer);
    }

    /// Captures never answer from now on.
    pub(crate) fn hang_captures(&self) {
        self.captures.send_replace(Captures::Hang);
    }

    /// Waits (up to 5 s) until `count` captures have started.
    pub(crate) async fn captures_started(&self, count: usize) -> Result<(), BoxError> {
        let mut started = self.started.subscribe();
        tokio::time::timeout(
            Duration::from_secs(5),
            started.wait_for(|started| *started >= count),
        )
        .await??;
        Ok(())
    }

    /// How many captures have started.
    pub(crate) fn captures(&self) -> usize {
        *self.started.borrow()
    }

    fn lock_tabs(&self) -> std::sync::MutexGuard<'_, Vec<i64>> {
        self.tabs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn tab(&self, tab: i64) -> Value {
        let url = self
            .urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&tab)
            .cloned()
            .unwrap_or_else(|| format!("https://example.test/{tab}"));
        let front = *self.front.lock().unwrap_or_else(PoisonError::into_inner);
        json!({
            "tabId": tab,
            "targetId": format!("target-{tab}"),
            "windowId": 1,
            "url": url,
            "title": format!("Tab {tab}"),
            "isActive": front.is_none_or(|front| front == tab),
            "isLoading": false,
            "loadProgress": 1.0,
            "isPinned": false,
            "isHidden": false,
        })
    }

    async fn capture(&self) -> Result<Value, CdpError> {
        self.started.send_modify(|started| *started += 1);
        let mut captures = self.captures.subscribe();
        if *captures.borrow_and_update() == Captures::Hang {
            std::future::pending::<()>().await;
        }
        if captures
            .wait_for(|captures| *captures == Captures::Answer)
            .await
            .is_err()
        {
            return Err(CdpError::ConnectionLost);
        }
        Ok(json!({ "data": STANDARD.encode(JPEG) }))
    }

    async fn reply(&self, method: &str, params: &Value) -> Result<Value, CdpError> {
        match method {
            "Browser.getTabs" if self.unreachable.load(Ordering::SeqCst) => {
                Err(CdpError::ConnectionLost)
            }
            "Browser.getTabs" => {
                let open = self.lock_tabs().clone();
                Ok(json!({
                    "tabs": open.into_iter().map(|tab| self.tab(tab)).collect::<Vec<_>>(),
                }))
            }
            "Browser.activateTab" => {
                if let Some(tab) = params["tabId"].as_i64() {
                    self.bring_to_front(tab);
                }
                Ok(json!({}))
            }
            "Browser.getTabInfo" => {
                let open = params["tabId"]
                    .as_i64()
                    .filter(|tab| self.lock_tabs().contains(tab));
                match open {
                    Some(tab) => Ok(json!({ "tab": self.tab(tab) })),
                    None => Err(CdpError::Protocol {
                        code: -32000,
                        message: "No tab with given id".into(),
                    }),
                }
            }
            "Target.attachToTarget" => {
                let session = SESSIONS.fetch_add(1, Ordering::SeqCst) + 1;
                Ok(json!({ "sessionId": format!("scripted-session-{session}") }))
            }
            "Page.captureScreenshot" => self.capture().await,
            // The control journal's new-document script, which a control display adds per tab.
            "Page.addScriptToEvaluateOnNewDocument" => {
                Ok(json!({ "identifier": "scripted-journal" }))
            }
            "Page.getFrameTree" => Ok(json!({ "frameTree": { "frame": { "id": "main" } } })),
            "Page.createIsolatedWorld" if params["worldName"] == SEALED_WORLD => {
                Ok(json!({ "executionContextId": 1 }))
            }
            "Runtime.evaluate" if params["expression"] == seal_source() => {
                Ok(json!({ "result": { "type": "number", "value": 1 } }))
            }
            "Runtime.evaluate" if params["expression"] == "__clawSeal.sensitiveValues()" => {
                let fields = self.fields.lock().unwrap_or_else(PoisonError::into_inner);
                Ok(json!({ "result": { "type": "object", "value": *fields } }))
            }
            "Browser.getWindowForTarget" => {
                Ok(json!({ "windowId": 1, "bounds": { "windowState": self.window() } }))
            }
            "Browser.getWindowBounds" => Ok(json!({ "bounds": { "windowState": self.window() } })),
            "Browser.getWindows" => Ok(json!({
                "windows": [{ "windowId": 1, "bounds": { "windowState": self.window() } }],
            })),
            "Browser.setWindowBounds" => {
                self.set_window(match params["bounds"]["windowState"].as_str() {
                    Some("fullscreen") => "fullscreen",
                    _ => "maximized",
                });
                Ok(json!({}))
            }
            _ => Ok(json!({})),
        }
    }
}

impl CdpConnection for ScriptedBrowser {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move { self.reply(method, &params).await })
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        _params_json: &'a str,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move { Err(CdpError::Transport(format!("{method}: not scripted"))) })
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn connection_epoch(&self) -> u64 {
        1
    }
}
