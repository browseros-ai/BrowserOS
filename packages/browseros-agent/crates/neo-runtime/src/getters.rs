//! Live reads of page state, retaining observed Frame metadata after page closure.
//! Each source was chosen because it matches what
//! real Playwright returns at the same moment, including while a navigation is in flight (see
//! `sync-getters-design.md`). A read is normally one CDP call. The exception is a `url` read
//! that lands in the few milliseconds of a cross-document commit, when BrowserOS refuses
//! navigation history: it checks the tab is still open and retries until the new document
//! answers. The main frame's URL uses that same history source, including in frame records.
//! Frame-tree getters still call Page.getFrameTree, which can block until a pending main-frame
//! navigation commits; their main-frame URL is read from history after the tree answers.

use std::collections::HashSet;
use std::sync::MutexGuard;
use std::time::Duration;

use browseros_cdp::{CdpClient, CdpError, CdpEvent, SessionId, browser};
use neo_runtime_spec::serve::{FrameChannel, FrameInitializer};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::TryRecvError;
use tokio::time;
use tokio::time::Instant;

use crate::bridge;
use crate::click::PageError;
use crate::frames::{FrameInfo, FrameScope, Host};
use crate::page;
use crate::page::PageState;
use crate::progress::PlaywrightError;
use crate::run::PageId;
use crate::tabs::{PageEntry, Tabs};
use crate::url_match::{SerializedUrlMatch, UrlMatcher};

/// Frame fields. The main URL comes directly from history; other fields read a fresh tree.
#[derive(Clone, Copy)]
pub(crate) enum FrameField {
    Url,
    Name,
    ParentFrame,
    ChildFrames,
    IsDetached,
}

/// Neo: A run's observed frame graph outlives its page, with real removals kept separately
/// from page closure. The receiver precedes domain setup; draining before each snapshot
/// prevents a detach between the last getter and page.close from reviving an old frame.
pub(crate) struct FrameMetadata {
    events: broadcast::Receiver<CdpEvent>,
    watched: HashSet<SessionId>,
    known: Vec<FrameInfo>,
    live: Vec<String>,
    detached: HashSet<String>,
    closing: bool,
    lost_events: bool,
    revision: u64,
}

/// Neo: A closing graph stays frozen across failed or cancelled live reads. Its candidate
/// owns a separate event cursor and can replace retained state only after reconciliation.
struct FrameSnapshot {
    revision: u64,
    closing_events: Option<broadcast::Receiver<CdpEvent>>,
}

/// Neo: Keeps observed metadata and real detach events separate from session closure.
impl FrameMetadata {
    pub(crate) fn new(cdp: &CdpClient) -> Self {
        Self {
            events: cdp.events(),
            watched: HashSet::new(),
            known: Vec::new(),
            live: Vec::new(),
            detached: HashSet::new(),
            closing: false,
            lost_events: false,
            revision: 0,
        }
    }

    /// Neo: Consume real removals before issuing close; OOPIF teardown during that write
    /// is page closure, not a new frame-detached event in Playwright's client graph.
    pub(crate) fn closing(&mut self) {
        let _observed = self.drain();
        self.closing = true;
    }

    fn drain(&mut self) -> Result<(), PlaywrightError> {
        if !self.closing {
            self.collect_events(&[])?;
        }
        if self.lost_events {
            return Err(PlaywrightError::error("Frame metadata events were lost"));
        }
        Ok(())
    }

