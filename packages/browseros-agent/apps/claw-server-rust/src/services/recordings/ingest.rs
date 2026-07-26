use super::{RecordingEventInput, RecordingStore};
use crate::{
    error::AppResult,
    services::browser::{BrowserService, TabRegistry},
};
use std::sync::Arc;

pub struct RecordingIngestService {
    recordings: Arc<RecordingStore>,
    browser: Arc<BrowserService>,
    tabs: Arc<TabRegistry>,
}

impl RecordingIngestService {
    pub fn new(
        recordings: Arc<RecordingStore>,
        browser: Arc<BrowserService>,
        tabs: Arc<TabRegistry>,
    ) -> Arc<Self> {
        Arc::new(Self {
            recordings,
            browser,
            tabs,
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
        self.recordings
            .append_batch(
                document_id,
                tab_id,
                target_id.as_deref(),
                events,
                batch_id,
                has_gap,
            )
            .await
    }
}
