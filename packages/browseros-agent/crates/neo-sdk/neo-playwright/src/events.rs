//! Page and context events (popups, closes, dialogs, console messages, page errors, downloads)
//! for `page.on`, `context.on('page')` and `waitForEvent`.
//!
//! ```text
//!  CDP events ─▶ TargetWatcher (one task per Playwright) ─▶ EventWriter ─▶ EventHub log
//!                 raw facts: target ids, never PageIds         one writer   ring of 1,024
//!                 holds dialogs for listening runs (DialogTable)            + Notify
//!                 tracks each download to its end (DownloadTable)              │
//!                                                                              │
//!  events.wait  (waitForEvent) ── reads from the script's `since` ─────────────┤ many readers,
//!  events.next  (the pump)     ── reads from each listener's own place ────────┘ none writes
//!      └─ event_reader: target ids to native tab handles, through the run's own view
//! ```
//!
//! Frame lifecycle records preserve event-time metadata for page/context callbacks. Internal
//! navigation records keep loader identity and renderer swaps for `waitForNavigation`.
//!
//! **One writer.** Only the watcher appends, through the one `EventWriter` the hub hands out,
//! so the log has a single order and needs nothing beyond a short std mutex and a `Notify`. A
//! window lookup for a popup runs in its own task and reports back to the watcher, which writes.
//!
//! **A cursor per reader.** Readers never write the log and never share a position. A
//! `waitForEvent` reads from the `Date.now()` the prelude stamped when the script called it, so
//! `const p = page.waitForEvent('popup'); await click(); await p` cannot miss the popup,
//! whichever of the two ops the host admits first: the popup is recorded after the click, which
//! came after the stamp. The pump keeps one position per listener in the run's `RunState`, so a
//! listener attached after the pump read past an event still gets it if it is newer than the
//! listener's own stamp. Run state is the host's (`OpEnv::run_state`) and dies with the run.
//!
//! **Raw facts, mapped per run.** Records name tabs by CDP target id. A reader maps a new tab to
//! a tab id through its run's view, so a run only ever sees tabs its view admits, and a popup
//! that opened as a separate window fails `waitForEvent('popup')` with `neo / unsupported` (D9).

mod dialog_table;
mod download_table;
pub(crate) mod event_hub;
pub(crate) mod event_op;
mod event_reader;
mod file_chooser;
mod frame_lifecycle;
mod navigation_event;
mod page_event;
mod run_downloads;
mod run_events;
mod subscription;
mod target_watcher;
mod watcher_progress;
pub(crate) mod worker_table;

pub(crate) use event_hub::EventHub;
pub(crate) use event_op::EventOp;
pub(crate) use navigation_event::NavigationEvent;
pub(crate) use page_event::PageEvent;
pub(crate) use run_downloads::RunDownloads;
pub(crate) use run_events::{RunEvents, WorkerReports};
pub(crate) use subscription::{ListenerKey, PageEventKind, Subscription};
pub(crate) use target_watcher::{BINDING_CALLED, BINDING_NAME, TargetWatcher};
pub(crate) use watcher_progress::{Fence, WatcherProgress};
