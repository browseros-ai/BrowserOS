//! Runs an agent's Playwright-style script in Neo against one BrowserOS tab, with no cache:
//! every synchronous getter such as `page.url()` is one live CDP call.
//!
//! ```text
//!  Runtime::run(RunRequest)                     runtime.rs: tab check, CDP session, outcome
//!    └─ realm::run                              realm.rs: fresh QuickJS, limits, host globals
//!         ├─ neo-sdk bundle → createRun()       crates/neo-sdk: Page, BrowserContext
//!         ├─ agent code (page, context)
//!         └─ __neo_runtime.get / .log           bridge.rs: the only door from JS into Rust
//!              └─ getters::url / is_closed      getters.rs: one CDP call each, via browseros-cdp
//! ```

mod bridge;
mod deadline;
mod getters;
mod realm;
mod run;
mod runtime;

pub use run::{PageId, RunError, RunOutcome, RunRequest, RunStatus};
pub use runtime::{Runtime, RuntimeError};
