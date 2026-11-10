//! Neo: An engine-independent SDK door. The host owns JavaScript; Runtime still owns the
//! deadline, browser objects, dispatch, event ordering, and bounded run cleanup.

use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::{oneshot, watch};
use tokio::time;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::browser_representations::tabs::Tabs;
use crate::chromium_bridge::cdp::{Browser, target};
use crate::chromium_bridge::playwright_injected_methods;
use crate::deadline::{Deadline, Stop};
use crate::events::CONTEXT_GUID;
use crate::method_router::Dispatcher;
use crate::playwright_methods::browser_methods::BROWSER_GUID;
use crate::progress::RunScope;
use crate::quickjs::Script;
use crate::quickjs_bridge::{self, Getters, Logs};
use crate::run::{PageId, RunError, RunOutcome, RunPage, RunStatus};

/// Neo: The CDP client exposes connection state without a wake; only hosted lifetimes poll it.
const CONNECTION_POLL: Duration = Duration::from_millis(250);
/// Neo: The host-owned starting tab has its own bounded close after ordinary run cleanup.
const CLOSE_WAIT: Duration = Duration::from_secs(5);

/// Neo: Shared admission and cancellation for hosted runs only; ordinary runs are unaffected.
#[derive(Clone, Default)]
pub(crate) struct Runs {
    pub(crate) cancel: CancellationToken,
    pub(crate) tasks: TaskTracker,
}

/// Neo: The native half of one SDK instance, usable from a host's non-Tokio JavaScript thread.
/// `get` and `drain` return synchronously. Finish and cancellation also wake in-flight getters.
pub struct HostedRun {
    init: Value,
    getters: Getters,
    dispatcher: Dispatcher,
    tabs: Tabs,
    logs: Logs,
    tokio: Handle,
    state: State,
    outcome: watch::Receiver<Option<RunOutcome>>,
}

/// Neo: Futures borrow cancellation state, not ownership of the host's run wrapper.
#[derive(Clone)]
struct State {
    done: CancellationToken,
    cancel: CancellationToken,
    closed: CancellationToken,
    deadline: Deadline,
    error: Arc<OnceLock<RunError>>,
}

/// Neo: One startup handoff and one completion report for the runtime's tracked lifecycle task.
pub(crate) struct Host {
    pub(crate) ready: oneshot::Sender<HostedRun>,
    pub(crate) cancel: CancellationToken,
    pub(crate) getter_stop: CancellationToken,
    pub(crate) outcome: watch::Receiver<Option<RunOutcome>>,
}

/// Neo: Publish the door once, then end it independently of the host engine's cooperation.
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
        let browser = tabs.cdp().clone();
        let state = State {
            done: CancellationToken::new(),
            cancel: self.cancel,
            closed: self.getter_stop,
            deadline: deadline.clone(),
            error: Arc::default(),
        };
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
            state: state.clone(),
            outcome: self.outcome,
        };
        // If open was abandoned after startup, the undelivered door's Drop cancels the run.
        let _ = self.ready.send(run);
        let mut outcome = tokio::select! {
            biased;
            stop = deadline.reached() => RunOutcome::stopped(stop, deadline.timeout(), Vec::new()),
            () = state.done.cancelled() => RunOutcome::returned(None, Vec::new()),
            () = disconnected(&browser) => RunOutcome::failed(RunError::target_closed(), Vec::new()),
        };
        let error = outcome.error.clone().unwrap_or_else(|| {
            if outcome.status == RunStatus::Cancelled {
                cancelled()
            } else {
                RunError::target_closed()
            }
        });
        let _ = state.error.set(error);
        state.closed.cancel();
        events.close();
        for note in events.take_notes() {
            logs.push(note);
        }
        outcome.logs = logs.take();
        outcome
    }
}

/// Neo: Every door entry observes the same terminal error, including after cleanup finishes.
impl State {
    fn active(&self) -> Result<(), RunError> {
        if let Some(error) = self.error.get() {
            return Err(error.clone());
        }
        match self.deadline.stop() {
            Some(Stop::Timeout) => Err(RunError::timeout(self.deadline.timeout())),
            Some(Stop::Cancelled) => Err(cancelled()),
            None if self.done.is_cancelled() => Err(RunError::target_closed()),
            None => Ok(()),
        }
    }
}

/// Neo: The same getters, dispatcher, subscriptions and event queue used by the QuickJS door.
impl HostedRun {
    /// Values passed unchanged to the SDK bundle's `createRun`.
    #[must_use]
    pub fn init(&self) -> &Value {
        &self.init
    }

    /// Reads a live value on the caller's thread, sharing QuickJS's getter and error mapping.
    ///
    /// # Errors
    /// Returns the native getter error, or the run's terminal error.
    pub fn get(&self, request: &str) -> Result<String, RunError> {
        self.state.active()?;
        self.getters.get_text(request).or_else(|error| {
            self.state.active()?;
            Err(error)
        })
    }

