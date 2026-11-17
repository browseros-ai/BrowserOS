//! The real SDK inside another JavaScript host, enabled only by `hosted-run`.
//!
//! `HostedRuntime` owns a private Tokio runtime and the sidecar's stack without listeners.
//! `open_run` seeds the prelude; `HostedRun::invoke` admits synchronously through `Bridge`,
//! then returns its asynchronous envelope. Stop, deadline and drop converge on one task
//! that drains operations and records the run's end. `close` flushes before shutting down
//! the private executor, including core's otherwise process-long browser tasks.
//! Trace records are real; the screenshot queue is disabled because there is no host camera.

mod hosted_error;
mod hosted_options;
mod hosted_run;
mod hosted_runtime;
mod hosted_stack;
#[cfg(test)]
mod tests;

pub use hosted_error::HostedError;
pub use hosted_options::HostedOptions;
pub use hosted_run::HostedRun;
pub use hosted_runtime::HostedRuntime;
