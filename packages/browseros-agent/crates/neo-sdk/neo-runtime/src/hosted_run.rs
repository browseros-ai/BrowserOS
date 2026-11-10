//! Neo: An engine-independent SDK door. The host owns JavaScript; Runtime still owns the
//! deadline, browser objects, dispatch, event ordering, and bounded run cleanup.

use std::future::Future;

use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::{oneshot, watch};
use tokio_util::sync::CancellationToken;

use crate::browser_representations::tabs::Tabs;
use crate::chromium_bridge::playwright_injected_methods;
use crate::deadline::Deadline;
use crate::events::CONTEXT_GUID;
use crate::method_router::Dispatcher;
use crate::playwright_methods::browser_methods::BROWSER_GUID;
use crate::quickjs::Script;
use crate::quickjs_bridge::{self, Getters, Logs};
use crate::run::{RunError, RunOutcome};

/// The native half of one SDK instance, usable from a host's non-Tokio JavaScript thread.
/// `get` and `drain` return synchronously; no protocol proxy or second JavaScript engine is involved.
pub struct HostedRun {
    init: Value,
    getters: Getters,
    dispatcher: Dispatcher,
    tabs: Tabs,
    logs: Logs,
    tokio: Handle,
    done: CancellationToken,
    closed: CancellationToken,
    outcome: watch::Receiver<Option<RunOutcome>>,
}

pub(crate) struct Host {
    pub(crate) ready: oneshot::Sender<HostedRun>,
    pub(crate) done: CancellationToken,
    pub(crate) outcome: watch::Receiver<Option<RunOutcome>>,
}

impl Host {
    pub(crate) async fn run(
        self,
        script: Script,
        getters: Getters,
        dispatcher: Dispatcher,
        deadline: Deadline,
        tabs: Tabs,
    ) -> RunOutcome {
        let logs = Logs::default();
        let events = tabs.events().clone();
        let closed = CancellationToken::new();
        let run = HostedRun {
            init: json!({
                "page": script.page_guid, "mainFrame": script.frame_guid,
                "context": CONTEXT_GUID, "browser": BROWSER_GUID, "url": script.url,
                "testIdAttribute": playwright_injected_methods::TEST_ID_ATTRIBUTE,
                "lineOffset": 0,
            }),
            getters,
            dispatcher,
            tabs,
            logs: logs.clone(),
            tokio: Handle::current(),
            done: self.done.clone(),
            closed: closed.clone(),
            outcome: self.outcome,
        };
        // A cancelled open drops the receiver; dropping its returned door ends this run too.
        let _ = self.ready.send(run);
        tokio::select! {
            () = self.done.cancelled() => {},
            _ = deadline.reached() => {},
        }
        closed.cancel();
        events.close();
        for note in events.take_notes() {
            logs.push(note);
        }
        if let Some(stop) = deadline.stop() {
            RunOutcome::stopped(stop, deadline.timeout(), logs.take())
        } else {
            RunOutcome::returned(None, logs.take())
        }
    }
}

impl HostedRun {
    /// Values passed unchanged to the SDK bundle's `createRun`.
    #[must_use]
    pub fn init(&self) -> &Value {
        &self.init
    }

    /// Reads a live value on the caller's thread, sharing QuickJS's getter and error mapping.
    ///
    /// # Errors
    /// Returns the native getter error, or an error once the run has ended.
    pub fn get(&self, request: &str) -> Result<String, RunError> {
        self.active()?;
        self.getters.get_text(request)
    }

    /// Starts the dispatch before returning, preserving JS call order, then waits only for
    /// its reply and event watermark. The temporary runtime entry never encloses a getter.
    ///
    /// # Errors
    /// Rejects an unreadable protocol envelope or a run that ends before replying.
    pub fn call_with_events(
        &self,
        message: &str,
    ) -> impl Future<Output = Result<(String, u64), RunError>> + Send + 'static {
        let reply = self.active().map(|()| {
            let _entered = self.tokio.enter();
            self.dispatcher.call(message)
        });
        async move {
            reply?
                .await
                .map_err(|error| RunError::new("Error", error.to_string()))
        }
    }

    /// Cancels the SDK call with this id; unknown and already-settled ids are ignored.
    pub fn abort(&self, id: u64, reason: String) {
        self.dispatcher.abort(id, reason);
    }

    /// Changes an event subscription through the same producers as QuickJS.
    ///
    /// # Errors
    /// Rejects a malformed request, unsupported event, or ended run.
    pub fn subscribe(&self, request: &str) -> Result<(), RunError> {
        self.active()?;
        let _entered = self.tokio.enter();
        quickjs_bridge::subscribe(&self.tabs, request)
            .map_err(|message| RunError::new("Error", message))
    }

    /// Removes events through a watermark; a negative watermark takes every queued event.
    #[must_use]
    pub fn drain(&self, until: i64) -> String {
        let events = self.tabs.events();
        let text = events.drain(u64::try_from(until).ok());
        for note in events.take_notes() {
            self.logs.push(note);
        }
        text
    }

    /// Waits for idle events. Shutdown settles the pending host promise, so it cannot retain
    /// Node's event loop. The host must stop the SDK's idle pump when this rejects.
    ///
    /// # Errors
    /// Returns an ended-run error at finish or the run deadline.
    pub async fn next_events(&self) -> Result<(), RunError> {
        tokio::select! {
            biased;
            () = self.closed.cancelled() => Err(ended()),
            () = self.tabs.events().next() => Ok(()),
        }
    }

    /// Adds one line to the run's existing bounded log.
    pub fn log(&self, line: String) {
        self.logs.push(line);
    }

    /// Ends the host script, waits for the shared dispatcher's cleanup, and returns its report.
    /// Repeated calls return the same outcome.
    ///
    /// # Errors
    /// Returns an error if the lifecycle task disappeared without an outcome.
    pub async fn finish(&self) -> Result<RunOutcome, RunError> {
        self.done.cancel();
        outcome(self.outcome.clone()).await
    }

    fn active(&self) -> Result<(), RunError> {
        if self.done.is_cancelled() || self.closed.is_cancelled() {
            Err(ended())
        } else {
            Ok(())
        }
    }
}

impl Drop for HostedRun {
    fn drop(&mut self) {
        self.done.cancel();
    }
}

fn ended() -> RunError {
    RunError::new("Error", "Hosted run has ended".to_string())
}

pub(crate) async fn outcome(
    mut receiver: watch::Receiver<Option<RunOutcome>>,
) -> Result<RunOutcome, RunError> {
    loop {
        if let Some(outcome) = receiver.borrow().clone() {
            return Ok(outcome);
        }
        receiver.changed().await.map_err(|_| {
            RunError::new("Error", "Hosted run ended without an outcome".to_string())
        })?;
    }
}
