use super::{LiveRecordingBus, RecordingEventInput, RecordingStore};
use crate::{
    error::AppResult,
    services::browser::{BrowserService, TabRegistry},
};
use std::sync::Arc;

pub struct RecordingIngestService {
    recordings: Arc<RecordingStore>,
    browser: Arc<BrowserService>,
    tabs: Arc<TabRegistry>,
    live: Arc<LiveRecordingBus>,
}

impl RecordingIngestService {
    pub fn new(
        recordings: Arc<RecordingStore>,
        browser: Arc<BrowserService>,
        tabs: Arc<TabRegistry>,
        live: Arc<LiveRecordingBus>,
    ) -> Arc<Self> {
        Arc::new(Self {
            recordings,
            browser,
            tabs,
            live,
        })
    }

    pub async fn append_document(
        &self,
        document_id: &str,
        tab_id: i64,
        events: &[RecordingEventInput],
        batch_id: &str,
        has_gap: bool,
    ) -> AppResult<bool> {
        let session = self.browser.session().await;
        let target_id = self
            .tabs
            .resolve(tab_id, session, self.browser.state().epoch)
            .await;
        let accepted = self
            .recordings
            .append_batch(
                document_id,
                tab_id,
                target_id.as_deref(),
                events,
                batch_id,
                has_gap,
            )
            .await?;
        // Fan the freshly accepted events out to any live-preview subscribers.
        // A duplicate (already-accepted) batch is not republished so a
        // reconnecting subscriber never double-applies it.
        if accepted && !events.is_empty() {
            self.live.publish(document_id, Arc::from(events)).await;
        }
        Ok(accepted)
    }
}
