use crate::{
    db::SessionTabLedger,
    error::{AppError, AppResult},
    ids::SessionId,
    services::{
        browser::BrowserService,
        cockpit::{TabActivityRecord, TabActivityService},
        sessions::Sessions,
    },
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use browseros_core::{
    PageId, TargetId,
    pages::PageInfo,
    screenshot::{ScreenshotCaptureOptions, ScreenshotFormat},
};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::time::timeout;

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone)]
struct CaptureCandidate {
    tab_id: i64,
    page_id: u32,
    target_id: String,
    last_activity_at: Option<i64>,
}

/// Resolves a live session's current owned tab and captures it through browser-core's serialized screenshot path.
pub struct SessionVisualService {
    sessions: Arc<Sessions>,
    session_tabs: Arc<SessionTabLedger>,
    browser: Arc<BrowserService>,
    tab_activity: Arc<TabActivityService>,
}

impl SessionVisualService {
    #[must_use]
    pub fn new(
        sessions: Arc<Sessions>,
        session_tabs: Arc<SessionTabLedger>,
        browser: Arc<BrowserService>,
        tab_activity: Arc<TabActivityService>,
    ) -> Arc<Self> {
        Arc::new(Self {
            sessions,
            session_tabs,
            browser,
            tab_activity,
        })
    }

    pub async fn capture(&self, session_id: &str) -> AppResult<Option<Vec<u8>>> {
        let session_key = SessionId::new(session_id);
        if !self.sessions.contains(&session_key).await {
            return Ok(None);
        }
        self.session_tabs.drain_writes().await;
        let browser = match self.browser.session().await {
            Some(browser) => browser,
            None => return Ok(None),
        };
        let pages = browser
            .pages
            .list()
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let ownership = self
            .session_tabs
            .list_open_session_tabs(&[session_id.to_string()])
            .await?;
        let activity = self.tab_activity.reconcile_pages(&pages).await;
        let Some(candidate) = select_candidate(session_id, &ownership, &pages, &activity) else {
            return Ok(None);
        };

        let page_id = PageId(candidate.page_id);
        let target_id = TargetId::from(candidate.target_id);
        let options = capture_options();
        let capture_browser = browser.clone();
        let mut capture = tokio::spawn(async move {
            capture_browser
                .screenshot_for_target(page_id, &target_id, options)
                .await
        });
        let result = match timeout(CAPTURE_TIMEOUT, &mut capture).await {
            Ok(result) => result.map_err(AppError::from)?,
            Err(_) => {
                tokio::spawn(async move {
                    let _ = capture.await;
                });
                return Err(AppError::Internal(
                    "session preview capture timed out".to_string(),
                ));
            }
        }
        .map_err(|error| AppError::Internal(error.to_string()))?;
        let Some(capture) = result else {
            return Ok(None);
        };
        let bytes = STANDARD
            .decode(capture.data)
            .map_err(|error| AppError::Internal(format!("invalid screenshot data: {error}")))?;
        if bytes.is_empty() {
            return Ok(None);
        }

        self.session_tabs.drain_writes().await;
        let owns_tab = self
            .session_tabs
            .open_session_tab(session_id, candidate.tab_id)
            .await?
            .is_some();
        if !owns_tab || !self.sessions.contains(&session_key).await {
            return Ok(None);
        }
        Ok(Some(bytes))
    }
}

fn select_candidate(
    session_id: &str,
    ownership: &[crate::db::entities::session_tabs::Model],
    pages: &[PageInfo],
    activity: &[TabActivityRecord],
) -> Option<CaptureCandidate> {
    let pages_by_tab = pages
        .iter()
        .map(|page| (page.tab_id.0, page))
        .collect::<HashMap<_, _>>();
    let activity_by_incarnation = activity
        .iter()
        .filter(|record| record.session_id == session_id)
        .map(|record| {
            (
                (record.tab_id, record.page_id, record.target_id.as_str()),
                record.last_tool_at,
            )
        })
        .collect::<HashMap<_, _>>();
    let mut candidates = ownership
        .iter()
        .filter(|row| row.session_id == session_id)
        .filter_map(|row| {
            let page = pages_by_tab.get(&row.tab_id)?;
            Some(CaptureCandidate {
                tab_id: row.tab_id,
                page_id: page.page_id.0,
                target_id: page.target_id.as_str().to_string(),
                last_activity_at: activity_by_incarnation
                    .get(&(row.tab_id, page.page_id.0, page.target_id.as_str()))
                    .copied(),
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .last_activity_at
            .cmp(&left.last_activity_at)
            .then_with(|| left.tab_id.cmp(&right.tab_id))
            .then_with(|| left.page_id.cmp(&right.page_id))
    });
    candidates.into_iter().next()
}

fn capture_options() -> ScreenshotCaptureOptions {
    ScreenshotCaptureOptions {
        format: Some(ScreenshotFormat::Jpeg),
        quality: Some(50),
        full_page: Some(false),
        annotate: Some(false),
        clip: None,
    }
}

#[cfg(test)]
mod tests {
    use super::capture_options;
    use browseros_core::screenshot::ScreenshotFormat;

    #[test]
    fn preview_capture_is_viewport_jpeg_without_annotations() {
        let options = capture_options();
        assert_eq!(options.format, Some(ScreenshotFormat::Jpeg));
        assert_eq!(options.quality, Some(50));
        assert_eq!(options.full_page, Some(false));
        assert_eq!(options.annotate, Some(false));
        assert_eq!(options.clip, None);
    }
}
