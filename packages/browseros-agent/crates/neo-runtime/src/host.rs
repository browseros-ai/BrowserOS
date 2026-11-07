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
    expect(
        dead_code,
        reason = "live views open once Runtime is built with host options"
    )
)]
mod capture;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "pure core: tests drive it until the host task does"
    )
)]
mod cdp;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "pure core: tests drive it until the host task does"
    )
)]
mod directory;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "scaffold: the host ledger consumes these, the run path emits them"
    )
)]
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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "pure core: tests drive it until the host task does"
    )
)]
mod sessions;
mod tap;
mod view;

use std::sync::Arc;
use std::time::Duration;

use crate::host::events::HostEvent;
use crate::host::tap::Tap;

pub use ids::{EpochMs, SessionKey, TabId};

/// How long a session stays live after its last run unless the server says otherwise:
/// claw-server's `DEFAULT_SESSION_IDLE_MS` (`apps/claw-server/src/config.rs:19`, origin/main).
const DEFAULT_IDLE: Duration = Duration::from_mins(30);

/// What a server sets to give its `Runtime` a host side (`RuntimeOptions::host`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostOptions {
    /// The browser's CDP port. The host side opens its own connection to it, never sharing the
    /// runs' client, so capture frames never queue behind run commands.
    pub cdp_port: u16,
    /// How long a session stays live after the pass of its last run was dropped.
    pub idle: Duration,
    /// The most runs the server admits at once. It sizes the tap's lifecycle band, so run and
    /// pass events are refused only when more runs than this are in flight.
    pub max_runs: usize,
}

impl HostOptions {
    /// Options for a browser on `cdp_port` and a server admitting `max_runs` runs at once, with
    /// sessions idling out after 30 minutes.
    #[must_use]
    pub fn new(cdp_port: u16, max_runs: usize) -> Self {
        Self {
            cdp_port,
            idle: DEFAULT_IDLE,
            max_runs,
        }
    }
}

/// A runtime's host side, from `Runtime::host`. Cheap to clone; clones share one tap.
#[derive(Clone)]
pub struct Host(Arc<Inner>);

struct Inner {
    #[expect(
        dead_code,
        reason = "scaffold: host capture reads `cdp_port`, the session ledger `idle`"
    )]
    options: HostOptions,
    /// Every host event, in the order the run path and the passes produced it.
    tap: Arc<Tap>,
}

impl Host {
    pub(crate) fn new(options: HostOptions) -> Self {
        Self(Arc::new(Inner {
            tap: Arc::new(Tap::new(options.max_runs)),
            options,
        }))
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "scaffold: the run path's emitter pushes into the tap"
        )
    )]
    pub(crate) fn tap(&self) -> &Arc<Tap> {
        &self.0.tap
    }

    /// The pass for one admitted run of `session`. Only admission (with the session ledger)
    /// calls this: a pass is the ledger's record that it counted the run.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "scaffold: the host's run admission issues passes")
    )]
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
        let host = Host::new(HostOptions::new(9222, 1));
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
        let options = HostOptions::new(9222, 4);
        assert_eq!(options.idle, Duration::from_mins(30));
        assert_eq!((options.cdp_port, options.max_runs), (9222, 4));
    }
}
