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
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};
use tokio::{sync::Mutex, time::timeout};

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(2);
const CAPTURE_TASK_CONCURRENCY_LIMIT: usize = 2;

/// The host learns an SDK run's effects only at its end. Captures must observe
/// both counters together, including runs that begin and finish during a capture.
#[derive(Default)]
struct ScriptActivity {
    state: StdMutex<ScriptActivityState>,
}

#[derive(Default)]
struct ScriptActivityState {
    begun: u64,
    active: usize,
}

impl ScriptActivity {
    fn begin(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.begun = state.begun.wrapping_add(1);
        state.active += 1;
    }

    fn finish(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .active -= 1;
    }

    fn idle_generation(&self) -> Option<u64> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (state.active == 0).then_some(state.begun)
    }

    fn unchanged_and_idle(&self, begun: u64) -> bool {
        self.idle_generation() == Some(begun)
    }
}

/// Owns the period when an agent script may change any browser page. Only a
/// complete CDP-free verdict releases it directly; abandonment holds documents
/// asynchronously before allowing any capture to proceed.
#[must_use]
pub struct ScriptRun {
    visuals: Option<Arc<SessionVisualService>>,
}

impl ScriptRun {
    pub fn clean(mut self) {
        if let Some(visuals) = self.visuals.take() {
            visuals.script_activity.finish();
        }
    }
}

impl Drop for ScriptRun {
    fn drop(&mut self) {
        let Some(visuals) = self.visuals.take() else {
            return;
        };
        // The run's caller may already be gone. This task keeps the visual
        // service and its active count until every open document is held.
        drop(tokio::spawn(async move {
            visuals.hold_script_documents().await;
        }));
    }
}

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

const HELD_DOCUMENT_LIMIT: usize = 256;

/// Unsafe documents accumulate by target incarnation. Inserts commute across
/// card fills and script cleanup; an unreadable document can never be overwritten
/// by a later readable one and accidentally license a capture.
#[derive(Default)]
struct DocumentHolds {
    documents: HashSet<DocumentId>,
    unknown: bool,
}

#[derive(Default)]
struct CardDocumentState {
    held: HashMap<TargetId, DocumentHolds>,
    inserts: u64,
}

#[derive(Default)]
struct CardDocuments {
    state: Mutex<CardDocumentState>,
}

/// The before-capture check binds bytes to the same hold state and, for a target
/// with history, the same safe document at the second check.
struct CardCaptureCheck {
    inserts: u64,
    document: Option<DocumentId>,
}

impl CardDocuments {
    async fn hold(&self, target_id: TargetId, document: Option<DocumentId>) {
        let mut state = self.state.lock().await;
        state.inserts = state.inserts.wrapping_add(1);
        let held = state.held.entry(target_id).or_default();
        match document {
            None => held.unknown = true,
            Some(document) if !held.unknown => {
                if held.documents.len() >= HELD_DOCUMENT_LIMIT
                    && !held.documents.contains(&document)
                {
                    held.unknown = true;
                } else {
                    held.documents.insert(document);
                }
            }
            Some(_) => {}
        }
    }

    async fn before_capture(
        &self,
        target_id: &TargetId,
        current: impl std::future::Future<Output = Option<DocumentId>>,
    ) -> Option<CardCaptureCheck> {
        let (inserts, held) = {
            let state = self.state.lock().await;
            let held = state.held.get(target_id);
            if held.is_some_and(|held| held.unknown) {
                return None;
            }
            (state.inserts, held.map(|held| held.documents.clone()))
        };
        let document = match held {
            None => None,
            Some(held) => {
                let current = current.await?;
                if held.contains(&current) {
                    return None;
                }
                Some(current)
            }
        };
        Some(CardCaptureCheck { inserts, document })
    }

