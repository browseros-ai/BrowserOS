//! The host side of a `Runtime`: what a server needs to serve its hosts (the partner backend
//! reading `/v1`, later the cockpit). Opt-in through `RuntimeOptions`; a runtime built without
//! it opens nothing extra and runs scripts exactly as before.
//!
//! ```text
//!  run path (dispatch / Trace / Runtime::run / NewPage::keep)
//!    └─ HostEvent ──▶ tap (one bounded FIFO) ──▶ ledger: sessions, touched tabs, current tab,
//!                                                 privacy, completion
//!  host's OWN CDP socket (never the runs' client: frames must not queue behind run commands)
//!    ├─ Target.* ──▶ directory: target id ↔ BrowserOS tab id, URL, title, open
//!    ├─ presence per touched tab: the one session that holds focus emulation, so background
//!    │  tabs keep painting between runs
//!    └─ capture per (tab, kind, quality): MJPEG frames, AV1 fMP4 fragments, after-call stills
//! ```
//!
//! The server's `/v1` adapter (`neo-sdk-server`) maps the ledger and the trace store onto the
//! hosted contract; nothing here serves HTTP. Design: `designs/neo-host-events/design.md` in
//! the claw-sdk-v2 notes.

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "runtime wiring starts the after-call worker next")
)]
mod after_call;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "live views open once Runtime is built with host options"
    )
)]
mod capture;
mod cdp;
mod directory;
mod emit;
mod events;
mod ids;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the host link opens once Runtime is built with host options"
    )
)]
mod link;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "presences start once Runtime is built with host options"
    )
)]
mod presence;
mod sessions;
mod store;
mod tap;
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "runtime wiring starts the host task next")
)]
mod task;
mod view;

use std::fmt;
use std::sync::{Arc, Mutex};

use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

use crate::host::capture::CaptureHub;
use crate::host::tap::Tap;
use crate::host::task::{Command, Published};
use crate::host::view::FollowView;
use crate::run::PageId;

pub use capture::{CaptureError as LiveError, Frame, Frames, Quality, VideoChunks};
pub(crate) use emit::RunEmitter;
pub(crate) use events::HostEvent;
pub use ids::{EpochMs, SessionKey, TabId};
pub use store::{CallArtifacts, HostStore, MemoryStore, StoreError};

/// How long a session stays live after its last run unless the server says otherwise:
/// claw-server's `DEFAULT_SESSION_IDLE_MS` (`apps/claw-server/src/config.rs:19`, origin/main).
const DEFAULT_IDLE: Duration = Duration::from_mins(30);

/// What a server sets to give its `Runtime` a host side (`RuntimeOptions::host`).
#[derive(Clone)]
pub struct HostOptions {
    /// The browser's CDP port. The host side opens its own connection to it, never sharing the
    /// runs' client, so capture frames never queue behind run commands.
    pub cdp_port: u16,
    /// How long a session stays live after the pass of its last run was dropped.
    pub idle: Duration,
    /// The most runs the server admits at once. It sizes the tap's lifecycle band, so run and
    /// pass events are refused only when more runs than this are in flight.
    pub max_runs: usize,
    /// The trace store that persists artifacts and acknowledges completion.
    pub store: Arc<dyn HostStore>,
}

impl fmt::Debug for HostOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostOptions")
            .field("cdp_port", &self.cdp_port)
            .field("idle", &self.idle)
            .field("max_runs", &self.max_runs)
            .finish_non_exhaustive()
    }
}

impl HostOptions {
    /// Options for a browser on `cdp_port` and a server admitting `max_runs` runs at once, with
    /// sessions idling out after 30 minutes.
    #[must_use]
    pub fn new(cdp_port: u16, max_runs: usize, store: Arc<dyn HostStore>) -> Self {
        Self {
            cdp_port,
            idle: DEFAULT_IDLE,
            max_runs,
            store,
        }
    }
}

/// A runtime's host side, from `Runtime::host`. Cheap to clone; clones share one tap.
#[derive(Clone)]
pub struct Host(Arc<Inner>);

struct Inner {
    options: HostOptions,
    tap: Arc<Tap>,
    commands: mpsc::UnboundedSender<Command>,
    published: Arc<Mutex<Published>>,
    captures: Option<Arc<CaptureHub>>,
    shutdown: CancellationToken,
}

