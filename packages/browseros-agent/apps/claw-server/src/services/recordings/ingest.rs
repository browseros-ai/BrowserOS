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
        let outcome = self
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
        // Fan the freshly accepted events out to any live-preview subscribers,
        // tagged with the document's committed length after this batch. A
        // subscriber forwards the batch only when that length exceeds what its
        // bootstrap already read, so a batch the bootstrap captured is never
        // double-applied. `committed_len` is `Some` only for a newly accepted,
        // non-empty batch, so a duplicate or empty batch is never republished.
        if let Some(committed_len) = outcome.committed_len {
            self.live
                .publish(document_id, committed_len, Arc::from(events))
                .await;
        }
        Ok(outcome.accepted)
    }
}
