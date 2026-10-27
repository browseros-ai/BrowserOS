use crate::{
    db::SessionTabLedger,
    error::{AppError, AppResult},
    ids::SessionId,
    services::{
        browser::BrowserService,
        cockpit::{TabActivityRecord, TabActivityService},
        sessions::{Session, Sessions},
    },
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use browseros_core::{
    BrowserSession, PageId, TargetId,
    pages::PageInfo,
    screenshot::{ScreenshotCaptureOptions, ScreenshotFormat},
    snapshot::DocumentId,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio::{sync::Mutex, time::timeout};

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(2);
const CAPTURE_TASK_CONCURRENCY_LIMIT: usize = 2;

#[derive(Clone)]
struct CaptureCandidate {
    tab_id: i64,
    page_id: u32,
    target_id: String,
    url: String,
    last_activity_at: Option<i64>,
}

/// One tab a session has open, as live views address it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionTab {
    pub tab_id: i64,
    pub target_id: String,
    pub url: String,
}

#[derive(Default)]
struct InFlightCaptures {
    page_ids: Mutex<HashSet<u32>>,
}

impl InFlightCaptures {
    async fn begin(&self, page_id: u32) -> bool {
        let mut page_ids = self.page_ids.lock().await;
        if page_ids.len() >= CAPTURE_TASK_CONCURRENCY_LIMIT {
            return false;
        }
        page_ids.insert(page_id)
    }

    async fn finish(&self, page_id: u32) {
        self.page_ids.lock().await.remove(&page_id);
    }
}

/// One card fill's hold on a page: the main-frame document it wrote into
/// (`None` when that couldn't be read), and a generation that tells this hold
/// apart from a newer fill's.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CardHold {
    generation: u64,
    document: Option<DocumentId>,
}

/// Pages a card fill wrote into. A filled card number shows in plain text, so
/// captures skip such a page until it shows another document.
#[derive(Default)]
struct CardDocuments {
    held: Mutex<HashMap<u32, CardHold>>,
    next_generation: std::sync::atomic::AtomicU64,
}

impl CardDocuments {
    async fn hold(&self, page_id: u32, document: Option<DocumentId>) {
        let generation = self
            .next_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.held.lock().await.insert(
            page_id,
            CardHold {
                generation,
                document,
            },
        );
    }

    /// The hold on a page, after dropping holds on closed pages.
    async fn held(&self, page_id: u32, open: &[PageInfo]) -> Option<CardHold> {
        let mut held = self.held.lock().await;
        held.retain(|page, _| open.iter().any(|info| info.page_id.0 == *page));
        held.get(&page_id).cloned()
    }

    async fn holds(&self, page_id: u32) -> bool {
        self.held.lock().await.contains_key(&page_id)
    }

    /// Releases `checked` once the page shows another document, and says
    /// whether a capture may proceed. A newer hold, or a document that can't
    /// be read on either side, keeps the page held.
    async fn release_if_changed(
        &self,
        page_id: u32,
        checked: &CardHold,
        current: Option<DocumentId>,
    ) -> bool {
        let changed = matches!(
            (&checked.document, &current),
            (Some(filled), Some(current)) if filled != current
        );
        if !changed {
            return false;
        }
        let mut held = self.held.lock().await;
        match held.get(&page_id) {
            Some(hold) if hold.generation == checked.generation => {
                held.remove(&page_id);
                true
            }
            Some(_) => false,
            None => true,
        }
    }
}

async fn main_document_id(browser: &BrowserSession, page_id: u32) -> Option<DocumentId> {
    timeout(CAPTURE_TIMEOUT, browser.main_document_id(PageId(page_id)))
        .await
        .ok()
        .and_then(Result::ok)
        .flatten()
}

/// Resolves a live session's current owned tab and captures it through browser-core's serialized screenshot path.
pub struct SessionVisualService {
    sessions: Arc<Sessions>,
    session_tabs: Arc<SessionTabLedger>,
    browser: Arc<BrowserService>,
    tab_activity: Arc<TabActivityService>,
    viewports: Option<Arc<browseros_core::viewport::Viewports>>,
    /// Entries retain the process-wide capture slot until the underlying CDP task resolves, even
    /// after caller timeout, so stuck tasks cannot escape the concurrency bound.
    in_flight: Arc<InFlightCaptures>,
    /// Every capture path (audit rows, script steps, live previews) passes the
    /// candidate page through this before shooting it.
    card_documents: CardDocuments,
}

