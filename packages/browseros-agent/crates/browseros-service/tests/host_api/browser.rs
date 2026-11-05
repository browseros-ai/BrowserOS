//! A scripted browser for the camera: the tabs a test opens, and how captures answer. It speaks
//! the CDP the capture path sends (`Browser.getTabs`, `Browser.getTabInfo`,
//! `Target.attachToTarget`, `Page.captureScreenshot`); every other command answers `{}`.

use crate::support::BoxError;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use browseros_cdp::{CdpError, CdpEvent, SessionId};
use browseros_core::{BrowserSession, BrowserSessionHooks, CdpConnection};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
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
    captures: watch::Sender<Captures>,
    started: watch::Sender<usize>,
    events: broadcast::Sender<CdpEvent>,
}

impl ScriptedBrowser {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            tabs: Mutex::default(),
            captures: watch::channel(Captures::Answer).0,
            started: watch::channel(0).0,
            events: broadcast::channel(16).0,
        })
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

    fn tab(tab: i64) -> Value {
        json!({
            "tabId": tab,
            "targetId": format!("target-{tab}"),
            "windowId": 1,
            "url": format!("https://example.test/{tab}"),
            "title": format!("Tab {tab}"),
            "isActive": true,
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
            "Browser.getTabs" => Ok(json!({
                "tabs": self.lock_tabs().iter().map(|tab| Self::tab(*tab)).collect::<Vec<_>>(),
            })),
            "Browser.getTabInfo" => {
                let open = params["tabId"]
                    .as_i64()
                    .filter(|tab| self.lock_tabs().contains(tab));
                match open {
                    Some(tab) => Ok(json!({ "tab": Self::tab(tab) })),
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
