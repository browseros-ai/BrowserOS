//! The host API's composition root. The app hands [`Service::start`] values it already owns
//! and serves [`Service::router`] through [`host_service`](crate::host_service).
//!
//! The service also builds the takeover parts once, so the app reaches them through it rather
//! than wiring each one: the lock policy and tab cleanup read [`Service::locks`], and the browser
//! link's tab events go to [`Service::tab_opened`], [`Service::tab_closed`] and
//! [`Service::retain_tabs`], which hand each event to every part that keeps state about tabs.

use crate::event_log::{EventKind, EventLog, LogEvent};
use crate::screenshot_taker::Camera;
use crate::{router, wire};
use axum::Router;
use browseros_core::viewport::Viewports;
use browseros_core::{BrowserSession, TabId};
use browseros_takeover::{BrowserTabs, Locks};
use browseros_trace::{EpochMillis, Privacy, PrivacyError, ShotJobs, TraceSink, TraceStore};
use std::sync::Arc;

/// What the app hands the service: values it already owns, no trait to implement.
pub struct ServiceInputs {
    /// Reads for the record routes and the event log.
    pub store: TraceStore,
    /// The trace writer. Its committed events feed `/v1/events` and `wait=`.
    pub sink: TraceSink,
    /// Tab privacy and the process-wide secret set.
    pub privacy: Arc<Privacy>,
    /// The agent's browser connection (the app's browser link). Screenshots are taken on it,
    /// never on a connection of their own (decision 21), and the takeover parts reach the
    /// browser through it.
    pub browser: Arc<BrowserSession>,
    /// The process's viewport state, so a capture never overlaps a viewport change.
    pub viewports: Arc<Viewports>,
    /// The camera's end of the writer's screenshot queue (`browseros_trace::screenshot_queue`;
    /// `spawn_writer` got the other end).
    pub screenshots: ShotJobs,
}

/// The host API, started. Cheap to clone.
#[derive(Clone)]
pub struct Service {
    state: Arc<ServiceState>,
}

/// What every route reads.
pub(crate) struct ServiceState {
    pub(crate) store: TraceStore,
    pub(crate) sink: TraceSink,
    pub(crate) privacy: Arc<Privacy>,
    pub(crate) events: Arc<EventLog>,
    pub(crate) browser: BrowserTabs,
    pub(crate) locks: Arc<Locks>,
}

impl Service {
    /// Starts the workers: the event log's numbering tasks, which subscribe to the writer and
    /// to the locks before this returns, so every event from then on gets a number, and the
    /// camera that answers the writer's screenshot jobs.
    ///
    /// # Panics
    /// Outside a Tokio runtime, as `tokio::spawn` does.
    #[must_use]
    pub fn start(inputs: ServiceInputs) -> Self {
        let ServiceInputs {
            store,
            sink,
            privacy,
            browser,
            viewports,
            screenshots,
        } = inputs;
        let events = EventLog::start(store.clone(), sink.subscribe());
        let locks = Locks::new(store.clone(), sink.clone());
        events.follow_locks(locks.subscribe());
        Camera {
            browser: Arc::clone(&browser),
            viewports: Arc::clone(&viewports),
            privacy: Arc::clone(&privacy),
        }
        .spawn(screenshots);
        Self {
            state: Arc::new(ServiceState {
                store,
                sink,
                privacy,
                events,
                browser: BrowserTabs::new(browser, viewports),
                locks,
            }),
        }
    }

    /// Every `/v1` route, each response stamped with a request id. Serve it through
    /// [`host_service`](crate::host_service), which puts the host auth in front.
    pub fn router(&self) -> Router {
        router::router(Arc::clone(&self.state))
    }

    /// The tab locks the routes take. The app's lock policy asks [`Locks::check`] about every
    /// agent operation, and its tab cleanup never closes a tab [`Locks::is_locked`] names.
    #[must_use]
    pub fn locks(&self) -> &Arc<Locks> {
        &self.state.locks
    }

    /// The browser link saw a tab open. While a control display is open, the tab starts
    /// private: the owner may already be typing into it.
    ///
    /// # Errors
    /// When the tab's private flag could not be committed. It still holds in memory.
    pub async fn tab_opened(&self, tab: TabId, at: EpochMillis) -> Result<(), PrivacyError> {
        self.state.privacy.tab_opened(tab, at).await
    }

    /// The browser link saw a tab close: its locks end, its private flag is forgotten, and then
    /// `tab.closed` is published, so a host that reads it finds the tab's state already gone.
    pub async fn tab_closed(&self, tab: TabId) {
        self.state.locks.tab_closed(tab);
        self.state.privacy.tab_closed(tab).await;
        self.state.events.publish(LogEvent::new(
            EventKind::TabClosed,
            None,
            Some(tab),
            &wire::tab_closed_event(tab, EpochMillis::now()),
        ));
    }

    /// The browser link (re)connected with `open` tabs. Whatever the parts keep about any other
    /// tab ends, as if it closed: it closed while Neo was not watching, or the browser restarted
    /// and numbers its tabs from 1 again.
    pub async fn retain_tabs(&self, open: &[TabId]) {
        self.state.locks.retain_tabs(open);
        self.state.privacy.retain_tabs(open).await;
    }
}