    /// Neo: Post-read events also cover frames first seen in the candidate tree. An
    /// unrelated unknown frame's commit does not require the candidate to be retried.
    fn collect_events(&mut self, candidate: &[FrameInfo]) -> Result<(), PlaywrightError> {
        loop {
            match self.events.try_recv() {
                Ok(event)
                    if event
                        .session_id
                        .as_ref()
                        .is_some_and(|id| self.watched.contains(id)) =>
                {
                    let Some(id) = page::event_frame(&event) else {
                        continue;
                    };
                    let Some(session) = &event.session_id else {
                        continue;
                    };
                    let observed = self
                        .known
                        .iter()
                        .chain(candidate)
                        .any(|frame| frame.id == id);
                    let applied = page::apply(&mut PageState::default(), &event, session, id);
                    let removed =
                        event.method == "Page.frameDetached" && event.params["reason"] == "remove";
                    let swapped =
                        event.method == "Page.frameDetached" && event.params["reason"] == "swap";
                    let navigated = applied.navigation.is_some();
                    // Playwright: `server/frames.ts` `FrameManager.frameCommittedNewDocumentNavigation`: `this.removeChildFramesRecursively(frame);`
                    // Playwright: `server/chromium/crPage.ts` `FrameSession._onFrameDetached`: `this._page.frameManager.removeChildFramesRecursively(frame);`
                    if applied.new_document || swapped {
                        self.detached.extend(
                            self.known
                                .iter()
                                .filter(|frame| frame.parent.as_deref() == Some(id))
                                .map(|frame| frame.id.clone()),
                        );
                    }
                    if removed {
                        self.detached.insert(id.to_string());
                    }
                    if let Some(navigation) = applied.navigation
                        && let Some(frame) = self.known.iter_mut().find(|frame| frame.id == id)
                    {
                        frame.url = navigation.url;
                        if let Some(name) = event.params["frame"]["name"].as_str() {
                            frame.name = name.to_string();
                        }
                    }
                    if removed || (observed && (applied.new_document || swapped || navigated)) {
                        self.revision = self.revision.wrapping_add(1);
                    }
                }
                Ok(_) => {}
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Closed | TryRecvError::Lagged(_)) => {
                    self.lost_events = true;
                    self.revision = self.revision.wrapping_add(1);
                    return Err(PlaywrightError::error("Frame metadata events were lost"));
                }
            }
        }
        loop {
            let mut added = false;
            for frame in &self.known {
                if frame
                    .parent
                    .as_ref()
                    .is_some_and(|id| self.detached.contains(id))
                {
                    added |= self.detached.insert(frame.id.clone());
                }
            }
            if !added {
                break;
            }
        }
        self.live.retain(|id| !self.detached.contains(id));
        Ok(())
    }

    /// Neo: A fresh receiver precedes recovery's live read. Lost history remains explicit
    /// until that read succeeds and no queued structural change invalidates its snapshot.
    fn begin_snapshot(&mut self, cdp: &CdpClient) -> FrameSnapshot {
        if self.closing {
            return FrameSnapshot {
                revision: self.revision,
                closing_events: Some(self.events.resubscribe()),
            };
        }
        let _collected = self.collect_events(&[]);
        if self.lost_events {
            self.events = cdp.events();
        }
        FrameSnapshot {
            revision: self.revision,
            closing_events: None,
        }
    }

    fn finish_snapshot(&mut self, tree: &[FrameInfo], snapshot: FrameSnapshot) -> bool {
        if let Some(events) = snapshot.closing_events {
            let mut candidate = Self {
                events,
                watched: self.watched.clone(),
                known: self.known.clone(),
                live: self.live.clone(),
                detached: self.detached.clone(),
                closing: false,
                lost_events: self.lost_events,
                revision: self.revision,
            };
            let accepted = candidate.publish_snapshot(tree, snapshot.revision);
            if accepted {
                *self = candidate;
            }
            return accepted;
        }
        if self.closing {
            // Closure started while this observation was in flight. Its queued teardown
            // belongs to the frozen graph, not an open-page observation.
            return false;
        }
        self.publish_snapshot(tree, snapshot.revision)
    }

    fn publish_snapshot(&mut self, tree: &[FrameInfo], revision: u64) -> bool {
        if self.collect_events(tree).is_err() || revision != self.revision {
            return false;
        }
        for frame in tree {
            match self.known.iter_mut().find(|known| known.id == frame.id) {
                Some(known) => *known = frame.clone(),
                None => self.known.push(frame.clone()),
            }
        }
        self.live = tree.iter().map(|frame| frame.id.clone()).collect();
        // A complete, reconciled live tree is authoritative after lost event history.
        self.detached.retain(|id| !self.live.contains(id));
        self.lost_events = false;
        true
    }

    fn tree(&mut self) -> Result<Vec<FrameInfo>, PlaywrightError> {
        self.drain()?;
        Ok(self
            .live
            .iter()
            .filter_map(|id| self.known.iter().find(|frame| &frame.id == id).cloned())
            .collect())
    }

    fn value(
        &mut self,
        page: &PageId,
        scope: &FrameScope,
        field: FrameField,
    ) -> Result<Value, PlaywrightError> {
        self.drain()?;
        let id = match scope {
            FrameScope::Main => page.as_str(),
            FrameScope::Frame(id) => id,
        };
        match field {
            FrameField::IsDetached => Ok(json!(
                !matches!(scope, FrameScope::Main) && !self.live.iter().any(|live| live == id)
            )),
            FrameField::ChildFrames => frame_records(
                page,
                &self.known,
                self.known.iter().filter(|frame| {
                    frame.parent.as_deref() == Some(id) && self.live.contains(&frame.id)
                }),
            ),
            _ => frame_value(page, &self.known, scope, field),
        }
    }
}

fn metadata(page: &PageEntry) -> Result<MutexGuard<'_, FrameMetadata>, PlaywrightError> {
    page.frame_metadata
        .lock()
        .map_err(|_| PlaywrightError::error("Frame metadata cannot be accessed"))
}

/// Neo: A newly enabled or disappeared child host requires a fresh tree. Only a confirmed
/// vanished session is rediscovered; connection, timeout and other protocol errors propagate.
async fn watch_frame_host(page: &PageEntry, host: &Host) -> Result<bool, PlaywrightError> {
    if metadata(page)?.watched.contains(host.session_id()) {
        return Ok(false);
    }
    let enabled = host.send("Page.enable", json!({})).await;
    if host.is_child() && matches!(&enabled, Err(CdpError::SessionGone)) {
        return Ok(true);
    }
    enabled.map_err(|error| frame_tree_error(host.classify(&error)))?;
    metadata(page)?.watched.insert(host.session_id().clone());
    Ok(true)
}

/// Uses the same matcher as `waitForURL`, without retaining a guid or matcher state.
pub(crate) fn matches_url(
    url: &str,
    matcher: &SerializedUrlMatch,
) -> Result<bool, PlaywrightError> {
    UrlMatcher::new(matcher)?.matches(url)
}

/// Every current frame, in the core's tree order. Rust alone mints the SDK's opaque guids.
/// This can wait for a pending main-frame navigation to commit before the tree answers.
pub(crate) async fn frames(tabs: &Tabs, page: &PageEntry) -> Result<Value, PlaywrightError> {
    let tree = frame_tree(tabs, page).await?;
    frame_records(&page.id, &tree, tree.iter())
}

/// The record of the frame CDP calls `frame_id` in `page`'s tree, `None` when the page has no
/// such frame: an element handle's `ownerFrame` and `contentFrame` (`element_handle.rs`), as
/// Playwright's `FrameManager.frame(frameId)`.
pub(crate) async fn frame_by_id(
    tabs: &Tabs,
    page: &PageEntry,
    frame_id: &str,
) -> Result<Option<Value>, PlaywrightError> {
    let tree = frame_tree(tabs, page).await?;
    tree.iter()
        .find(|frame| frame.id == frame_id)
        .map(|frame| frame_record(&page.id, &tree, frame))
        .transpose()
}

/// A direct frame's live field, or its observed metadata after page closure. A real detach
/// preserves the frame's parent identity while removing it from the parent's child list.
/// Only the main URL avoids a frame-tree read while the page is alive.
pub(crate) async fn frame(
    tabs: &Tabs,
    page: &PageEntry,
    scope: &FrameScope,
    field: FrameField,
) -> Result<Value, PlaywrightError> {
    if page.closed.is_cancelled() {
        return metadata(page)?.value(&page.id, scope, field);
    }
    if matches!((scope, field), (FrameScope::Main, FrameField::Url)) {
        let url = tabs.url(page).await?;
        if let Some(url) = &url
            && let Some(main) = metadata(page)?
                .known
                .iter_mut()
                .find(|frame| frame.id == page.id.as_str())
        {
            main.url.clone_from(url);
        }
        return Ok(json!(url));
    }
    frame_tree(tabs, page).await?;
    metadata(page)?.value(&page.id, scope, field)
}

