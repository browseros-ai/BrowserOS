//! Live reads of page state, nothing cached. Each source was chosen because it matches what
//! real Playwright returns at the same moment, including while a navigation is in flight (see
//! `sync-getters-design.md`). A read is normally one CDP call. The exception is a `url` read
//! that lands in the few milliseconds of a cross-document commit, when BrowserOS refuses
//! navigation history: it checks the tab is still open and retries until the new document
//! answers. The main frame's URL uses that same history source, including in frame records.
//! Frame-tree getters still call Page.getFrameTree, which can block until a pending main-frame
//! navigation commits; their main-frame URL is read from history after the tree answers.

use std::time::Duration;

use browseros_cdp::{CdpClient, CdpError, SessionId, browser};
use neo_runtime_spec::serve::{FrameChannel, FrameInitializer};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time;
use tokio::time::Instant;

use crate::bridge;
use crate::click::PageError;
use crate::frames::{FrameInfo, FrameScope};
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

/// A direct frame's live field. Detached metadata is null so the SDK can return its last
/// observed URL/name; a removed frame has no parent or children. Only the main URL avoids
/// Page.getFrameTree's possible wait for a pending main-frame navigation to commit.
pub(crate) async fn frame(
    tabs: &Tabs,
    page: &PageEntry,
    scope: &FrameScope,
    field: FrameField,
) -> Result<Value, PlaywrightError> {
    if matches!((scope, field), (FrameScope::Main, FrameField::Url)) {
        return tabs.url(page).await.map(|url| json!(url));
    }
    let tree = frame_tree(tabs, page).await?;
    frame_value(&page.id, &tree, scope, field)
}

/// Neo: Keeps frame records on the same committed-URL source as the page getter.
async fn frame_tree(tabs: &Tabs, page: &PageEntry) -> Result<Vec<FrameInfo>, PlaywrightError> {
    let session = tabs.session(page).await?;
    let mut tree = session.frames.tree().await.map_err(frame_tree_error)?;
    if let Some(main) = tree.first_mut() {
        main.url = tabs
            .url(page)
            .await?
            .ok_or_else(PlaywrightError::target_closed)?;
    }
    Ok(tree)
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
    use serde_json::{Value, json};

    use super::{FrameField, frame_records, frame_value, matches_url};
    use crate::frames::{FrameInfo, FrameScope};
    use crate::run::PageId;
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
