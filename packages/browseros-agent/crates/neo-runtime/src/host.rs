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

#[expect(
    dead_code,
    reason = "scaffold: the host ledger consumes these, the run path emits them"
)]
mod events;
mod ids;

pub use ids::{EpochMs, SessionKey, TabId};