/// The current tab's two identities and live directory facts, for the server's `/v1` adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabRef {
    pub page: PageId,
    pub tab: TabId,
    pub url: String,
    pub title: String,
}

/// A session snapshot. Completion is monotonic and includes the store's flush acknowledgement.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent fields in the public server snapshot contract"
)]
pub struct SessionView {
    pub live: bool,
    pub withheld: bool,
    pub current_tab: Option<TabRef>,
    pub complete: bool,
    pub damaged: bool,
    pub ended_at: Option<EpochMs>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    #[error("host task has stopped")]
    Stopped,
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl Host {
    pub(crate) fn new(options: HostOptions) -> Self {
        Self::build(options, None, CancellationToken::new()).0
    }

    fn build(
        options: HostOptions,
        captures: Option<Arc<CaptureHub>>,
        shutdown: CancellationToken,
    ) -> (Self, mpsc::UnboundedReceiver<Command>) {
        let (commands, receiver) = mpsc::unbounded_channel();
        (
            Self(Arc::new(Inner {
                tap: Arc::new(Tap::new(options.max_runs)),
                options,
                commands,
                published: Arc::new(Mutex::new(Published::default())),
                captures,
                shutdown,
            })),
            receiver,
        )
    }

    pub(crate) fn tap(&self) -> &Arc<Tap> {
        &self.0.tap
    }

    /// Admits a run before it starts. The task serializes admissions, including the store's
    /// restart and collision checks, so two callers using one label join the same session.
    pub async fn admit_run(&self, label: Option<&SessionKey>) -> RunPass {
        let (reply, receiver) = oneshot::channel();
        let _ = self.0.commands.send(Command::Admit {
            label: label.cloned(),
            reply,
        });
        if let Ok(admitted) = receiver.await {
            self.issue_pass(admitted.session, admitted.successor_of)
        } else {
            tracing::warn!("admission after host shutdown");
            self.issue_pass(SessionKey::mint(), None)
        }
    }

    /// Ends live views synchronously. The task consumes the end command before issuing another
    /// pass under this label; already admitted runs keep reporting until their passes release.
    pub fn end_session(&self, session: &SessionKey) {
        if let Ok(mut published) = self.0.published.lock() {
            published.end(session);
        }
        let _ = self.0.commands.send(Command::End(session.clone()));
    }

    /// Revokes visuals before returning the future. The task owns the durable write even if
    /// the caller never polls or drops that future; callers entering secrets await it first.
    ///
    /// # Errors
    /// Returns a store failure, or `Stopped` when shutdown prevents persistence.
    pub fn withhold(
        &self,
        session: &SessionKey,
    ) -> impl Future<Output = Result<(), HostError>> + Send + 'static {
        let receiver = self.0.published.lock().ok().map(|mut published| {
            let (receiver, first, targets) = published.withhold(session);
            if let Some(hub) = &self.0.captures {
                for target in targets {
                    hub.end_captures(&target);
                }
            }
            if first {
                let _ = self.0.commands.send(Command::Withhold(session.clone()));
            }
            receiver
        });
        async move {
            let mut receiver = receiver.ok_or(HostError::Stopped)?;
            let value = receiver
                .wait_for(Option::is_some)
                .await
                .map_err(|_| HostError::Stopped)?;
            value.clone().unwrap_or(Err(HostError::Stopped))
        }
    }

    #[must_use]
    pub fn view(&self, session: &SessionKey) -> Option<SessionView> {
        self.0.published.lock().ok()?.view(session)
    }

    /// Registers a watch before reading completion, so a flush resolving concurrently cannot
    /// be missed. The deadline returns the latest snapshot, including one still settling.
    pub async fn wait_complete(
        &self,
        session: &SessionKey,
        deadline: Instant,
    ) -> Option<SessionView> {
        let mut changed = self.0.published.lock().ok()?.subscribe(session)?;
        let _ = time::timeout_at(deadline, changed.wait_for(|view| view.complete)).await;
        let snapshot = changed.borrow().clone();
        Some(snapshot)
    }

    fn follow(&self, session: &SessionKey) -> Result<watch::Receiver<FollowView>, LiveError> {
        self.0
            .published
            .lock()
            .ok()
            .and_then(|mut published| published.follow(session))
            .ok_or(LiveError::SessionNotFound)
    }

