//! Every tab the crate has acted on, and where each one's seqs start.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use browseros_core::{BrowserSession, PageId, TargetId};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::TypedBy;
use crate::input::Platform;
use crate::op_scope::AdmittedPage;
use crate::page::PageState;
use crate::world::FrameSeq;

/// The lowest seq a page that was already open may start at.
const REOPENED_BASE_MIN: u32 = 100_000;
/// How many bases there are to pick from: 100000 to 999999.
const REOPENED_BASES: u32 = 900_000;

/// The page states of one `Playwright`, by page id. An entry stays while the instance lives
/// (see `PageState` for why).
///
/// Where a page's seqs start depends on whether the tab was open when this `Playwright`
/// started. A tab first seen after that starts at 0, so its main frame's refs print bare
/// (`e5`), as upstream's do. A tab that was already open may hold refs a previous claw-sdk
/// process printed, and every bare `e5` of that process named seq 0; such a tab starts at a
/// random base from 100000 to 999999 instead.
///
/// Two layers keep an old ref from naming a document of the new numbering. Every world the
/// new instance re-keys reports the seq it held, and from then on the page never hands that
/// one out (`FrameWorlds`). What is left is an old document whose seq a new document takes
/// before this instance meets the old one, or that it never meets: both need the two ranges
/// to overlap, about 1e-5 per restarted page with this many bases (decisions.md, M1 review).
#[derive(Debug)]
pub(crate) struct PageRegistry {
    pages: Mutex<HashMap<PageId, Arc<PageState>>>,
    /// The targets open when this instance started, read once.
    open_at_start: OnceCell<OpenAtStart>,
    /// The first seq of a tab that was already open.
    reopened_base: FrameSeq,
    /// The browser's platform, read once: it decides what `ControlOrMeta` presses.
    platform: OnceCell<Platform>,
}

/// The tabs that were open when this `Playwright` started.
#[derive(Debug)]
enum OpenAtStart {
    Known(HashSet<TargetId>),
    /// The list could not be read. Every tab counts as already open: a random base only
    /// changes how refs print, and 0 could let an old ref through.
    Unknown,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Targets {
    target_infos: Vec<TargetInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetInfo {
    target_id: TargetId,
}

/// The browser did not say what it runs on.
struct Unanswered;

/// `Browser.getVersion`'s answer, of which only the user agent is read.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Version {
    user_agent: String,
}

impl PageRegistry {
    /// An empty registry with a fresh random base.
    pub(crate) fn new() -> Self {
        let pick = u32::try_from(Uuid::new_v4().as_u128() % u128::from(REOPENED_BASES))
            .unwrap_or_default();
        Self {
            pages: Mutex::default(),
            open_at_start: OnceCell::new(),
            reopened_base: FrameSeq(REOPENED_BASE_MIN + pick),
            platform: OnceCell::new(),
        }
    }

    /// Reads which tabs are open, once. `Playwright::new` starts it right away, so the list is
    /// the one from the instance's start; an op that comes first waits for the same read.
    pub(crate) async fn note_open_tabs(&self, browser: &BrowserSession) {
        self.open_at_start(browser).await;
        self.platform(browser).await;
    }

    /// The state of the tab `admitted` names, created on first use.
    pub(crate) async fn page(
        &self,
        browser: &BrowserSession,
        admitted: &AdmittedPage,
    ) -> Arc<PageState> {
        if let Some(page) = self.get(&admitted.id) {
            return page;
        }
        let first = if self
            .open_at_start(browser)
            .await
            .includes(&admitted.session.target_id)
        {
            self.reopened_base
        } else {
            FrameSeq(0)
        };
        let platform = self.platform(browser).await;
        Arc::clone(
            self.lock()
                .entry(admitted.id.clone())
                .or_insert_with(|| Arc::new(PageState::new(first, platform))),
        )
    }

    /// The state of `page`, if an op has acted on it.
    pub(crate) fn get(&self, page: &PageId) -> Option<Arc<PageState>> {
        self.lock().get(page).map(Arc::clone)
    }

    /// Who typed a value the journal saw on `page` just now.
    pub(crate) fn typed_by(&self, page: &PageId) -> TypedBy {
        self.get(page)
            .map_or(TypedBy::NotAgent, |state| state.typed_by())
    }

    /// Upstream's `CRBrowser._platform()`, from the user agent. A browser that does not answer
    /// reads as Linux, the hosted cell's platform, without being remembered, so the next op
    /// asks again.
    async fn platform(&self, browser: &BrowserSession) -> Platform {
        self.platform
            .get_or_try_init(|| async {
                let version = browser
                    .cdp("Browser.getVersion", json!({}), None)
                    .await
                    .ok()
                    .and_then(|version: Value| serde_json::from_value::<Version>(version).ok());
                version
                    .map(|version| Platform::from_user_agent(&version.user_agent))
                    .ok_or(Unanswered)
            })
            .await
            .copied()
            .unwrap_or(Platform::Linux)
    }

    async fn open_at_start(&self, browser: &BrowserSession) -> &OpenAtStart {
        self.open_at_start
            .get_or_init(|| async {
                let targets = browser
                    .cdp("Target.getTargets", json!({}), None)
                    .await
                    .ok()
                    .and_then(|targets: Value| serde_json::from_value::<Targets>(targets).ok());
                targets.map_or(OpenAtStart::Unknown, |targets| {
                    OpenAtStart::Known(
                        targets
                            .target_infos
                            .into_iter()
                            .map(|info| info.target_id)
                            .collect(),
                    )
                })
            })
            .await
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<PageId, Arc<PageState>>> {
        self.pages.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl OpenAtStart {
    fn includes(&self, target: &TargetId) -> bool {
        match self {
            Self::Known(open) => open.contains(target),
            Self::Unknown => true,
        }
    }
}