    async fn after_capture(
        &self,
        checked: CardCaptureCheck,
        current: impl std::future::Future<Output = Option<DocumentId>>,
    ) -> bool {
        if self.state.lock().await.inserts != checked.inserts {
            return false;
        }
        if let Some(expected) = checked.document
            && current.await.as_ref() != Some(&expected)
        {
            return false;
        }
        // A hold may have been inserted while the document read was in flight.
        self.state.lock().await.inserts == checked.inserts
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
    script_activity: ScriptActivity,
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
            script_activity: ScriptActivity::default(),
        })
    }

    /// Agent runs can reach tabs outside their session, so activity is browser-wide.
    /// Host-authored census calls do not acquire this lease.
    pub fn begin_script_run(self: &Arc<Self>) -> ScriptRun {
        self.script_activity.begin();
        ScriptRun {
            visuals: Some(self.clone()),
        }
    }

    async fn hold_script_documents(&self) {
        // Inventory retries are host cleanup, never a retry of the agent script.
        // Keep the active count until the link recovers and every page is held.
        let (browser, pages) = loop {
            if let Some(browser) = self.browser.session().await {
                match browser.pages.list().await {
                    Ok(pages) => break (browser, pages),
                    Err(error) => {
                        tracing::debug!(%error, "script document holds are waiting for page inventory")
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        };
        for page in pages {
            let document = main_document_id(&browser, page.page_id.0).await;
            self.card_documents.hold(page.target_id, document).await;
        }
        // Releasing before the holds would admit a queued picture of data that
        // raw CDP may have exposed.
        self.script_activity.finish();
    }

    /// Records the target's current document before a card fill exposes its number.
    /// Document history is retained so returning to that document cannot capture it.
    pub async fn hold_card_data(&self, page_id: u32) {
        let Some(browser) = self.browser.session().await else {
            return;
        };
        let Some(page) = browser.pages.get_info(PageId(page_id)).await else {
            return;
        };
        let document = main_document_id(&browser, page_id).await;
        self.card_documents.hold(page.target_id, document).await;
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
        let Some(script_generation) = self.script_activity.idle_generation() else {
            return Ok(None);
        };
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
        // holding the page.
        if browser
            .page_signals
            .pending_dialog(&PageId(candidate.page_id))
            .is_some()
        {
            return Ok(None);
        }
        let target_id = TargetId::from(candidate.target_id.clone());
        let Some(card_check) = self
            .card_documents
            .before_capture(&target_id, main_document_id(&browser, candidate.page_id))
            .await
        else {
            return Ok(None);
        };
        if !self.in_flight.begin(candidate.page_id).await {
            return Ok(None);
        }

        let page_id = PageId(candidate.page_id);
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
        // A script that overlapped capture may already have ended cleanly. Its
        // begun counter still rejects these bytes before they can reach storage.
        // Unsafe scripts keep the page held even after their active count falls.
        if bytes.is_empty()
            || !self.script_activity.unchanged_and_idle(script_generation)
            || !self
                .card_documents
                .after_capture(card_check, main_document_id(&browser, candidate.page_id))
                .await
        {
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
    use super::{CardDocuments, InFlightCaptures, ScriptActivity, capture_options};
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

    #[test]
    fn a_finished_script_still_invalidates_an_older_capture() -> anyhow::Result<()> {
        let activity = ScriptActivity::default();
        let original = activity
            .idle_generation()
            .ok_or_else(|| anyhow::anyhow!("fresh activity was not idle"))?;
        activity.begin();
        assert_eq!(activity.idle_generation(), None);
        activity.finish();
        assert!(!activity.unchanged_and_idle(original));
        let current = activity
            .idle_generation()
            .ok_or_else(|| anyhow::anyhow!("finished activity was not idle"))?;
        assert!(activity.unchanged_and_idle(current));
        Ok(())
    }

    #[test]
    fn every_overlapping_script_must_finish_before_capture_is_idle() {
        let activity = ScriptActivity::default();
        activity.begin();
        activity.begin();
        assert_eq!(activity.idle_generation(), None);
        activity.finish();
        assert_eq!(activity.idle_generation(), None);
        activity.finish();
        assert_eq!(activity.idle_generation(), Some(2));
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

    async fn can_capture(cards: &CardDocuments, target: &str, document: Option<&str>) -> bool {
        cards
            .before_capture(&browseros_core::TargetId::from(target.to_owned()), async {
                document.map(str::to_owned)
            })
            .await
            .is_some()
    }

    #[tokio::test]
    async fn unsafe_document_inserts_commute_and_navigation_does_not_erase_history()
    -> anyhow::Result<()> {
        for documents in [["main:a", "main:b"], ["main:b", "main:a"]] {
            let cards = CardDocuments::default();
            for document in documents {
                cards
                    .hold(
                        browseros_core::TargetId::from("target-a".to_owned()),
                        Some(document.into()),
                    )
                    .await;
            }
            assert!(!can_capture(&cards, "target-a", Some("main:a")).await);
            assert!(!can_capture(&cards, "target-a", Some("main:b")).await);
            assert!(!can_capture(&cards, "target-a", None).await);
            let checked = cards
                .before_capture(
                    &browseros_core::TargetId::from("target-a".to_owned()),
                    async { Some("main:c".into()) },
                )
                .await
                .ok_or_else(|| anyhow::anyhow!("new document should be safe"))?;
            assert!(
                cards
                    .after_capture(checked, async { Some("main:c".into()) })
                    .await
            );
            assert!(!can_capture(&cards, "target-a", Some("main:b")).await);
        }
        Ok(())
    }

    #[tokio::test]
    async fn unreadable_document_holds_are_sticky_in_either_insert_order() {
        for documents in [[None, Some("main:a")], [Some("main:a"), None]] {
            let cards = CardDocuments::default();
            for document in documents {
                cards
                    .hold(
                        browseros_core::TargetId::from("target-a".to_owned()),
                        document.map(str::to_owned),
                    )
                    .await;
            }
            for current in [None, Some("main:a"), Some("main:b")] {
                assert!(!can_capture(&cards, "target-a", current).await);
            }
            assert!(can_capture(&cards, "different-target", None).await);
        }
    }

    #[tokio::test]
    async fn unrelated_capture_checks_never_prune_another_targets_holds() -> anyhow::Result<()> {
        let cards = CardDocuments::default();
        cards
            .hold(
                browseros_core::TargetId::from("new-target".to_owned()),
                Some("main:new".into()),
            )
            .await;
        let checked = cards
            .before_capture(
                &browseros_core::TargetId::from("old-target".to_owned()),
                async { None },
            )
            .await
            .ok_or_else(|| anyhow::anyhow!("unheld target should be safe"))?;
        assert!(cards.after_capture(checked, async { None }).await);
        assert!(!can_capture(&cards, "new-target", Some("main:new")).await);
        Ok(())
    }

    #[tokio::test]
    async fn inserts_and_document_changes_during_capture_reject_its_bytes() -> anyhow::Result<()> {
        let cards = CardDocuments::default();
        cards
            .hold(
                browseros_core::TargetId::from("target-a".to_owned()),
                Some("main:unsafe".into()),
            )
            .await;
        let check = || async {
            cards
                .before_capture(
                    &browseros_core::TargetId::from("target-a".to_owned()),
                    async { Some("main:safe".into()) },
                )
                .await
                .ok_or_else(|| anyhow::anyhow!("safe document should be capturable"))
        };
        let checked = check().await?;
        assert!(
            !cards
                .after_capture(checked, async { Some("main:other".into()) })
                .await
        );
        let checked = check().await?;
        cards
            .hold(
                browseros_core::TargetId::from("target-b".to_owned()),
                Some("main:b".into()),
            )
            .await;
        assert!(
            !cards
                .after_capture(checked, async { Some("main:safe".into()) })
                .await
        );
        let checked = check().await?;
        assert!(
            !cards
                .after_capture(checked, async {
                    cards
                        .hold(browseros_core::TargetId::from("target-c".to_owned()), None)
                        .await;
                    Some("main:safe".into())
                })
                .await
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_target_document_limit_becomes_an_unknown_hold() {
        let cards = CardDocuments::default();
        for number in 0..super::HELD_DOCUMENT_LIMIT {
            cards
                .hold(
                    browseros_core::TargetId::from("target-a".to_owned()),
                    Some(format!("main:{number}")),
                )
                .await;
        }
        assert!(can_capture(&cards, "target-a", Some("main:next")).await);
        cards
            .hold(
                browseros_core::TargetId::from("target-a".to_owned()),
                Some("main:0".into()),
            )
            .await;
        assert!(can_capture(&cards, "target-a", Some("main:next")).await);
        cards
            .hold(
                browseros_core::TargetId::from("target-a".to_owned()),
                Some("main:next".into()),
            )
            .await;
        assert!(!can_capture(&cards, "target-a", Some("main:last")).await);
    }
}
