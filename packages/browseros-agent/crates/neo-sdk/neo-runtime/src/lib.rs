//! Neo: Runs an agent's Playwright-style script from an existing or new background tab. Each
//! handed-out Page keeps its target identity; live getters read CDP. Browser calls run on owned
//! tasks, with progress and delivery records that outlive the script's realm.
//!
//! ```text
//! Runtime::run                           runtime.rs: starting tab, run lifetime, outcome
//!   ├─ Tabs::open                        tabs.rs: start page, context scope, ordered registry
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
//!                       ├─ click         typed click/dblclick/hover/tap; check and drag compose it
//!                       │    ├─ retry    selector resolution and shared attempt pre-checks
//!                       │    └─ aim      actionability, geometry, input turn and hit-target arming
//!                       ├─ drop_on       page-function input; shares retry and aim
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
//! creation replies, task draining, closing-state reads and the tab-strip read that gives pages
//! their URLs and titles. Late unkept creations close after the outcome, even when their IDs
//! were not yet known. Input turns (`page.rs` `TabInputs`) are shared by every call and run of a
//! `Runtime` on the same target.
//!
//! Locator construction does no I/O. The SDK uses Playwright's selector code, with the test
//! id attribute shared with the page through `playwright_injected_methods.rs`.
//!
//! The default-off `test-host` feature adds an opaque transport owned by one explicitly
//! hosted run. `quickjs_bridge.rs` installs its receive/reply callbacks; page operations still
//! use the same SDK and dispatcher. Ordinary runs never receive that additional global.
//!
//! The default-off `hosted-run` feature lends that SDK door to another JavaScript engine through
//! `Runtime::open_hosted_run`. The host supplies its deadline and calls `HostedRun::finish`;
//! drop, browser loss and the deadline also end the run. `Runtime::shutdown_hosted_runs` stops
//! admission, cancels active runs and drains cleanup before the host disconnects CDP. The hosted
//! starting tab is private and closes at run end. Ordinary QuickJS runs are unchanged.
//! Hosted runs keep the existing call/log budgets in [`RunOutcome`], an event queue bounded to
//! 10,000 events / 16 MiB, and at most 10,000 undisposed JS handles and 10,000 live handle groups.
//! Disposing handles frees their slots; groups belonging to closed pages are reclaimed on reuse.
//! Page/context identities remain in first-seen order for the outcome, so workers should close
//! each test's context and release its SDK objects. The Node add-on defaults to a 24-hour run.
//!
//! `run_observed` publishes kept call starts and ends from Trace's existing mutex.
//! The optional neo-trace crate consumes them; the runtime never owns run ids or log formatting.

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
mod captcha;
mod chromium_bridge;
mod deadline;
mod debugger;
mod events;
mod file_navigation;
#[cfg(feature = "hosted-run")]
mod hosted_run;
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
pub use captcha::{CaptchaHost, SolveReply};
#[cfg(feature = "hosted-run")]
pub use hosted_run::HostedRun;
pub use run::{
    CallEvent, CallLocation, CallRecord, CallResult, CallStarted, Delivery, DroppedCalls, PageId,
    RunBinary, RunEnd, RunError, RunOutcome, RunPage, RunRequest, RunStatus, StartPage,
};
pub use runtime::{Runtime, RuntimeError};
#[cfg(feature = "test-host")]
pub use test_host::{TestHost, TestHostError};

/// Describes the script realm and material Playwright differences for model-facing hosts.
/// The runtime owns these facts because it installs the globals and owns run-scoped cleanup.
/// Hosts add their transport, output-budget and artifact policies without listing SDK methods.
#[must_use]
pub fn api_description() -> &'static str {
    concat!(
        "Use the Playwright API on page, context, and playwright. Code runs in a fresh QuickJS realm for each run. ",
        "An async function receives (page, context, playwright); an async function body or a lone expression also works. Return the result you need. ",
        "The realm provides console, setTimeout, clearTimeout, setInterval, clearInterval, URL, URLSearchParams, Buffer, TextEncoder, TextDecoder, structuredClone, crypto, AbortController, and AbortSignal. ",
        "fetch, Intl, require, process, and imports are unavailable. Use page.evaluate for code that needs the page's DOM and globals. ",
        "Neo uses the attached BrowserOS browser. BrowserType launch/connect methods, Firefox, WebKit, and playwright.request.newContext are unavailable. ",
        "Ordinary tabs survive runs. JavaScript variables, JS handles, event listeners, routes, init scripts, permission grants, clock changes, and session overrides belong to one run. ",
        "Contexts created with browser.newContext and their pages are disposed when the run ends. Viewport and media emulation also end with the run. ",
        "context.newPage opens a background tab. A route can fetch its paused request once; route.fetch sends that browser request rather than an independent Node request, and route.fulfill cannot reuse its response body. ",
        "The API has explicit unsupported operations and options; their errors describe the limitation. Browser call logs and delivery records distinguish completed, unfinished, and uncertain input."
    )
}
