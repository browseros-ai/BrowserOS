//! Playwright: `server/chromium/crConnection.ts` `CDPSession`
//!
//! (G) `context.newCDPSession(page | frame)`: a CDP session of the script's own, attached to the
//! page's target (or an out-of-process frame's), that forwards what the script sends and hands
//! it that session's events, as Playwright's `CRBrowserContext.newCDPSession` attaches one. Its
//! domains are its own: what the script enables there never touches the run's sessions.
//!
//! ```text
//!  newCDPSession  Target.attachToTarget {targetId, flatten}  → cdpsession@<run>.<n>
//!  send           REFUSED? → Neo's error, nothing sent;  else the method on that session
//!  events         that session's events → `event` {method, params}, while subscribed
//!  detach / close Target.detachFromTarget (or the target went) → `close`; run end detaches all
//! ```
//!
//! The forwarder is guarded: a method that runs or freezes page JavaScript outside Neo's two
//! world doors, reaches another target or the browser, or writes input outside the tab's input
//! turns is refused before anything is sent ([`REFUSED`]). `tests/page_js_guard.rs` checks that
//! the table names every method of its own list, so the two cannot drift.

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use browseros_cdp::{CdpClient, CdpError, CdpEvent, SessionId};
use serde_json::{Value, json};
use tokio::sync::broadcast::{self, error::RecvError};
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::events::EventHub;
use crate::progress::{CallError, PlaywrightError, Progress};

/// Neo: the guid prefix of a script's CDP session.
pub(crate) const CDP_SESSION_PREFIX: &str = "cdpsession@";