impl SessionVisualService {
    #[must_use]
    pub fn new(
        sessions: Arc<Sessions>,
        session_tabs: Arc<SessionTabLedger>,
        browser: Arc<BrowserService>,
        tab_activity: Arc<TabActivityService>,
        viewports: Option<Arc<browseros_core::viewport::Viewports>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            sessions,
            session_tabs,
            browser,
            tab_activity,
            viewports,
            in_flight: Arc::new(InFlightCaptures::default()),
            card_documents: CardDocuments::default(),
        })
    }

    /// Keeps the page out of captures until its main-frame document changes.
    /// Call before a card fill: the number then shows in plain text, and a
    /// capture would store it in an audit screenshot.
    pub async fn hold_card_data(&self, page_id: u32) {
        let document = match self.browser.session().await {
            Some(browser) => main_document_id(&browser, page_id).await,
            None => None,
        };
        self.card_documents.hold(page_id, document).await;
    }

    pub async fn capture(
        &self,
        session_id: &str,
        browser_tab_id: Option<i64>,
    ) -> AppResult<Option<Vec<u8>>> {
        let session_key = SessionId::new(session_id);
        let Some(session) = self.sessions.lookup(&session_key).await else {
            return Ok(None);
        };
        self.capture_with_session(&session, true, browser_tab_id)
            .await
    }

    /// The session's open, live tabs, the current one first: the order the preview picks from,
    /// so every live view agrees on which tab a session is "on".
    pub async fn ranked_tabs(&self, session_id: &str) -> AppResult<Vec<SessionTab>> {
        self.session_tabs.drain_writes().await;
        let Some(browser) = self.browser.session().await else {
            return Ok(Vec::new());
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
        Ok(
            rank_candidates(session_id, &ownership, &pages, &activity, None)
                .into_iter()
                .map(|candidate| SessionTab {
                    tab_id: candidate.tab_id,
                    target_id: candidate.target_id,
                    url: candidate.url,
                })
                .collect(),
        )
    }

    /// Captures through a teardown-owned session lease after live request resolution has stopped.
    pub async fn capture_for_session(&self, session: &Arc<Session>) -> AppResult<Option<Vec<u8>>> {
        self.capture_with_session(session, false, None).await
    }

    async fn capture_with_session(
        &self,
        session: &Arc<Session>,
        require_live_at_end: bool,
        browser_tab_id: Option<i64>,
    ) -> AppResult<Option<Vec<u8>>> {
        let session_id = session.id().as_str();
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
        let Some(candidate) =
            select_candidate(session_id, &ownership, &pages, &activity, browser_tab_id)
        else {
            return Ok(None);
        };
        // A JavaScript dialog stalls the renderer: the capture would only wait on it while
        // holding the page's window.
        if browser
            .page_signals
            .pending_dialog(&PageId(candidate.page_id))
            .is_some()
        {
            return Ok(None);
        }
        if let Some(hold) = self.card_documents.held(candidate.page_id, &pages).await {
            let current = main_document_id(&browser, candidate.page_id).await;
            if !self
                .card_documents
                .release_if_changed(candidate.page_id, &hold, current)
                .await
            {
                return Ok(None);
            }
        }
        if !self.in_flight.begin(candidate.page_id).await {
            return Ok(None);
        }

        let page_id = PageId(candidate.page_id);
        let target_id = TargetId::from(candidate.target_id);
        let options = capture_options();
        let capture_browser = browser.clone();
        let viewports = self.viewports.clone();
        let worker = tokio::spawn(async move {
            // A capture rewrites an emulated page's viewport until it finishes, then restores
            // what it saw first; viewport changes and fixed tabs' operations must not overlap.
            let _held = match viewports {
                Some(viewports) => Some(
                    browseros_core::BrowserView::unbound(capture_browser.clone(), None)
                        .with_viewports(viewports)
                        .hold_viewport(page_id.clone())
                        .await,
                ),
                None => None,
            };
            capture_browser
                .screenshot_for_target(page_id, &target_id, options)
                .await
        });
        // The HTTP handler can disappear when a card hides or changes tabs.
        // A detached supervisor owns cleanup: keep the slot until CDP finishes,
        // then release it even if the requester is gone or the worker panics.
        let in_flight = self.in_flight.clone();
        let mut capture = tokio::spawn(async move {
            let result = worker.await;
            in_flight.finish(candidate.page_id).await;
            result
        });
        let result = match timeout(CAPTURE_TIMEOUT, &mut capture).await {
            Ok(result) => result.map_err(AppError::from)?.map_err(AppError::from)?,
            Err(_) => {
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
        // A fill that started while the screenshot was taken holds the page
        // before it dispatches, so the image may show its card.
        if bytes.is_empty() || self.card_documents.holds(candidate.page_id).await {
            return Ok(None);
        }

        self.session_tabs.drain_writes().await;
        let owns_tab = self
            .session_tabs
            .open_session_tab(session_id, candidate.tab_id)
            .await?
            .is_some();
        if !owns_tab
            || (require_live_at_end && !self.sessions.contains(&SessionId::new(session_id)).await)
        {
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
    browser_tab_id: Option<i64>,
) -> Option<CaptureCandidate> {
    rank_candidates(session_id, ownership, pages, activity, browser_tab_id)
        .into_iter()
        .next()
}

/// The session's owned tabs that are live pages, most recently used by a tool first.
fn rank_candidates(
    session_id: &str,
    ownership: &[crate::db::entities::session_tabs::Model],
    pages: &[PageInfo],
    activity: &[TabActivityRecord],
    browser_tab_id: Option<i64>,
) -> Vec<CaptureCandidate> {
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
        // Selection narrows session ownership; it never bypasses it or falls
        // back to a different tab under the pinned card's address/caption.
        .filter(|row| browser_tab_id.is_none_or(|tab_id| row.tab_id == tab_id))
        .filter_map(|row| {
            let page = pages_by_tab.get(&row.tab_id)?;
            Some(CaptureCandidate {
                tab_id: row.tab_id,
                page_id: page.page_id.0,
                target_id: page.target_id.as_str().to_string(),
                url: page.url.clone(),
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
    candidates
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
    use super::{CardDocuments, InFlightCaptures, capture_options};
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

    #[tokio::test]
    async fn page_capture_stays_single_flight_until_the_worker_finishes() {
        let captures = InFlightCaptures::default();
        assert!(captures.begin(7).await);
        assert!(!captures.begin(7).await);
        assert!(captures.begin(8).await);
        assert!(!captures.begin(9).await);

        captures.finish(7).await;
        assert!(captures.begin(9).await);
    }

    fn page(id: u32) -> browseros_core::pages::PageInfo {
        browseros_core::pages::PageInfo {
            page_id: browseros_core::PageId(id),
            target_id: browseros_core::TargetId::from(format!("target-{id}")),
            tab_id: browseros_core::TabId(i64::from(id)),
            url: "https://shop.test/checkout".into(),
            title: "Checkout".into(),
            is_active: true,
            is_loading: false,
            load_progress: 1.0,
            is_pinned: false,
            window_id: None,
            index: None,
            group_id: None,
            browser_profile_id: None,
            live_browser_context_id: None,
        }
    }

    #[tokio::test]
    async fn card_pages_stay_held_until_their_document_changes() -> anyhow::Result<()> {
        let cards = CardDocuments::default();
        let open = [page(1), page(2)];
        assert_eq!(cards.held(1, &open).await, None);

        cards.hold(1, Some("main:loader-a".into())).await;
        let checked = cards
            .held(1, &open)
            .await
            .ok_or_else(|| anyhow::anyhow!("page 1 not held"))?;
        // The same document, or one that can't be read, keeps the page held.
        assert!(
            !cards
                .release_if_changed(1, &checked, Some("main:loader-a".into()))
                .await
        );
        assert!(!cards.release_if_changed(1, &checked, None).await);
        assert!(cards.holds(1).await);
        // A new document releases it.
        assert!(
            cards
                .release_if_changed(1, &checked, Some("main:loader-b".into()))
                .await
        );
        assert!(!cards.holds(1).await);

        // An unreadable fill document holds until the page closes.
        cards.hold(2, None).await;
        let checked = cards
            .held(2, &open)
            .await
            .ok_or_else(|| anyhow::anyhow!("page 2 not held"))?;
        assert!(
            !cards
                .release_if_changed(2, &checked, Some("main:loader-c".into()))
                .await
        );
        assert_eq!(cards.held(2, &[page(1)]).await, None);
        Ok(())
    }

    #[tokio::test]
    async fn a_newer_fill_keeps_its_hold_through_a_stale_check() -> anyhow::Result<()> {
        let cards = CardDocuments::default();
        let open = [page(1)];
        cards.hold(1, Some("main:loader-a".into())).await;
        let stale = cards
            .held(1, &open)
            .await
            .ok_or_else(|| anyhow::anyhow!("page 1 not held"))?;
        // The page reloads and a second card fills the new document, while a
        // capture still compares against the first hold.
        cards.hold(1, Some("main:loader-b".into())).await;
        for current in ["main:loader-a", "main:loader-b", "main:loader-c"] {
            assert!(
                !cards
                    .release_if_changed(1, &stale, Some(current.into()))
                    .await
            );
        }
        assert!(cards.holds(1).await);
        Ok(())
    }
}
