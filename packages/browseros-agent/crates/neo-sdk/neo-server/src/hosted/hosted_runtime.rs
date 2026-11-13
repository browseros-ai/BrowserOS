use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use browseros_core::NativeCapabilities;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::OnceCell;

use super::hosted_stack::HostedStack;
use super::{HostedError, HostedOptions, HostedRun};
use crate::App;
use crate::browser::{BrowserLink, TabRegistry};

/// A browser connection, its real server stack, and its private executor. Explicitly `close`
/// to drain runs and flush traces; dropping an abandoned host releases its executor and sockets.
pub struct HostedRuntime {
    stack: Arc<HostedStack>,
    executor: Executor,
    closed: OnceCell<Result<(), HostedError>>,
}

/// Shutdown must never block Node or drop a Tokio runtime inside another runtime's async task.
struct Executor {
    runtime: Mutex<Option<Runtime>>,
    handle: Handle,
}

impl Executor {
    fn new() -> Result<Self, HostedError> {
        let runtime = App::runtime().map_err(HostedError::from_error)?;
        Ok(Self {
            handle: runtime.handle().clone(),
            runtime: Mutex::new(Some(runtime)),
        })
    }

    fn stop(&self) {
        if let Some(runtime) = self
            .runtime
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            runtime.shutdown_background();
        }
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        self.stop();
    }
}

impl HostedRuntime {
    /// Starts the product's browser link to loopback CDP, and resolves after it connects.
    /// The caller owns the browser, and `options.hosted_mode` must say whether it launched it
    /// with `BROWSERCLAW_HOSTED_MODE=1`: a mismatch refuses console and page-error waits the
    /// browser would answer, or waits for ones hosted BrowserOS never sends.
    pub async fn connect(cdp_port: u16, options: HostedOptions) -> Result<Self, HostedError> {
        Self::start(options, move |native, tabs| {
            BrowserLink::start(cdp_port, native, tabs)
        })
        .await
    }

    pub(super) async fn start(
        options: HostedOptions,
        link: impl FnOnce(NativeCapabilities, Arc<TabRegistry>) -> Arc<BrowserLink> + Send + 'static,
    ) -> Result<Self, HostedError> {
        let executor = Executor::new()?;
        let stack = executor
            .handle
            .spawn(HostedStack::start(options, link))
            .await
            .map_err(HostedError::from_error)??;
        Ok(Self {
            stack,
            executor,
            closed: OnceCell::new(),
        })
    }

    /// Opens a run with the product's initial tab listing. The caller chooses its deadline;
    /// a test worker usually uses 24 hours and leaves individual action timeouts to the SDK.
    pub async fn open_run(&self, timeout: Duration) -> Result<HostedRun, HostedError> {
        let stack = Arc::clone(&self.stack);
        self.executor
            .handle
            .spawn(async move { stack.open_run(timeout).await })
            .await
            .map_err(HostedError::from_error)?
    }

    /// Stops all runs, drains their operations, flushes traces, then releases all background
    /// browser tasks and the CDP socket. Repeated and concurrent closes share one result.
    pub async fn close(&self) -> Result<(), HostedError> {
        self.stack.stop();
        self.closed
            .get_or_init(|| async {
                let stack = Arc::clone(&self.stack);
                let result = self
                    .executor
                    .handle
                    .spawn(async move { stack.close().await })
                    .await
                    .map_err(HostedError::from_error)
                    .and_then(|result| result);
                self.executor.stop();
                result
            })
            .await
            .clone()
    }
}

impl Drop for HostedRuntime {
    fn drop(&mut self) {
        self.stack.stop();
    }
}

impl fmt::Debug for HostedRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostedRuntime")
            .field("closed", &self.closed.initialized())
            .finish_non_exhaustive()
    }
}