/// Neo: Keeps frame records on the same committed-URL source as the page getter.
async fn frame_tree(tabs: &Tabs, page: &PageEntry) -> Result<Vec<FrameInfo>, PlaywrightError> {
    if page.closed.is_cancelled() {
        return metadata(page)?.tree();
    }
    let session = tabs.session(page).await?;
    loop {
        let snapshot = metadata(page)?.begin_snapshot(tabs.cdp());
        watch_frame_host(page, &session.frames.main().host).await?;
        let mut tree = session.frames.tree().await.map_err(frame_tree_error)?;
        loop {
            let mut refresh = false;
            for frame in &tree {
                if let Some(context) = session.frames.cached(&frame.id) {
                    refresh |= watch_frame_host(page, &context.host).await?;
                }
            }
            if !refresh {
                break;
            }
            // A host can lose a frame before Page.enable. Re-read after subscribing so that
            // missing frames cannot enter the retained graph without a corresponding event.
            tree = session.frames.tree().await.map_err(frame_tree_error)?;
        }
        if let Some(main) = tree.first_mut() {
            main.url = tabs
                .url(page)
                .await?
                .ok_or_else(PlaywrightError::target_closed)?;
        }
        let mut metadata = metadata(page)?;
        if metadata.finish_snapshot(&tree, snapshot) {
            return metadata.tree();
        }
    }
}

fn frame_value(
    page: &PageId,
    tree: &[FrameInfo],
    scope: &FrameScope,
    field: FrameField,
) -> Result<Value, PlaywrightError> {
    let info = match scope {
        FrameScope::Main => tree.first(),
        FrameScope::Frame(id) => tree.iter().find(|frame| frame.id == *id),
    };
    let Some(info) = info else {
        return Ok(match field {
            FrameField::IsDetached => json!(true),
            FrameField::ChildFrames => json!([]),
            _ => Value::Null,
        });
    };
    match field {
        FrameField::Url => Ok(json!(info.url)),
        FrameField::Name => Ok(json!(info.name)),
        FrameField::ParentFrame => info.parent.as_ref().map_or(Ok(Value::Null), |id| {
            let parent = tree.iter().find(|frame| frame.id == *id).ok_or_else(|| {
                PlaywrightError::error("Frame parent is missing from the frame tree")
            })?;
            frame_record(page, tree, parent)
        }),
        FrameField::ChildFrames => frame_records(
            page,
            tree,
            tree.iter()
                .filter(|frame| frame.parent.as_ref() == Some(&info.id)),
        ),
        FrameField::IsDetached => Ok(json!(false)),
    }
}

fn frame_records<'a>(
    page: &PageId,
    tree: &[FrameInfo],
    records: impl Iterator<Item = &'a FrameInfo>,
) -> Result<Value, PlaywrightError> {
    records
        .map(|frame| frame_record(page, tree, frame))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn frame_record(
    page: &PageId,
    tree: &[FrameInfo],
    frame: &FrameInfo,
) -> Result<Value, PlaywrightError> {
    let parent_frame = frame
        .parent
        .as_ref()
        .map(|id| -> Result<FrameChannel, PlaywrightError> {
            let parent = tree.iter().find(|info| info.id == *id).ok_or_else(|| {
                PlaywrightError::error("Frame parent is missing from the frame tree")
            })?;
            Ok(FrameChannel {
                guid: frame_guid(page, parent),
            })
        })
        .transpose()?;
    let initializer = FrameInitializer {
        url: frame.url.clone(),
        name: frame.name.clone(),
        parent_frame,
        load_states: Vec::new(),
    };
    Ok(json!({ "guid": frame_guid(page, frame), "initializer": initializer }))
}

fn frame_guid(page: &PageId, frame: &FrameInfo) -> String {
    if frame.parent.is_none() {
        bridge::frame_guid(page)
    } else {
        format!("frame@{}/{}", page.as_str(), frame.id)
    }
}

fn frame_tree_error(error: PageError) -> PlaywrightError {
    match error {
        PageError::Closed => PlaywrightError::target_closed(),
        PageError::Threw(message) | PageError::Contract(message) => PlaywrightError::error(message),
        PageError::NotConnected => PlaywrightError::error("Frame tree is not available"),
    }
}

/// BrowserOS's `Browser.getTabInfo` error message for a tab that no longer exists. It is the
/// only signal that tells "closed" apart from a failed call, so `Runtime::connect` checks the
/// browser still says it before any run relies on it.
pub(crate) const CLOSED_TAB_MESSAGE: &str = "No target with given id";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NavigationHistory {
    current_index: usize,
    entries: Vec<NavigationEntry>,
}

#[derive(Deserialize)]
struct NavigationEntry {
    url: String,
}

/// How long `page.url()` keeps re-reading navigation history while the tab is still open. At
/// every cross-document commit BrowserOS answers that read with "Not attached to an active
/// page" for under 10 ms (1 to 8 ms, one to three calls), then with the new URL; measured on
/// BrowserOS neo 155.0.8318.26 by the navigation probes in `sync-getters-design.md`. The
/// budget is far above that window only so that a session which never recovers fails loudly
/// instead of hanging the script.
const HISTORY_RETRY_BUDGET: Duration = Duration::from_millis(250);
/// Retries wait 1, 2, 4, 8, then 16 ms, so the first ones land inside the commit window.
const MAX_RETRY_BACKOFF: Duration = Duration::from_millis(16);

/// `page.url()`: the committed URL of the session's current history entry, or `None` once
/// the tab is closed. Playwright keeps the old URL until a navigation commits, and so does
/// navigation history. `Page.getFrameTree` and `location.href` block until the commit, and
/// the target and tab infos show the destination before it, so history is the only source
/// and nothing else stands in when it fails.
///
/// A failed history read means the tab closed or a navigation is committing; the tab check
/// tells them apart without reading the error text. Closed gives `None`. Open retries within
/// [`HISTORY_RETRY_BUDGET`] and then returns the last history error. Every caller races the
/// whole call against the run's deadline, so the budget never outlives the run.
pub(crate) async fn url(
    cdp: &CdpClient,
    session: &SessionId,
    page: &PageId,
) -> Result<Option<String>, CdpError> {
    let give_up_at = Instant::now() + HISTORY_RETRY_BUDGET;
    let mut backoff = Duration::from_millis(1);
    loop {
        let history_error = match current_url(cdp, session).await {
            Ok(url) => return Ok(Some(url)),
            Err(error) => error,
        };
        if is_closed(cdp, page).await? {
            return Ok(None);
        }
        if Instant::now() + backoff >= give_up_at {
            return Err(history_error);
        }
        time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_RETRY_BACKOFF);
    }
}

