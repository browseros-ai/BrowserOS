//! Runs an agent's Playwright-style script from a starting BrowserOS tab. Each handed-out
//! Page keeps its target identity; live getters read CDP. Browser calls run on owned tasks,
//! with progress and delivery records that outlive the script's realm.
//!
//! ```text
//! Runtime::run                           runtime.rs: starting tab, run lifetime, outcome
//!   ├─ Tabs::open                        tabs.rs: context scope, ordered page registry
//!   └─ realm::run                        realm.rs: fresh QuickJS and embedded neo-sdk
//!        └─ __neo_runtime                bridge.rs: JS/Rust boundary
//!             ├─ get → Tabs              live pages, URL, initializer and closed state
//!             └─ Dispatcher::call        dispatch.rs: target routing, task, trace, reply
//!                  ├─ tab_calls          context/page lifecycle; tracked creation and cleanup
//!                  └─ Tabs::session      one lazy session for the call's page
//!                       ├─ click::click  click.rs: policy over the Tab seam
//!                       ├─ navigate      navigate.rs: goto (commits, lifecycle, response) and title
//!                       ├─ responses     responses.rs: Request/Response commands on goto's answer
//!                       ├─ reads         reads.rs: textContent, innerText, getAttribute,
//!                       │                ariaSnapshot;
//!                       │                aria_yaml.rs renders the snapshot as Playwright's YAML
//!                       ├─ waits         waits.rs: selector, URL and lifecycle waits; no input lock
//!                       └─ FrameTab      one frame scope over PageSession
//!                            ├─ PageSession: shared input, watcher and navigation
//!                            │    └─ NetworkLog  network.rs: the main frame's navigation requests
//!                            └─ frame_selector::locate → Frames → FrameContext
//!                                 ├─ Host: tab or child target CDP session
//!                                 └─ World: per-document injected glue
//! ```
//!
//! The registry and session writers share each `PageSession`. Its last owner detaches the
//! session; ordinary browser tabs survive the run. Finish shares one bounded wait across
//! creation replies, task draining and closing-state reads. Late unkept creations close
//! after the outcome, even when their IDs were not yet known. Input turns (`page.rs`
//! `TabInputs`) are shared by every call and run of a `Runtime` on the same target.
//!
//! Locator construction does no I/O. The SDK uses Playwright's selector code, with the test
//! id attribute shared with the page through `world.rs`.

#[cfg(test)]
extern crate self as neo_runtime;

#[cfg(test)]
#[path = "../tests/support/fake_browser.rs"]
#[expect(
    dead_code,
    reason = "unit tests use the same browser fixture as integration tests"
)]
mod fake_browser;

mod aria_yaml;
mod bridge;
mod check;
mod click;
mod console;
mod deadline;
mod dialogs;
mod dispatch;
mod evaluate;
mod events;
mod fill;
mod forms;
mod frame_selector;
mod frames;
mod getters;
mod keyboard;
mod lists;
mod main_world;
mod navigate;
mod network;
mod page;
mod page_events;
mod press;
mod progress;
mod reads;
mod realm;
mod responses;
mod run;
mod runtime;
mod states;
mod tab_calls;
mod tabs;
mod url_match;
mod waits;
mod world;

pub use run::{
    CallLocation, CallRecord, CallResult, Delivery, DroppedCalls, PageId, RunEnd, RunError,
    RunOutcome, RunPage, RunRequest, RunStatus,
};
pub use runtime::{Runtime, RuntimeError};
