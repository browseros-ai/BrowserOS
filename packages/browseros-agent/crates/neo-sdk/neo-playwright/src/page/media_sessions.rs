//! Replays a page's committed media preferences when CDP attaches another iframe renderer.
//! This owns only session bindings; `PageState` owns preferences across runs. Existing renderer
//! discovery and frame-world setup also register endpoints, covering attachments before startup.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use browseros_core::{BrowserSession, CoreError, ProtocolSession, SessionId};
use serde::Deserialize;
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

use crate::page::media_emulation::MediaState;

struct Binding {
    session: ProtocolSession,
    state: Arc<AsyncMutex<MediaState>>,
    ready: Arc<AtomicBool>,
}

/// The facade owns the watch task, so dropping Playwright drops subscriptions and stops replay.
/// The task owns the shared binding map rather than this wrapper, avoiding a reference cycle.
pub(crate) struct MediaSessions {
    bindings: Arc<Mutex<HashMap<SessionId, Binding>>>,
    task: JoinHandle<()>,
}

impl Drop for MediaSessions {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl MediaSessions {
    pub(crate) fn new(browser: Arc<BrowserSession>) -> Self {
        let bindings = Arc::new(Mutex::new(HashMap::<SessionId, Binding>::new()));
        let recorded = Arc::clone(&bindings);
        // These lifecycle events also feed core and TargetWatcher through the broadcast ring.
        // Taking a targeted subscription would remove them from that ring and starve those
        // readers. Missed replay is repaired before a frame's first operation below.
        let mut events = browser.cdp_events();
        let task = tokio::spawn(async move {
            loop {
                let event = match events.recv().await {
                    Ok(event) => event,
                    Err(RecvError::Lagged(_)) => {
                        // Lost detaches could leave dead endpoints. Admission and frame setup
                        // rediscover live sessions against the page's retained settings.
                        recorded
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .clear();
                        continue;
                    }
                    Err(RecvError::Closed) => break,
                };
                match event.method.as_str() {
                    "Target.attachedToTarget" => {
                        let Ok(attached) = serde_json::from_value::<Attached>(event.params) else {
                            continue;
                        };
                        // Popups are separate pages and must not inherit their opener's overrides.
                        if attached.target_info.r#type != "iframe" {
                            continue;
                        }
                        let state = {
                            let mut known = recorded.lock().unwrap_or_else(PoisonError::into_inner);
                            let state = event
                                .session_id
                                .as_ref()
                                .and_then(|parent| known.get(parent))
                                .map(|binding| Arc::clone(&binding.state));
                            state.map(|state| {
                                let ready = Arc::new(AtomicBool::new(false));
                                let session = browser.protocol_session(attached.session_id.clone());
                                known.insert(
                                    attached.session_id.clone(),
                                    Binding {
                                        session,
                                        state: Arc::clone(&state),
                                        ready: Arc::clone(&ready),
                                    },
                                );
                                (state, ready)
                            })
                        };
                        if let Some((state, ready)) = state {
                            let session = browser.protocol_session(attached.session_id);
                            // Replay is the already-authorized page setting, not a new agent call.
                            // A detached target needs no restoration; frame setup retries a missed replay.
                            let guard = Arc::clone(&state).lock_owned().await;
                            if guard.replay(&session).await.is_ok() {
                                ready.store(true, Ordering::Release);
                            }
                        }
                    }
                    "Target.detachedFromTarget" => {
                        if let Ok(detached) = serde_json::from_value::<Detached>(event.params) {
                            recorded
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .remove(&detached.session_id);
                        }
                    }
                    _ => {}
                }
            }
        });
        Self { bindings, task }
    }

    /// Register a known descendant without holding the std mutex across a browser await.
    pub(crate) fn remember(
        &self,
        session: &ProtocolSession,
        state: Arc<AsyncMutex<MediaState>>,
    ) -> Arc<AtomicBool> {
        let mut bindings = self.bindings.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(id) = session.session_id() else {
            return Arc::new(AtomicBool::new(false));
        };
        if let Some(binding) = bindings.get(id)
            && Arc::ptr_eq(&binding.state, &state)
        {
            return Arc::clone(&binding.ready);
        }
        let ready = Arc::new(AtomicBool::new(false));
        bindings.insert(
            id.clone(),
            Binding {
                session: session.clone(),
                state,
                ready: Arc::clone(&ready),
            },
        );
        ready
    }

    /// A world can be discovered before the attachment watcher runs. Initialize its renderer
    /// synchronously before evaluating there, so the first frame read sees the page preference.
    pub(crate) async fn ensure(
        &self,
        session: &ProtocolSession,
        state: &Arc<AsyncMutex<MediaState>>,
    ) -> Result<(), CoreError> {
        let ready = self.remember(session, Arc::clone(state));
        // Synchronize with in-flight page changes and with a watcher that registered this
        // session but has not replayed it yet. A failed replay stays unready for the next call.
        let guard = Arc::clone(state).lock_owned().await;
        if !ready.load(Ordering::Acquire) {
            guard.replay(session).await?;
            ready.store(true, Ordering::Release);
        }
        Ok(())
    }

    /// Every renderer currently bound to this page; session ids deduplicate same-process frames.
    pub(crate) fn sessions(&self, state: &Arc<AsyncMutex<MediaState>>) -> Vec<ProtocolSession> {
        self.bindings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .filter(|binding| Arc::ptr_eq(&binding.state, state))
            .map(|binding| binding.session.clone())
            .collect()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Attached {
    session_id: SessionId,
    target_info: TargetInfo,
}
#[derive(Deserialize)]
struct TargetInfo {
    r#type: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Detached {
    session_id: SessionId,
}