async fn current_url(cdp: &CdpClient, session: &SessionId) -> Result<String, CdpError> {
    let reply = cdp
        .send("Page.getNavigationHistory", json!({}), Some(session))
        .await?;
    let history: NavigationHistory = serde_json::from_value(reply)?;
    let entry_count = history.entries.len();
    history
        .entries
        .into_iter()
        .nth(history.current_index)
        .map(|entry| entry.url)
        .ok_or_else(|| {
            CdpError::Json(format!(
                "Page.getNavigationHistory: currentIndex {} is outside its {entry_count} entries",
                history.current_index
            ))
        })
}

/// `page.isClosed()`: whether BrowserOS still has a tab for this target.
pub(crate) async fn is_closed(cdp: &CdpClient, page: &PageId) -> Result<bool, CdpError> {
    let params = browser::GetTabInfoParams {
        target_id: Some(page.as_str().to_string()),
        tab_id: None,
    };
    let reply = cdp
        .send_typed::<_, Value>("Browser.getTabInfo", params, None)
        .await;
    match reply {
        Ok(_) => Ok(false),
        Err(CdpError::Protocol { message, .. }) if message.contains(CLOSED_TAB_MESSAGE) => Ok(true),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use browseros_cdp::{CdpEvent, SessionId};
    use serde_json::{Value, json};

    use tokio::runtime::Handle;
    use tokio::sync::broadcast;
    use tokio_util::task::TaskTracker;

    use super::{
        FrameField, FrameMetadata, frame, frame_records, frame_value, matches_url, watch_frame_host,
    };
    use crate::discovery::Discovery;
    use crate::fake_browser::{FakeBrowser, FrameReplies, PAGE, PAGE_URL, Replies};
    use crate::frames::{FrameInfo, FrameScope, Host};
    use crate::page::TabInputs;
    use crate::progress::ErrorName;
    use crate::run::PageId;
    use crate::tabs::{PageEntry, Tabs};
    use crate::url_match::SerializedUrlMatch;

    fn tree() -> Vec<FrameInfo> {
        [
            ("root", None, "", "https://root.test/"),
            ("local", Some("root"), "login", "https://root.test/login"),
            ("remote", Some("local"), "payment", "https://pay.test/"),
            (
                "nested",
                Some("remote"),
                "nested",
                "https://pay.test/nested",
            ),
        ]
        .into_iter()
        .map(|(id, parent, name, url)| FrameInfo {
            id: id.to_string(),
            parent: parent.map(str::to_string),
            name: name.to_string(),
            url: url.to_string(),
        })
        .collect()
    }

    fn fixture_tree(name: &str, old_child: bool) -> FrameReplies {
        let mut children = vec![
            json!({"frame":{"id":"local","parentId":PAGE,"name":name,"url":"https://example.test/local"}}),
        ];
        if old_child {
            children.push(json!({"frame":{"id":"old","parentId":PAGE,"name":"old","url":"https://example.test/old"}}));
        }
        FrameReplies {
            trees: [(
                PAGE.to_string(),
                json!({"frameTree":{"frame":{"id":PAGE,"url":PAGE_URL},"childFrames":children}}),
            )]
            .into(),
            ..FrameReplies::default()
        }
    }

    async fn frame_fixture() -> (FakeBrowser, Tabs, Arc<PageEntry>) {
        let browser = FakeBrowser::start(Replies::default()).await;
        browser
            .page
            .lock()
            .unwrap_or_else(|error| panic!("fixture state: {error}"))
            .frames = Some(fixture_tree("before", true));
        let (tabs, _) = Tabs::open(
            browser.cdp().await,
            Handle::current(),
            TabInputs::default(),
            Discovery::default(),
            TaskTracker::new(),
            &PageId::new(PAGE.to_string()),
        )
        .await
        .unwrap_or_else(|error| panic!("open fixture: {error:?}"));
        let (page, _) = tabs
            .resolve(&format!("frame@{PAGE}"))
            .unwrap_or_else(|| panic!("fixture page was not registered"));
        (browser, tabs, page)
    }

    async fn remote_fixture() -> (FakeBrowser, Tabs, Arc<PageEntry>, Arc<Host>) {
        let (browser, tabs, page) = frame_fixture().await;
        let mut replies = fixture_tree("before", true);
        replies
            .targets
            .push(json!({"type":"iframe","targetId":"remote","parentFrameId":PAGE}));
        replies.trees.insert("remote".to_string(), json!({"frameTree":{"frame":{"id":"remote","name":"remote","url":"https://remote.test/"}}}));
        browser
            .page
            .lock()
            .unwrap_or_else(|error| panic!("fixture state: {error}"))
            .frames = Some(replies);
        let session = page
            .attached_session()
            .unwrap_or_else(|| panic!("fixture has no session"));
        session
            .frames
            .tree()
            .await
            .unwrap_or_else(|error| panic!("fixture frame tree: {error:?}"));
        let context = session
            .frames
            .cached("remote")
            .unwrap_or_else(|| panic!("remote host missing"));
        let host = Arc::clone(&context.host);
        (browser, tabs, page, host)
    }

    fn lose_events(page: &PageEntry) -> broadcast::Sender<CdpEvent> {
        let (sender, receiver) = broadcast::channel(1);
        page.frame_metadata
            .lock()
            .unwrap_or_else(|error| panic!("metadata state: {error}"))
            .events = receiver;
        for _ in 0..2 {
            let _sent = sender.send(CdpEvent {
                method: "Runtime.consoleAPICalled".to_string(),
                params: json!({}),
                session_id: None,
            });
        }
        sender
    }

    fn metadata_events(page: &PageEntry) -> (broadcast::Sender<CdpEvent>, SessionId) {
        let (sender, receiver) = broadcast::channel(16);
        let mut metadata = page
            .frame_metadata
            .lock()
            .unwrap_or_else(|error| panic!("metadata state: {error}"));
        let session = metadata
            .watched
            .iter()
            .next()
            .cloned()
            .unwrap_or_else(|| panic!("fixture has no watched host"));
        metadata.events = receiver;
        (sender, session)
    }

    fn commit(session: &SessionId, id: &str) -> CdpEvent {
        CdpEvent {
            method: "Page.frameNavigated".to_string(),
            params: json!({"frame":{"id":id,"url":"https://example.test/new","loaderId":"new-document"}}),
            session_id: Some(session.clone()),
        }
    }

    #[test]
    fn tests_that_frame_records_use_generated_initializers_and_opaque_guids() {
        let page = PageId::new("tab".to_string());
        let tree = tree();
        let records = frame_records(&page, &tree, tree.iter()).unwrap();
        assert_eq!(
            records[0],
            json!({
                "guid": "frame@tab",
                "initializer": {"url": "https://root.test/", "name": "", "loadStates": []},
            })
        );
        assert_eq!(
            records[1],
            json!({
                "guid": "frame@tab/local",
                "initializer": {"url": "https://root.test/login", "name": "login", "loadStates": [], "parentFrame": {"guid": "frame@tab"}},
            })
        );
        assert_eq!(
            records[2]["initializer"]["parentFrame"],
            json!({"guid": "frame@tab/local"})
        );
        assert_eq!(
            records[3]["initializer"]["parentFrame"],
            json!({"guid": "frame@tab/remote"})
        );
    }

    #[test]
    fn tests_that_each_frame_getter_reads_its_own_scope() {
        let page = PageId::new("tab".to_string());
        let tree = tree();
        let main = FrameScope::Main;
        let remote = FrameScope::Frame("remote".to_string());
        assert_eq!(
            frame_value(&page, &tree, &main, FrameField::ParentFrame).unwrap(),
            Value::Null
        );
        assert_eq!(
            frame_value(&page, &tree, &remote, FrameField::Url).unwrap(),
            "https://pay.test/"
        );
        assert_eq!(
            frame_value(&page, &tree, &remote, FrameField::Name).unwrap(),
            "payment"
        );
        assert_eq!(
            frame_value(&page, &tree, &remote, FrameField::IsDetached).unwrap(),
            false
        );
        assert_eq!(
            frame_value(&page, &tree, &remote, FrameField::ParentFrame).unwrap()["guid"],
            "frame@tab/local"
        );
        let children = frame_value(&page, &tree, &remote, FrameField::ChildFrames).unwrap();
        assert_eq!(children.as_array().unwrap().len(), 1);
        assert_eq!(children[0]["guid"], "frame@tab/nested");
        let leaf = FrameScope::Frame("nested".to_string());
        assert_eq!(
            frame_value(&page, &tree, &leaf, FrameField::ChildFrames).unwrap(),
            json!([])
        );
    }

    #[test]
    fn tests_that_detached_frames_return_empty_live_getter_values() {
        let page = PageId::new("tab".to_string());
        let tree = tree();
        let removed = FrameScope::Frame("removed".to_string());
        assert_eq!(
            frame_value(&page, &tree, &removed, FrameField::IsDetached).unwrap(),
            true
        );
        for field in [FrameField::Url, FrameField::Name, FrameField::ParentFrame] {
            assert_eq!(
                frame_value(&page, &tree, &removed, field).unwrap(),
                Value::Null
            );
        }
        assert_eq!(
            frame_value(&page, &tree, &removed, FrameField::ChildFrames).unwrap(),
            json!([])
        );
    }

    #[tokio::test]
    async fn tests_that_observed_frame_metadata_does_not_hide_connection_errors() {
        let browser = FakeBrowser::start(Replies::default()).await;
        browser.page.lock().unwrap().frames = Some(FrameReplies {
            trees: [(
                PAGE.to_string(),
                json!({"frameTree":{"frame":{"id":PAGE,"url":PAGE_URL}}}),
            )]
            .into(),
            ..FrameReplies::default()
        });
        let cdp = browser.cdp().await;
        let (tabs, _) = Tabs::open(
            cdp.clone(),
            Handle::current(),
            TabInputs::default(),
            Discovery::default(),
            TaskTracker::new(),
            &PageId::new(PAGE.to_string()),
        )
        .await
        .unwrap();
        let (page, scope) = tabs.resolve(&format!("frame@{PAGE}")).unwrap();
        assert_eq!(
            frame(&tabs, &page, &scope, FrameField::Name).await.unwrap(),
            ""
        );
        cdp.disconnect().await;
        let error = frame(&tabs, &page, &scope, FrameField::Name)
            .await
            .unwrap_err();
        assert_eq!(error.name, ErrorName::TargetClosedError);
    }

    #[tokio::test]
    async fn tests_that_a_fresh_live_tree_recovers_lost_metadata_events_before_close() {
        let (browser, tabs, page) = frame_fixture().await;
        let local = FrameScope::Frame("local".to_string());
        let old = FrameScope::Frame("old".to_string());
        assert_eq!(
            frame(&tabs, &page, &local, FrameField::Name).await.unwrap(),
            "before"
        );
        let _events = lose_events(&page);
        browser.page.lock().unwrap().frames = Some(fixture_tree("after", false));
        assert_eq!(
            frame(&tabs, &page, &local, FrameField::Name).await.unwrap(),
            "after"
        );
        assert_eq!(
            frame(&tabs, &page, &old, FrameField::IsDetached)
                .await
                .unwrap(),
            true
        );
        page.request_close();
        tabs.cdp()
            .send("Target.closeTarget", json!({"targetId":PAGE}), None)
            .await
            .unwrap();
        assert!(tabs.is_closed(&page).await.unwrap());
        assert_eq!(
            frame(&tabs, &page, &local, FrameField::Name).await.unwrap(),
            "after"
        );
        assert_eq!(
            frame(&tabs, &page, &old, FrameField::IsDetached)
                .await
                .unwrap(),
            true
        );
        assert_eq!(
            frame(&tabs, &page, &old, FrameField::Name).await.unwrap(),
            "old"
        );
    }

    #[tokio::test]
    async fn tests_that_event_loss_without_a_fresh_tree_stays_explicit_after_close() {
        let (_browser, tabs, page) = frame_fixture().await;
        let local = FrameScope::Frame("local".to_string());
        assert_eq!(
            frame(&tabs, &page, &local, FrameField::Name).await.unwrap(),
            "before"
        );
        let _events = lose_events(&page);
        page.request_close();
        tabs.cdp()
            .send("Target.closeTarget", json!({"targetId":PAGE}), None)
            .await
            .unwrap();
        assert!(tabs.is_closed(&page).await.unwrap());
        assert!(frame(&tabs, &page, &local, FrameField::Name).await.is_err());
    }

    #[tokio::test]
    async fn tests_that_lost_metadata_never_hides_a_failed_live_recovery() {
        let (_browser, tabs, page) = frame_fixture().await;
        let local = FrameScope::Frame("local".to_string());
        assert_eq!(
            frame(&tabs, &page, &local, FrameField::Name).await.unwrap(),
            "before"
        );
        let _events = lose_events(&page);
        tabs.cdp().disconnect().await;
        let error = frame(&tabs, &page, &local, FrameField::Name)
            .await
            .unwrap_err();
        assert_eq!(error.name, ErrorName::TargetClosedError);
    }

    #[tokio::test]
    async fn tests_that_a_commit_during_the_first_snapshot_cannot_keep_old_children() {
        let (_browser, tabs, page) = frame_fixture().await;
        frame(&tabs, &page, &FrameScope::Main, FrameField::Name)
            .await
            .unwrap();
        let old_tree = page.frame_metadata.lock().unwrap().known.clone();
        let (sender, session) = metadata_events(&page);
        let mut fresh = super::FrameMetadata::new(tabs.cdp());
        fresh.events = sender.subscribe();
        fresh.watched.insert(session.clone());
        let snapshot = fresh.begin_snapshot(tabs.cdp());
        sender.send(commit(&session, PAGE)).unwrap();
        if !fresh.finish_snapshot(&old_tree, snapshot) {
            let snapshot = fresh.begin_snapshot(tabs.cdp());
            assert!(fresh.finish_snapshot(&[old_tree[0].clone()], snapshot));
        }
        fresh.closing();
        assert_eq!(
            fresh
                .value(
                    &page.id,
                    &FrameScope::Frame("local".to_string()),
                    FrameField::IsDetached
                )
                .unwrap(),
            true
        );
    }

    #[tokio::test]
    async fn tests_that_initial_commit_replay_cannot_detach_a_fresh_snapshot() {
        let (_browser, tabs, page) = frame_fixture().await;
        frame(&tabs, &page, &FrameScope::Main, FrameField::Name)
            .await
            .unwrap();
        let (sender, session) = metadata_events(&page);
        let tree = page.frame_metadata.lock().unwrap().known.clone();
        let mut metadata = FrameMetadata::new(tabs.cdp());
        metadata.events = sender.subscribe();
        metadata.watched.insert(session.clone());
        sender.send(commit(&session, PAGE)).unwrap();
        let revision = metadata.begin_snapshot(tabs.cdp());
        assert!(metadata.finish_snapshot(&tree, revision));
        metadata.closing();
        assert_eq!(
            metadata
                .value(
                    &page.id,
                    &FrameScope::Frame("local".to_string()),
                    FrameField::IsDetached
                )
                .unwrap(),
            false
        );
    }

    #[tokio::test]
    async fn tests_that_a_commit_before_a_snapshot_keeps_the_new_children_attached() {
        let (_browser, tabs, page) = frame_fixture().await;
        frame(&tabs, &page, &FrameScope::Main, FrameField::Name)
            .await
            .unwrap();
        let (sender, session) = metadata_events(&page);
        sender.send(commit(&session, PAGE)).unwrap();
        let mut metadata = page.frame_metadata.lock().unwrap();
        let revision = metadata.begin_snapshot(tabs.cdp());
        let mut tree = metadata.known.clone();
        tree.retain(|frame| frame.id != "old" && frame.id != "grand");
        tree.iter_mut()
            .find(|frame| frame.id == "local")
            .unwrap()
            .id = "new-local".to_string();
        assert!(metadata.finish_snapshot(&tree, revision));
        metadata.closing();
        assert_eq!(
            metadata
                .value(
                    &page.id,
                    &FrameScope::Frame("new-local".to_string()),
                    FrameField::IsDetached
                )
                .unwrap(),
            false
        );
    }

    #[tokio::test]
    async fn tests_that_a_commit_after_a_snapshot_cannot_revive_old_children() {
        let (_browser, tabs, page) = frame_fixture().await;
        frame(&tabs, &page, &FrameScope::Main, FrameField::Name)
            .await
            .unwrap();
        let (sender, session) = metadata_events(&page);
        let mut metadata = page.frame_metadata.lock().unwrap();
        let stale = metadata.known.clone();
        let revision = metadata.begin_snapshot(tabs.cdp());
        sender.send(commit(&session, PAGE)).unwrap();
        if !metadata.finish_snapshot(&stale, revision) {
            let revision = metadata.begin_snapshot(tabs.cdp());
            let current = vec![stale[0].clone()];
            assert!(metadata.finish_snapshot(&current, revision));
        }
        metadata.closing();
        assert_eq!(
            metadata
                .value(
                    &page.id,
                    &FrameScope::Frame("local".to_string()),
                    FrameField::IsDetached
                )
                .unwrap(),
            true
        );
    }

    #[tokio::test]
    async fn tests_that_a_same_document_event_after_a_snapshot_keeps_its_url_and_children() {
        let (_browser, tabs, page) = frame_fixture().await;
        frame(&tabs, &page, &FrameScope::Main, FrameField::Name)
            .await
            .unwrap();
        let (sender, session) = metadata_events(&page);
        let mut metadata = page.frame_metadata.lock().unwrap();
        let stale = metadata.known.clone();
        let revision = metadata.begin_snapshot(tabs.cdp());
        sender
            .send(CdpEvent {
                method: "Page.navigatedWithinDocument".to_string(),
                params: json!({"frameId":PAGE,"url":format!("{PAGE_URL}#next")}),
                session_id: Some(session),
            })
            .unwrap();
        if !metadata.finish_snapshot(&stale, revision) {
            let revision = metadata.begin_snapshot(tabs.cdp());
            let mut current = stale;
            current[0].url = format!("{PAGE_URL}#next");
            assert!(metadata.finish_snapshot(&current, revision));
        }
        metadata.closing();
        assert_eq!(
            metadata
                .value(&page.id, &FrameScope::Main, FrameField::Url)
                .unwrap(),
            format!("{PAGE_URL}#next")
        );
        assert_eq!(
            metadata
                .value(
                    &page.id,
                    &FrameScope::Frame("local".to_string()),
                    FrameField::IsDetached
                )
                .unwrap(),
            false
        );
    }

    #[tokio::test]
    async fn tests_that_unrelated_broadcast_events_do_not_invalidate_live_snapshots() {
        let (_browser, tabs, page) = frame_fixture().await;
        frame(&tabs, &page, &FrameScope::Main, FrameField::Name)
            .await
            .unwrap();
        let (sender, session) = metadata_events(&page);
        let mut metadata = page.frame_metadata.lock().unwrap();
        let tree = metadata.known.clone();
        let revision = metadata.begin_snapshot(tabs.cdp());
        sender
            .send(CdpEvent {
                method: "Runtime.consoleAPICalled".to_string(),
                params: json!({}),
                session_id: Some(session.clone()),
            })
            .unwrap();
        sender.send(commit(&session, "unobserved-churn")).unwrap();
        assert!(metadata.finish_snapshot(&tree, revision));
        assert_eq!(
            metadata
                .value(
                    &page.id,
                    &FrameScope::Frame("local".to_string()),
                    FrameField::IsDetached
                )
                .unwrap(),
            false
        );
    }

    #[tokio::test]
    async fn tests_that_an_oopif_root_commit_only_detaches_its_own_descendants() {
        let (browser, tabs, page) = frame_fixture().await;
        browser
            .page
            .lock()
            .unwrap()
            .frames
            .as_mut()
            .unwrap()
            .trees
            .get_mut(PAGE)
            .unwrap()["frameTree"]["childFrames"][0]["childFrames"] = json!([{"frame":{"id":"grand","parentId":"local","name":"grand","url":"https://example.test/grand"}}]);
        frame(&tabs, &page, &FrameScope::Main, FrameField::Name)
            .await
            .unwrap();
        let (sender, session) = metadata_events(&page);
        sender.send(commit(&session, "local")).unwrap();
        let mut metadata = page.frame_metadata.lock().unwrap();
        metadata.closing();
        assert_eq!(
            metadata
                .value(
                    &page.id,
                    &FrameScope::Frame("local".to_string()),
                    FrameField::IsDetached
                )
                .unwrap(),
            false
        );
        assert_eq!(
            metadata
                .value(
                    &page.id,
                    &FrameScope::Frame("grand".to_string()),
                    FrameField::IsDetached
                )
                .unwrap(),
            true
        );
        assert_eq!(
            metadata
                .value(
                    &page.id,
                    &FrameScope::Frame("old".to_string()),
                    FrameField::IsDetached
                )
                .unwrap(),
            false
        );
    }

    #[tokio::test]
    async fn tests_that_a_process_swap_retires_descendants_without_detaching_the_frame() {
        let (browser, tabs, page) = frame_fixture().await;
        browser
            .page
            .lock()
            .unwrap()
            .frames
            .as_mut()
            .unwrap()
            .trees
            .get_mut(PAGE)
            .unwrap()["frameTree"]["childFrames"][0]["childFrames"] = json!([{"frame":{"id":"grand","parentId":"local","name":"grand","url":"https://example.test/grand"}}]);
        frame(&tabs, &page, &FrameScope::Main, FrameField::Name)
            .await
            .unwrap();
        let (sender, session) = metadata_events(&page);
        sender
            .send(CdpEvent {
                method: "Page.frameDetached".to_string(),
                params: json!({"frameId":"local","reason":"swap"}),
                session_id: Some(session),
            })
            .unwrap();
        let mut metadata = page.frame_metadata.lock().unwrap();
        metadata.closing();
        assert_eq!(
            metadata
                .value(
                    &page.id,
                    &FrameScope::Frame("local".to_string()),
                    FrameField::IsDetached
                )
                .unwrap(),
            false
        );
        assert_eq!(
            metadata
                .value(
                    &page.id,
                    &FrameScope::Frame("grand".to_string()),
                    FrameField::IsDetached
                )
                .unwrap(),
            true
        );
        assert_eq!(
            metadata
                .value(
                    &page.id,
                    &FrameScope::Frame("old".to_string()),
                    FrameField::IsDetached
                )
                .unwrap(),
            false
        );
        assert_eq!(
            metadata
                .value(
                    &page.id,
                    &FrameScope::Frame("grand".to_string()),
                    FrameField::Name
                )
                .unwrap(),
            "grand"
        );
    }

    #[tokio::test]
    async fn tests_that_a_failed_read_during_close_keeps_retained_attachment() {
        let (_browser, tabs, page) = frame_fixture().await;
        let local = FrameScope::Frame("local".to_string());
        assert_eq!(
            frame(&tabs, &page, &local, FrameField::Name).await.unwrap(),
            "before"
        );
        let (sender, receiver) = broadcast::channel(8);
        let session = {
            let mut metadata = page.frame_metadata.lock().unwrap();
            metadata.events = receiver;
            metadata.watched.iter().next().cloned().unwrap()
        };
        page.request_close();
        tabs.cdp()
            .send("Target.closeTarget", json!({"targetId":PAGE}), None)
            .await
            .unwrap();
        sender
            .send(CdpEvent {
                method: "Page.frameDetached".to_string(),
                params: json!({"frameId":"local","reason":"remove"}),
                session_id: Some(session),
            })
            .unwrap();
        tabs.cdp().disconnect().await;
        assert!(frame(&tabs, &page, &local, FrameField::Name).await.is_err());
        page.closed.cancel();
        assert_eq!(
            frame(&tabs, &page, &local, FrameField::IsDetached)
                .await
                .unwrap(),
            false
        );
    }

    #[tokio::test]
    async fn tests_that_an_invalidated_read_during_close_keeps_retained_attachment() {
        let (_browser, tabs, page) = frame_fixture().await;
        let local = FrameScope::Frame("local".to_string());
        assert_eq!(
            frame(&tabs, &page, &local, FrameField::Name).await.unwrap(),
            "before"
        );
        let (sender, session) = metadata_events(&page);
        page.request_close();
        tabs.cdp()
            .send("Target.closeTarget", json!({"targetId":PAGE}), None)
            .await
            .unwrap();
        {
            let mut metadata = page.frame_metadata.lock().unwrap();
            let tree = metadata.known.clone();
            let snapshot = metadata.begin_snapshot(tabs.cdp());
            sender
                .send(CdpEvent {
                    method: "Page.frameDetached".to_string(),
                    params: json!({"frameId":"local","reason":"remove"}),
                    session_id: Some(session),
                })
                .unwrap();
            assert!(!metadata.finish_snapshot(&tree, snapshot));
        }
        tabs.cdp().disconnect().await;
        assert!(frame(&tabs, &page, &local, FrameField::Name).await.is_err());
        page.closed.cancel();
        assert_eq!(
            frame(&tabs, &page, &local, FrameField::IsDetached)
                .await
                .unwrap(),
            false
        );
    }

    #[tokio::test]
    async fn tests_that_an_observation_started_before_close_preserves_the_closing_snapshot() {
        let (_browser, tabs, page) = frame_fixture().await;
        let local = FrameScope::Frame("local".to_string());
        frame(&tabs, &page, &local, FrameField::Name).await.unwrap();
        let (sender, session) = metadata_events(&page);
        let (tree, snapshot) = {
            let mut metadata = page.frame_metadata.lock().unwrap();
            (metadata.known.clone(), metadata.begin_snapshot(tabs.cdp()))
        };
        page.request_close();
        tabs.cdp()
            .send("Target.closeTarget", json!({"targetId":PAGE}), None)
            .await
            .unwrap();
        sender
            .send(CdpEvent {
                method: "Page.frameDetached".to_string(),
                params: json!({"frameId":"local","reason":"remove"}),
                session_id: Some(session),
            })
            .unwrap();
        assert!(
            !page
                .frame_metadata
                .lock()
                .unwrap()
                .finish_snapshot(&tree, snapshot)
        );
        tabs.cdp().disconnect().await;
        assert!(frame(&tabs, &page, &local, FrameField::Name).await.is_err());
        page.closed.cancel();
        assert_eq!(
            frame(&tabs, &page, &local, FrameField::IsDetached)
                .await
                .unwrap(),
            false
        );
    }

    #[tokio::test]
    async fn tests_that_a_disappearing_child_host_requests_fresh_discovery() {
        let (_browser, tabs, page, host) = remote_fixture().await;
        host.gone().cancel();
        assert!(watch_frame_host(&page, &host).await.unwrap());
        assert_eq!(
            frame(
                &tabs,
                &page,
                &FrameScope::Frame("remote".to_string()),
                FrameField::Name
            )
            .await
            .unwrap(),
            "remote"
        );
    }

    #[tokio::test]
    async fn tests_that_child_host_setup_preserves_connection_failures() {
        let (_browser, tabs, page, host) = remote_fixture().await;
        tabs.cdp().disconnect().await;
        assert!(watch_frame_host(&page, &host).await.is_err());
    }

    #[test]
    fn tests_that_url_matching_reuses_all_recorded_playwright_cases() {
        let goldens: Value =
            serde_json::from_str(include_str!("../tests/fixtures/url-match-goldens.json")).unwrap();
        for golden in goldens["globs"].as_array().unwrap() {
            let matcher: SerializedUrlMatch =
                serde_json::from_value(json!({"glob": golden["glob"]})).unwrap();
            if golden.get("error").is_some() {
                assert!(
                    matches_url("https://a.test/", &matcher).is_err(),
                    "{golden}"
                );
                continue;
            }
            for case in golden["urls"].as_array().unwrap() {
                assert_eq!(
                    matches_url(case[0].as_str().unwrap(), &matcher).unwrap(),
                    case[1].as_bool().unwrap(),
                    "{golden}: {case}"
                );
            }
        }
        for golden in goldens["regexes"].as_array().unwrap() {
            let matcher: SerializedUrlMatch = serde_json::from_value(
                json!({"regexSource": golden["source"], "regexFlags": golden["flags"]}),
            )
            .unwrap();
            for case in golden["urls"].as_array().unwrap() {
                assert_eq!(
                    matches_url(case[0].as_str().unwrap(), &matcher).unwrap(),
                    case[1].as_bool().unwrap(),
                    "{golden}: {case}"
                );
            }
        }
    }
}
