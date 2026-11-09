//! Runs an agent's Playwright-style script from a starting BrowserOS tab. Each handed-out
//! Page keeps its target identity; live getters read CDP. Browser calls run on owned tasks,
//! with progress and delivery records that outlive the script's realm.
//!
//! ```text
//! Runtime::run                           runtime.rs: starting tab, run lifetime, outcome
//!   ├─ Tabs::open                        tabs.rs: context scope, ordered page registry
//!   └─ realm::run                        realm.rs: fresh QuickJS and embedded neo-quickjs-injected
//!        └─ __neo_runtime                bridge.rs: JS/Rust boundary
//!             ├─ get → Tabs              live pages, URL, initializer, opener and closed state
//!             ├─ subscribe → EventHub    events.rs: the run's event queue; producers:
//!             │                          page_events.rs (one tab's watcher), frame_events.rs
//!             │                          (its frame tree), discovery.rs (new tabs, popups)
//!             └─ Dispatcher::call        dispatch.rs: target routing, task, trace, reply
//!                  ├─ tab_calls          context/page lifecycle; tracked creation and cleanup
//!                  ├─ browser_calls      the run's Browser: created contexts, close, disposal
//!                  ├─ debugger           context.debugger: holds a call before it starts
//!                  └─ Tabs::session      one lazy session for the call's page
//!                       ├─ click::click  click.rs: policy over the Tab seam
//!                       ├─ navigate      navigate.rs: goto (commits, lifecycle, response) and title
//!                       ├─ responses     responses.rs: Request/Response commands on goto's answer
//!                       ├─ reads         reads.rs: textContent, innerText, getAttribute,
//!                       │                ariaSnapshot;
//!                       │                aria_yaml.rs renders the snapshot as Playwright's YAML
//!                       ├─ waits         waits.rs: selector, URL and lifecycle waits; no input lock
//!                       ├─ capture       capture.rs: screenshot (screenshot.rs) and pdf (pdf.rs);
//!                       │                `path` writes into the run's artifacts.rs directory
//!                       ├─ emulation     emulation.rs: viewport, media, a created context's options
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

mod abort;
mod artifacts;
mod bridge;
mod browser_representations;
mod chromium_bridge;
mod deadline;
mod debugger;
mod dispatch;
mod events;
mod mime;
mod playwright_methods;
mod progress;
mod realm;
mod run;
mod runtime;
mod url_match;

// Folder move, round 1: every module keeps its old `crate::<name>` path until the renames.
use browser_representations::{
    bindings, clock, console, dialogs, downloads, emulation, init_scripts, locator_handlers,
    network, page, responses, routes, session_state, tabs, web_storage,
};
use chromium_bridge::{
    contexts, discovery, frame_events, frame_selector, frames, input_devices, keyboard, main_world,
    network_events, page_events, switch, world,
};
use playwright_methods::{
    aria_yaml, browser_calls, capture, check, click, content, devices, drag, drop_on, element_ops,
    evaluate, fill, forms, frame_element, getters, highlight, history, lists, navigate,
    network_idle, pdf, press, reads, screenshot, set_input_files, states, tab_calls, waits,
};

pub use artifacts::{AbsolutePaths, RunArtifact};
pub use run::{
    CallLocation, CallRecord, CallResult, Delivery, DroppedCalls, PageId, RunBinary, RunEnd,
    RunError, RunOutcome, RunPage, RunRequest, RunStatus,
};
pub use runtime::{Runtime, RuntimeError};
