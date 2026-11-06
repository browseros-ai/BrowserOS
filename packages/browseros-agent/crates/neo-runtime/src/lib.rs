//! Runs an agent's Playwright-style script in Neo against one BrowserOS tab, with no cache:
//! every synchronous getter such as `page.url()` is one live CDP call. Browser calls such as
//! `locator.click()` are Playwright's own protocol messages, served here the way Playwright's
//! server serves them, each on its own task so it outlives the realm, and each traced with its
//! delivery state.
//!
//! ```text
//!  Runtime::run(RunRequest)                     runtime.rs: tab check, run order, outcome
//!    └─ realm::run                              realm.rs: fresh QuickJS, limits, host globals
//!         ├─ neo-sdk bundle → createRun()       crates/neo-sdk: Page, Frame, Locator, BrowserContext
//!         ├─ agent code (page, context)
//!         └─ __neo_runtime.get / .log / .call   bridge.rs: the only door from JS into Rust
//!              ├─ getters::url / is_closed      getters.rs: one CDP call each, via browseros-cdp
//!              └─ Dispatcher::call              dispatch.rs: route, trace entry, task, reply
//!                   └─ Progress, Trace          progress.rs: deadlines, call log, delivery, records
//! ```
//!
//! `page.rs` owns the run's CDP session; the dispatcher and every task that writes on the
//! session share it, so it detaches only after the last of them is done.
//!
//! Building a locator does no I/O: neo-sdk runs Playwright's own selector code in the realm, with
//! the test id attribute from `world.rs`, the one name the SDK and the page share.

mod bridge;
#[cfg(test)]
mod click_goldens;
mod deadline;
mod dispatch;
mod getters;
mod page;
mod progress;
mod realm;
mod run;
mod runtime;
mod world;

pub use run::{
    CallLocation, CallRecord, CallResult, Delivery, DroppedCalls, PageId, RunEnd, RunError,
    RunOutcome, RunRequest, RunStatus,
};
pub use runtime::{Runtime, RuntimeError};
