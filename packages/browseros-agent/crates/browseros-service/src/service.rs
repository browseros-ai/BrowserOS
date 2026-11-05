//! The host API's composition root. The app hands [`Service::start`] values it already owns
//! and serves [`Service::router`] through [`host_service`](crate::host_service).

use crate::event_log::EventLog;
use crate::router;
use crate::screenshot_taker::Camera;
use axum::Router;
use browseros_core::BrowserSession;
use browseros_core::viewport::Viewports;
use browseros_trace::{Privacy, ShotJobs, TraceSink, TraceStore};
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
    /// never on a connection of their own (decision 21).
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
}

impl Service {
    /// Starts the workers: the event log's numbering task, which subscribes to the writer
    /// before this returns so every event committed from then on gets a number, and the camera
    /// that answers the writer's screenshot jobs.
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
        Camera {
            browser,
            viewports,
            privacy: Arc::clone(&privacy),
        }
        .spawn(screenshots);
        Self {
            state: Arc::new(ServiceState {
                store,
                sink,
                privacy,
                events,
            }),
        }
    }

    /// Every `/v1` route, each response stamped with a request id. Serve it through
    /// [`host_service`](crate::host_service), which puts the host auth in front.
    pub fn router(&self) -> Router {
        router::router(Arc::clone(&self.state))
    }
}
