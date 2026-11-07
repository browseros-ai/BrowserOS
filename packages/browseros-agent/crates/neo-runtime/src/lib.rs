//! Runs an agent's Playwright-style script from a starting BrowserOS tab. Each handed-out
//! Page keeps its target identity; live getters read CDP. Browser calls run on owned tasks,
//! with progress and delivery records that outlive the script's realm.
//!
//! ```text
//! Runtime::run                           runtime.rs: starting tab, run lifetime, outcome
//!   ├─ Tabs::open                        tabs.rs: context scope, ordered page registry
//!   └─ quickjs::run                        quickjs.rs: fresh QuickJS and embedded neo-quickjs-injected
//!        └─ __neo_runtime                quickjs_bridge.rs: JS/Rust boundary
//!             ├─ get → Tabs              live pages, URL, initializer, opener and closed state
//!             ├─ subscribe → EventHub    events.rs: the run's event queue; producers:
//!             │                          page_events.rs (one tab's watcher), frame_events.rs
//!             │                          (its frame tree), discovery.rs (new tabs, popups)
//!             └─ Dispatcher::call        method_router.rs: target routing, task, trace, reply
//!                  ├─ page_and_context_methods  context/page lifecycle; tracked creation and cleanup
//!                  ├─ browser_methods    the run's Browser: created contexts, close, disposal
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
//! id attribute shared with the page through `playwright_injected_methods.rs`.
//!
//! The default-off `test-host` feature adds an opaque transport owned by one explicitly
//! hosted run. `quickjs_bridge.rs` installs its receive/reply callbacks; page operations still
//! use the same SDK and dispatcher. Ordinary runs never receive that additional global.

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
mod browser_representations;
mod chromium_bridge;
mod deadline;
mod debugger;
mod events;
mod method_router;
mod mime;
mod playwright_methods;
mod progress;
mod quickjs;
mod quickjs_bridge;
mod run;
mod runtime;
#[cfg(feature = "test-host")]
mod test_host;
mod url_match;

pub use artifacts::{AbsolutePaths, RunArtifact};
pub use run::{
    CallLocation, CallRecord, CallResult, Delivery, DroppedCalls, PageId, RunBinary, RunEnd,
    RunError, RunOutcome, RunPage, RunRequest, RunStatus,
};
pub use runtime::{Runtime, RuntimeError};
#[cfg(feature = "test-host")]
pub use test_host::{TestHost, TestHostError};