    /// Starts dispatch before returning, preserving JS call order. Only the reply is async.
    ///
    /// # Errors
    /// Rejects an unreadable protocol envelope or a run that ends before replying.
    pub fn call_with_events(
        &self,
        message: &str,
    ) -> impl Future<Output = Result<(String, u64), RunError>> + Send + 'static {
        let reply = self.state.active().map(|()| {
            let _entered = self.tokio.enter();
            self.dispatcher.call(message)
        });
        let state = self.state.clone();
        async move {
            let reply = reply?;
            tokio::select! {
                biased;
                () = state.closed.cancelled() => {
                    state.active()?;
                    Err(RunError::target_closed())
                },
                reply = reply => {
                    state.active()?;
                    reply.map_err(|error| RunError::new("Error", error.to_string()))
                },
            }
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
        self.state.active()?;
        let _entered = self.tokio.enter();
        quickjs_bridge::subscribe(&self.tabs, request)
            .map_err(|message| RunError::new("Error", message))
    }

    /// Removes events through a watermark; a negative watermark takes every queued event.
    ///
    /// # Errors
    /// Returns the run's terminal error after finish, cancellation or its deadline.
    pub fn drain(&self, until: i64) -> Result<String, RunError> {
        self.state.active()?;
        let events = self.tabs.events();
        let text = events.drain(u64::try_from(until).ok());
        for note in events.take_notes() {
            self.logs.push(note);
        }
        Ok(text)
    }

    /// Waits for idle events without retaining the host's owner. Shutdown settles this future.
    ///
    /// # Errors
    /// Returns the run's terminal error or a closed-context error.
    pub fn next_events(&self) -> impl Future<Output = Result<(), RunError>> + Send + 'static {
        let state = self.state.clone();
        let events = self.tabs.events().clone();
        async move {
            state.active()?;
            tokio::select! {
                biased;
                () = state.closed.cancelled() => state.active(),
                () = events.next() => {
                    state.active()?;
                    if events.subscriptions_ended() { Err(RunError::target_closed()) } else { Ok(()) }
                },
            }
        }
    }

    /// Adds one line to the run's existing bounded log.
    pub fn log(&self, line: String) {
        if self.state.active().is_ok() {
            self.logs.push(line);
        }
    }

    /// Cancels without waiting, including when the host drops its wrapper with promises open.
    pub fn cancel(&self) {
        if !self.state.done.is_cancelled() {
            self.state.cancel.cancel();
        }
    }

    /// Ends the host script, waits for native cleanup, and returns the same report on repeats.
    ///
    /// # Errors
    /// Returns an error if the lifecycle task disappeared without an outcome.
    pub fn finish(&self) -> impl Future<Output = Result<RunOutcome, RunError>> + Send + 'static {
        self.state.done.cancel();
        outcome(self.outcome.clone())
    }
}

/// Neo: The host's last owner cancels even when no explicit finish was requested.
impl Drop for HostedRun {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Neo: Matches the synchronous getter's existing cancellation error.
fn cancelled() -> RunError {
    RunError::new("Error", "Run cancelled".to_string())
}

/// Neo: A browser that disappears ends an idle hosted run as well as its active calls.
async fn disconnected(browser: &Browser) {
    while browser.is_connected() {
        time::sleep(CONNECTION_POLL).await;
    }
}

/// Neo: Only hosted getters use a separate finish wake and bounded native handles.
pub(crate) fn getters(host: Option<&Host>, run: &RunScope, tabs: Tabs, tokio: Handle) -> Getters {
    let deadline = host.map_or_else(
        || run.deadline.clone(),
        |host| {
            run.handles.limit_hosted();
            run.deadline.with_cancellation(host.getter_stop.clone())
        },
    );
    Getters::new(tabs, deadline, tokio, Arc::clone(&run.handles))
}

/// Neo: Hosted startup owns this scratch tab; ordinary `Runtime::run` tabs still survive.
pub(crate) async fn close_page(
    owned: Option<(Tabs, PageId)>,
    mut pages: Vec<RunPage>,
) -> Vec<RunPage> {
    let Some((tabs, page)) = owned else {
        return pages;
    };
    let closed = matches!(
        time::timeout(
            CLOSE_WAIT,
            tabs.cdp().send(target::CloseTarget {
                target_id: page.as_str().to_string(),
            }),
        )
        .await,
        Ok(Ok(_))
    );
    if closed && let Some(record) = pages.iter_mut().find(|record| record.id == page.as_str()) {
        record.closed = true;
    }
    pages
}

/// Neo: Completion is retained for simultaneous and repeated finish calls.
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
