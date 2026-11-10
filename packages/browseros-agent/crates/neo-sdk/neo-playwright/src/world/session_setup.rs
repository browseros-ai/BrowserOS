//! What each CDP session needs once before the first world on it is sealed.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use browseros_core::sensitive_fields::{JOURNAL_BINDING, SEALED_WORLD};
use browseros_core::{CdpConnection, PageId, ProtocolSession, SessionId};
use serde_json::{Value, json};
use tokio::sync::OnceCell;

use crate::probe::ProbeFailure;
use crate::world::ContextWatch;

/// How a session reached the crate, which decides what its setup must enable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionKind {
    /// The page's own session. Core enabled `Runtime` and `Page` on it when it attached the tab.
    Page,
    /// An out-of-process iframe's session. Core enables only `DOM` and `Accessibility` there,
    /// and `Page` too on an agent's own tab, for its file chooser interception.
    Oopif,
}

/// Per CDP session, once, before its first seal: `Runtime.enable` on an OOPIF session, then
/// `Runtime.addBinding { name: __neoJournal, executionContextName: browseros-sdk }`.
///
/// Only what a sealed world needs belongs here: a frame's first sweep installs its world, so
/// every call this setup sends spends that frame's `FRAME_BUDGET`, and under load each one can
/// wait hundreds of milliseconds behind the renderer's other work. `Page` events are enabled by
/// the ops that watch documents instead (`DocumentWatch::enable_page`).
///
/// The order is measured, not chosen. The seal reads `globalThis.__neoJournal` once, when it
/// installs, so the binding must already be in the world by then; and Chromium puts a
/// named binding into contexts created later only on a session with `Runtime` enabled. On an
/// OOPIF session without `Runtime.enable`, the new world had no binding, the seal captured
/// none, and that frame's journal stayed silent even after a later enable.
///
/// Process-wide, keyed by session id, because a session outlives any one page's state. It also
/// remembers each session's page, so the journal can attribute an entry. A new connection
/// epoch invalidates every session id, so the table starts over then.
pub(crate) struct SessionSetup {
    connection: Arc<dyn CdpConnection>,
    inner: Mutex<SetupInner>,
}

// By hand: the connection is a trait object, and the table is not worth printing.
impl fmt::Debug for SessionSetup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionSetup").finish_non_exhaustive()
    }
}

struct SetupInner {
    /// The connection epoch the entries belong to.
    epoch: u64,
    sessions: HashMap<SessionId, SessionEntry>,
}

struct SessionEntry {
    page: PageId,
    /// Single-flight: concurrent first worlds on a session share one setup. A failed setup
    /// leaves it empty for the next caller.
    done: Arc<OnceCell<()>>,
}

impl SessionSetup {
    /// The table for sessions on `connection`.
    pub(crate) fn new(connection: Arc<dyn CdpConnection>) -> Self {
        let epoch = connection.connection_epoch();
        Self {
            connection,
            inner: Mutex::new(SetupInner {
                epoch,
                sessions: HashMap::new(),
            }),
        }
    }

    /// Sets `session` up for sealed worlds, the first time only, and records that it belongs
    /// to `page`. Idempotent: CDP keeps both settings for the session's lifetime.
    pub(crate) async fn ensure(
        &self,
        session: &ProtocolSession,
        page: &PageId,
        kind: SessionKind,
    ) -> Result<(), ProbeFailure> {
        let Some(id) = session.session_id() else {
            return Err(ProbeFailure::Broken(
                "a sealed world needs a target session".to_owned(),
            ));
        };
        let done = {
            let mut inner = self.lock();
            let entry = inner
                .sessions
                .entry(id.clone())
                .or_insert_with(|| SessionEntry {
                    page: page.clone(),
                    done: Arc::default(),
                });
            Arc::clone(&entry.done)
        };
        done.get_or_try_init(|| async {
            if kind == SessionKind::Oopif {
                session
                    .send::<_, Value>("Runtime.enable", json!({}))
                    .await
                    .map_err(ProbeFailure::from_core)?;
            }
            session
                .send::<_, Value>(
                    "Runtime.addBinding",
                    json!({ "name": JOURNAL_BINDING, "executionContextName": SEALED_WORLD }),
                )
                .await
                .map_err(ProbeFailure::from_core)?;
            Ok(())
        })
        .await?;
        Ok(())
    }

    /// A watch over the connection's `Runtime` context events, for an install to take right
    /// before it creates its world (`SealedWorld::install`).
    pub(crate) fn watch_contexts(&self) -> ContextWatch {
        ContextWatch::new(self.connection.events())
    }

    /// The page `session` was set up for, for the journal.
    pub(crate) fn page_of(&self, session: &SessionId) -> Option<PageId> {
        self.lock()
            .sessions
            .get(session)
            .map(|entry| entry.page.clone())
    }

    /// The table, emptied first if the connection moved to a new epoch since it was filled.
    fn lock(&self) -> MutexGuard<'_, SetupInner> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let epoch = self.connection.connection_epoch();
        if inner.epoch != epoch {
            inner.epoch = epoch;
            inner.sessions.clear();
        }
        inner
    }
}
