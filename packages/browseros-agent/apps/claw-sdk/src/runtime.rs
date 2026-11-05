//! The agent's JavaScript: one fresh QuickJS runtime per run (rquickjs 0.12.1, QuickJS-ng
//! 0.15.1). The only subtree that names rquickjs (STYLE.md P1).
//!
//! ```text
//!  run_script ─ AsyncRuntime + AsyncContext ─ js/bootstrap.js, js/page_prelude.js,
//!      │        js/neo_prelude.js, then the agent's code compiled as
//!      │        `async function (console, …, page, context, neo)`
//!      │        natives: __neo_invoke(name, json, site) → Promise<string>, never rejects
//!      │                 __neo_log(level, text)       __neo_sleep(ms) → Promise
//!      ▼
//!  Bridge::invoke ─▶ Dispatcher::submit ─▶ op task on the run's JoinSet ─▶ envelope over a oneshot
//! ```
//!
//! Three rules keep QuickJS safe inside a multi-threaded server:
//! - Only strings and numbers cross. A native takes owned strings and returns one, and every JS
//!   value lives inside the run's `async_with` future, so none outlives its runtime. QuickJS
//!   asserts on that in `JS_FreeRuntime`, and the assert aborts the whole process.
//! - Nothing awaits the runtime from inside it. rquickjs locks the runtime on every poll and
//!   drives the natives' futures inside that poll. An op runs as its own tokio task and answers
//!   through a channel, so a slow op never holds the lock and a panicking one never unwinds
//!   through QuickJS.
//! - Every way out is bounded: the heap cap, the stack cap, the run's stop, its deadline and a
//!   per-slice CPU budget, all through the one interrupt handler, plus a grace after the stop
//!   for a script awaiting a promise that never settles.

mod bridge;
mod run_logs;
mod run_script;
#[cfg(test)]
mod tests;

use bridge::Bridge;
pub(crate) use run_logs::{LogLevel, LogLine, RunLogs};
pub(crate) use run_script::{Script, ScriptEnd, ScriptOutcome, run_script};
