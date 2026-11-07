//! Neo: each session's one `Target.setAutoAttach` (slice W, DECIDED). Chromium keeps a single
//! auto-attach setting per session, so a second call replaces the first's filter: the run's
//! consumers say here what they want a session to attach (init scripts its iframes, `workers.rs`
//! its dedicated workers), and this sends the union, or turns auto-attach off once nothing is
//! wanted. Every target is held at its start (`waitForDebuggerOnStart`) for its consumer to
//! resume, as Playwright's page sessions hold them.
//!
//! ```text
//!  init scripts: want(Iframe)       → setAutoAttach{autoAttach, filter: [iframe]}
//!  workers.rs:   want(Worker)       → setAutoAttach{autoAttach, filter: [iframe, worker]}
//!  init scripts: release(Iframe)    → setAutoAttach{autoAttach, filter: [worker]}
//!  workers.rs:   release(Worker)    → setAutoAttach{autoAttach: false}
//! ```

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, LazyLock, Mutex as SyncMutex};

use browseros_cdp::{CdpClient, CdpError, SessionId};
use serde_json::{Value, json};
use tokio::sync::Mutex;

/// Neo: what a consumer wants a session to attach.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum AttachKind {
    /// Out-of-process iframes, for init scripts (`init_scripts.rs`).
    Iframe,
    /// Dedicated workers, for `workers.rs`.
    Worker,
}

impl AttachKind {
    /// The `TargetFilter` entry that attaches this kind.
    fn filter(self) -> Value {
        match self {
            Self::Iframe => json!({ "type": "iframe" }),
            Self::Worker => json!({ "type": "worker" }),
        }
    }
}

/// Neo: what is wanted of each session that set auto-attach, by session id. A CDP session id
/// names one session of one connection, so every consumer of a session meets here without being
/// handed anything; an entry goes once nothing is wanted of it, or with its session ([`forget`]).
static SESSIONS: LazyLock<SyncMutex<HashMap<SessionId, Arc<Mutex<BTreeSet<AttachKind>>>>>> =
    LazyLock::new(SyncMutex::default);

/// Adds `kind` to what `session` auto-attaches, and sends the union when it changed.
///
/// # Errors
///
/// The session's answer, when it refused the setting or is gone.
pub(crate) async fn want(
    cdp: &CdpClient,
    session: &SessionId,
    kind: AttachKind,
) -> Result<(), CdpError> {
    let wants = wants(session);
    // Held across the send, so a session's settings go out in the order they changed.
    let mut wants = wants.lock().await;
    if !wants.insert(kind) {
        return Ok(());
    }
    send(cdp, session, &wants).await
}

/// Removes `kind` from what `session` auto-attaches: sends the rest, or turns auto-attach off
/// once nothing is wanted, which also lets go of the session's entry.
///
/// # Errors
///
/// The session's answer, when it refused the setting or is gone.
pub(crate) async fn release(
    cdp: &CdpClient,
    session: &SessionId,
    kind: AttachKind,
) -> Result<(), CdpError> {
    let entry = wants(session);
    let mut wants = entry.lock().await;
    if !wants.remove(&kind) {
        return Ok(());
    }
    let sent = send(cdp, session, &wants).await;
    if wants.is_empty() {
        forget(session);
    }
    sent
}

/// Lets go of `session`'s entry: the session is gone, and its auto-attach with it.
pub(crate) fn forget(session: &SessionId) {
    if let Ok(mut sessions) = SESSIONS.lock() {
        sessions.remove(session);
    }
}

fn wants(session: &SessionId) -> Arc<Mutex<BTreeSet<AttachKind>>> {
    SESSIONS
        .lock()
        .map(|mut sessions| Arc::clone(sessions.entry(session.clone()).or_default()))
        .unwrap_or_default()
}

/// `session`'s auto-attach for `wants`: on with their filters, or off when there are none.
/// Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `this._client.send('Target.setAutoAttach', { autoAttach: true, waitForDebuggerOnStart: true, flatten: true }),`
async fn send(
    cdp: &CdpClient,
    session: &SessionId,
    wants: &BTreeSet<AttachKind>,
) -> Result<(), CdpError> {
    let on = !wants.is_empty();
    // Off still names a filter, as the setting it replaces did.
    let filter: Vec<Value> = if on {
        wants.iter().map(|kind| kind.filter()).collect()
    } else {
        vec![AttachKind::Iframe.filter()]
    };
    cdp.send(
        "Target.setAutoAttach",
        json!({
            "autoAttach": on,
            "waitForDebuggerOnStart": on,
            "flatten": true,
            "filter": filter,
        }),
        Some(session),
    )
    .await
    .map(|_| ())
}