/// Neo: one rule of what a script's CDP session never sends.
pub(crate) enum Refusal {
    /// This method.
    Method(&'static str),
    /// Every method of this domain (its name and the dot), but these.
    Domain(&'static str, &'static [&'static str]),
    /// `Page.navigate` to a `javascript:` URL, which runs the URL in the page as script.
    JavascriptUrl,
}

/// Deviation: `server/chromium/crConnection.ts` `CDPSession._send`: `return await this._session.send(method as any, params);` Playwright forwards any method; Neo refuses what would run or freeze page JavaScript outside its two world doors (the page JavaScript guard's list, document writes, a `javascript:` navigation, every `Debugger` method but its enable, disable and script reads), reach another target or the browser, or send input outside the tab's input turns.
///
/// `tests/page_js_guard.rs` reads this table: each method of its guarded list is named here, and
/// nowhere else in this file may name one.
pub(crate) const REFUSED: &[Refusal] = &[
    Refusal::Method("Runtime.evaluate"),
    Refusal::Method("Runtime.callFunctionOn"),
    Refusal::Method("Runtime.compileScript"),
    Refusal::Method("Runtime.runScript"),
    Refusal::Method("Runtime.addBinding"),
    Refusal::Method("Page.addScriptToEvaluateOnNewDocument"),
    Refusal::Method("Page.createIsolatedWorld"),
    Refusal::Method("Debugger.evaluateOnCallFrame"),
    Refusal::Method("Page.setDocumentContent"),
    Refusal::Method("DOM.setOuterHTML"),
    Refusal::JavascriptUrl,
    Refusal::Domain("Target.", &[]),
    Refusal::Domain("Browser.", &[]),
    Refusal::Domain("Input.", &[]),
    Refusal::Domain(
        "Debugger.",
        &[
            "Debugger.enable",
            "Debugger.disable",
            "Debugger.getScriptSource",
        ],
    ),
];

/// Neo: the method a [`Refusal::JavascriptUrl`] reads the `url` of.
const NAVIGATE: &str = "Page.navigate";

/// Whether a script's CDP session refuses `method` with `params` ([`REFUSED`]).
pub(crate) fn refused(method: &str, params: &Value) -> bool {
    REFUSED.iter().any(|rule| match rule {
        Refusal::Method(name) => method == *name,
        Refusal::Domain(domain, except) => method.starts_with(domain) && !except.contains(&method),
        Refusal::JavascriptUrl => {
            method == NAVIGATE && params["url"].as_str().is_some_and(is_javascript_url)
        }
    })
}

/// Platform: a URL whose scheme is `javascript`, as the URL standard reads it: tabs and newlines
/// anywhere are removed, leading spaces and controls trimmed, the scheme matched in any case.
fn is_javascript_url(url: &str) -> bool {
    let url: String = url
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    url.trim_start_matches(|c: char| c <= ' ')
        .get(..11)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("javascript:"))
}

/// The run's CDP sessions, by guid (`Tabs::cdp_sessions`).
pub(crate) struct CdpSessions {
    /// This run's random token in every guid, as init scripts' and handles' carry one.
    run: String,
    next: AtomicU64,
    sessions: Mutex<HashMap<String, Arc<Forwarded>>>,
    /// Set as run end starts: no session opens after it.
    ended: AtomicBool,
}

/// One session the script opened: its CDP session, and the token that stops its forwarder.
struct Forwarded {
    session: SessionId,
    /// Set once the session is gone, detached by the script or with its target.
    closed: AtomicBool,
    stop: CancellationToken,
}

impl Default for CdpSessions {
    fn default() -> Self {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(NEXT_RUN.fetch_add(1, Ordering::Relaxed));
        Self {
            run: format!("{:016x}", hasher.finish()),
            next: AtomicU64::new(1),
            sessions: Mutex::default(),
            ended: AtomicBool::new(false),
        }
    }
}

/// Neo: the process-wide count that keeps two runs' tokens apart within one process.
static NEXT_RUN: AtomicU64 = AtomicU64::new(1);

impl CdpSessions {
    /// Playwright: `server/chromium/crConnection.ts` `CDPSession.attachToTarget`: `const { sessionId } = await this._send('Target.attachToTarget', { targetId, flatten: true });`
    ///
    /// # Errors
    ///
    /// `TargetClosedError` once the run ended or when the target is gone; the call's stop.
    pub(crate) async fn open(
        &self,
        cdp: &CdpClient,
        hub: &EventHub,
        target: &str,
        progress: &Progress,
    ) -> Result<String, CallError> {
        if self.ended.load(Ordering::Acquire) {
            return Err(PlaywrightError::target_closed().into());
        }
        // Subscribed before the attach, so no event of the new session is missed.
        let events = cdp.events();
        let attached = progress
            .race(cdp.send(
                "Target.attachToTarget",
                json!({ "targetId": target, "flatten": true }),
                None,
            ))
            .await?
            .map_err(|_| PlaywrightError::target_closed())?;
        let session = attached["sessionId"]
            .as_str()
            .map(SessionId::from)
            .ok_or_else(unattached)?;
        let guid = format!(
            "{CDP_SESSION_PREFIX}{}.{}",
            self.run,
            self.next.fetch_add(1, Ordering::Relaxed)
        );
        let forwarded = Arc::new(Forwarded {
            session,
            closed: AtomicBool::new(false),
            stop: CancellationToken::new(),
        });
        tokio::spawn(forward(
            events,
            guid.clone(),
            Arc::clone(&forwarded),
            hub.clone(),
        ));
        let late = match self.sessions.lock() {
            Ok(mut sessions) if !self.ended.load(Ordering::Acquire) => {
                sessions.insert(guid.clone(), Arc::clone(&forwarded));
                false
            }
            _ => true,
        };
        if late {
            // The run ended while the attach was in flight: the session goes with it.
            detach_session(cdp, &forwarded).await;
            return Err(PlaywrightError::target_closed().into());
        }
        Ok(guid)
    }

    fn get(&self, guid: &str) -> Option<Arc<Forwarded>> {
        self.sessions.lock().ok()?.get(guid).cloned()
    }

    /// Playwright: `server/chromium/crConnection.ts` `CDPSession.send`
    ///
    /// # Errors
    ///
    /// Neo's refusal of a guarded method, Playwright's `Protocol error (<method>): …` for a
    /// method Chromium refuses, `TargetClosedError` once the session is gone, the call's stop.
    pub(crate) async fn send(
        &self,
        cdp: &CdpClient,
        guid: &str,
        method: &str,
        params: Option<Value>,
        progress: &Progress,
    ) -> Result<Value, CallError> {
        let params = params.unwrap_or_else(|| json!({}));
        if refused(method, &params) {
            return Err(refusal(method).into());
        }
        let forwarded = self.get(guid).ok_or_else(PlaywrightError::target_closed)?;
        if forwarded.closed.load(Ordering::Acquire) {
            return Err(PlaywrightError::target_closed().into());
        }
        match progress
            .race(cdp.send(method, params, Some(&forwarded.session)))
            .await?
        {
            Ok(result) => Ok(result),
            Err(CdpError::Protocol { message, .. })
                if !forwarded.closed.load(Ordering::Acquire) =>
            {
                // Playwright: `server/protocolError.ts` `ProtocolError.setMessage`: `Protocol error (${this.method}): ${message}`
                Err(PlaywrightError::error(format!("Protocol error ({method}): {message}")).into())
            }
            Err(_) => Err(PlaywrightError::target_closed().into()),
        }
    }

    /// Playwright: `server/chromium/crConnection.ts` `CRSession.detach`
    ///
    /// A session already gone fails as closed: Playwright's dispatcher of a session went with its
    /// `close`, and a call to it answers `TargetClosedError`.
    ///
    /// # Errors
    ///
    /// `TargetClosedError` for a session already gone, and the call's stop.
    pub(crate) async fn detach(
        &self,
        cdp: &CdpClient,
        guid: &str,
        progress: &Progress,
    ) -> Result<(), CallError> {
        let forwarded = self.get(guid).ok_or_else(PlaywrightError::target_closed)?;
        if forwarded.closed.load(Ordering::Acquire) {
            // Playwright: `server/dispatchers/dispatcher.ts` `DispatcherConnection.dispatch`: `this.onmessage({ id, error: serializeError(new TargetClosedError(undefined)) });`
            return Err(PlaywrightError::target_closed().into());
        }
        progress
            .race(cdp.send(
                "Target.detachFromTarget",
                json!({ "sessionId": forwarded.session.as_str() }),
                None,
            ))
            .await?
            .map_err(|_| PlaywrightError::target_closed())?;
        Ok(())
    }

    /// Detaches every session the run opened and stops their forwarders: none outlives the run.
    pub(crate) async fn close(&self, cdp: &CdpClient) {
        self.ended.store(true, Ordering::Release);
        let sessions: Vec<Arc<Forwarded>> = match self.sessions.lock() {
            Ok(mut sessions) => sessions.drain().map(|(_, forwarded)| forwarded).collect(),
            Err(_) => Vec::new(),
        };
        for forwarded in sessions {
            detach_session(cdp, &forwarded).await;
        }
    }
}

/// Neo: the run's sessions end with it.
impl Drop for CdpSessions {
    fn drop(&mut self) {
        if let Ok(sessions) = self.sessions.lock() {
            for forwarded in sessions.values() {
                forwarded.stop.cancel();
            }
        }
    }
}

/// Neo: Chromium answered an attach without a session.
fn unattached() -> PlaywrightError {
    PlaywrightError::error("neo-runtime: no session attached")
}

/// Neo: why a guarded method is not sent.
fn refusal(method: &str) -> PlaywrightError {
    PlaywrightError::error(format!(
        "Neo refuses to send {method} over a CDPSession: it would run or pause page JavaScript, \
         reach another target or the browser, or send input outside Neo's own paths"
    ))
}

/// Detaches `forwarded`'s session, best effort, and stops its forwarder.
async fn detach_session(cdp: &CdpClient, forwarded: &Forwarded) {
    if !forwarded.closed.swap(true, Ordering::AcqRel) {
        let _detached = cdp
            .send(
                "Target.detachFromTarget",
                json!({ "sessionId": forwarded.session.as_str() }),
                None,
            )
            .await;
    }
    forwarded.stop.cancel();
}

/// Playwright: `server/chromium/crConnection.ts` `CDPSession.constructor`: `this._session = parentSession.createChildSession(sessionId, (method, params) => this.emit(CDPSession.Events.Event, { method, params }));`
///
/// Hands the script its session's events while it listens, and `close` once the session
/// detached, by the script, with its target, or at run end.
async fn forward(
    mut events: broadcast::Receiver<CdpEvent>,
    guid: String,
    forwarded: Arc<Forwarded>,
    hub: EventHub,
) {
    loop {
        let event = tokio::select! {
            biased;
            () = forwarded.stop.cancelled() => return,
            event = events.recv() => event,
        };
        let event = match event {
            Ok(event) => event,
            Err(RecvError::Lagged(skipped)) => {
                debug!(skipped, "a script's CDP session missed events");
                continue;
            }
            Err(RecvError::Closed) => return,
        };
        if event.session_id.as_ref() == Some(&forwarded.session) {
            if hub.subscribed(&guid, "event") {
                hub.emit(
                    &guid,
                    "event",
                    &json!({ "method": event.method, "params": event.params }),
                );
            }
            continue;
        }
        // Playwright: `server/chromium/crConnection.ts` `CDPSession.constructor`: `if (event.sessionId === sessionId)`
        if event.method == "Target.detachedFromTarget"
            && event.params["sessionId"].as_str() == Some(forwarded.session.as_str())
        {
            forwarded.closed.store(true, Ordering::Release);
            if hub.subscribed(&guid, "close") {
                hub.emit(&guid, "close", &json!({}));
            }
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_forwarder_refuses_what_runs_page_javascript_or_leaves_the_page() {
        let none = json!({});
        for method in [
            "Runtime.evaluate",
            "Page.createIsolatedWorld",
            "Debugger.setBreakpointByUrl",
            "Debugger.pause",
            "Target.createTarget",
            "Browser.close",
            "Input.dispatchMouseEvent",
            "DOM.setOuterHTML",
            "Page.setDocumentContent",
        ] {
            assert!(refused(method, &none), "{method}");
        }
        for method in [
            "Runtime.enable",
            "Debugger.enable",
            "Debugger.getScriptSource",
            "Network.enable",
            "Page.getFrameTree",
            "DOM.getDocument",
        ] {
            assert!(!refused(method, &none), "{method}");
        }
        let navigate = |url: &str| refused("Page.navigate", &json!({ "url": url }));
        assert!(navigate("javascript:alert(1)"));
        assert!(navigate("  JavaScript:alert(1)"));
        assert!(navigate("java\tscript:alert(1)"));
        assert!(!navigate("https://example.com/javascript:"));
        assert!(!navigate("data:text/html,hi"));
    }
}