    /// Opens a screencast following the session's current tab, or pins it to `tab`.
    ///
    /// # Errors
    /// Returns the session/tab refusal or browser startup failure described by `LiveError`.
    pub async fn screencast(
        &self,
        session: &SessionKey,
        quality: Quality,
        tab: Option<TabId>,
    ) -> Result<Frames, LiveError> {
        let view = self.follow(session)?;
        let hub = self
            .0
            .captures
            .as_ref()
            .ok_or_else(|| LiveError::Browser("host task stopped".to_string()))?;
        hub.screencast(view, quality, tab).await
    }

    /// Opens fragmented AV1 video of the selected tab; video stays on that tab.
    ///
    /// # Errors
    /// Returns the session/tab refusal or browser startup failure described by `LiveError`.
    pub async fn video(
        &self,
        session: &SessionKey,
        quality: Quality,
        tab: Option<TabId>,
    ) -> Result<VideoChunks, LiveError> {
        let view = self.follow(session)?;
        let hub = self
            .0
            .captures
            .as_ref()
            .ok_or_else(|| LiveError::Browser("host task stopped".to_string()))?;
        hub.video(view, quality, tab).await
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "runtime shutdown wiring follows")
    )]
    pub(crate) fn shutdown(&self) {
        self.0.shutdown.cancel();
        self.0.tap.close();
    }

    /// The pass for one admitted run of `session`. Only admission (with the session ledger)
    /// calls this: a pass is the ledger's record that it counted the run.
    pub(crate) fn issue_pass(
        &self,
        session: SessionKey,
        successor_of: Option<SessionKey>,
    ) -> RunPass {
        RunPass {
            session,
            successor_of,
            tap: Arc::clone(&self.0.tap),
        }
    }
}

/// One admitted run's hold on its session, held by the run tool for exactly the run's duration.
/// While a pass of a session lives, the session is live and cannot read complete. Dropping the
/// pass puts `PassReleased` in the tap; the ledger counts the run over only once it consumes
/// that marker (design §6.6).
///
/// Drop it only after the run's future has finished or been dropped: the marker then lands
/// behind every event the run put in the tap, so consuming it proves the ledger has seen them
/// all.
#[must_use = "dropping a pass releases its run; hold it until `Runtime::run` returns"]
pub struct RunPass {
    session: SessionKey,
    successor_of: Option<SessionKey>,
    tap: Arc<Tap>,
}

impl RunPass {
    /// The key to pass in `RunRequest::session` and to echo to the host.
    #[must_use]
    pub fn session(&self) -> &SessionKey {
        &self.session
    }

    /// The label the run asked for, when that session had already ended and the run joined a
    /// fresh successor instead. Ending is final, so a session that read complete stays complete.
    #[must_use]
    pub fn successor_of(&self) -> Option<&SessionKey> {
        self.successor_of.as_ref()
    }
}

impl Drop for RunPass {
    fn drop(&mut self) {
        // A lifecycle event: refused only past the band admission sizes, and then the tap marks
        // the session damaged, so a lost release can never let the session read complete.
        let _pushed = self.tap.try_push(HostEvent::PassReleased {
            session: self.session.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dropped_pass_releases_its_session_in_the_tap() {
        let host = Host::new(HostOptions::new(9222, 1, Arc::new(MemoryStore::default())));
        let label = SessionKey::new("ended-label");
        let pass = host.issue_pass(SessionKey::new("successor"), Some(label.clone()));
        assert_eq!(pass.session(), &SessionKey::new("successor"));
        assert_eq!(pass.successor_of(), Some(&label));
        assert!(
            host.tap().drain().is_empty(),
            "a held pass releases nothing"
        );

        drop(pass);
        assert_eq!(
            host.tap().drain(),
            [HostEvent::PassReleased {
                session: SessionKey::new("successor")
            }]
        );
    }

    #[test]
    fn options_default_to_claw_servers_idle() {
        let options = HostOptions::new(9222, 4, Arc::new(MemoryStore::default()));
        assert_eq!(options.idle, Duration::from_mins(30));
        assert_eq!((options.cdp_port, options.max_runs), (9222, 4));
    }
}
