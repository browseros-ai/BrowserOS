//! A live browser for the crate's own live tests: one CDP connection with a `Playwright` over
//! it, as claw-sdk builds them, tabs it opens and closes, and hooks that record what the
//! journal reports. Needs BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005). Also the
//! scripted one-tab browser unit tests run an op against ([`one_tab`]); it lives here because
//! core's `page_js_guard` reads every other non-test file as production code, and the script
//! answers `Runtime.evaluate`.

#[path = "../tests/support/fixture_server.rs"]
pub(crate) mod fixture_server;

use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use browseros_cdp::{CdpClient, ConnectOptions};
use browseros_core::sensitive_fields::{FieldValues, JournalEntry, SensitiveKind};
use browseros_core::testing::TestConnection;
use browseros_core::{
    BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, PageId, ProtocolSession,
    TabId, TargetId,
};
use serde_json::{Value, json};
use tokio::time;

use crate::op::Op;
use crate::test_env::TestEnv;
use crate::{Playwright, PlaywrightHooks, Refusal, TypedBy};

pub(crate) type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// One value the journal reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Journaled {
    pub(crate) page: PageId,
    pub(crate) kind: SensitiveKind,
    pub(crate) value: String,
    pub(crate) by: TypedBy,
}

/// Hooks that remember what the journal reported. They name no secret files unless built
/// with [`RecordingHooks::naming_secrets`], so path uploads are refused, as a host's default.
#[derive(Default)]
pub(crate) struct RecordingHooks {
    journaled: Mutex<Vec<Journaled>>,
    secrets: Option<Vec<PathBuf>>,
}

impl RecordingHooks {
    /// Hooks that name `secrets` as Neo's secret files.
    pub(crate) fn naming_secrets(secrets: Vec<PathBuf>) -> Self {
        Self {
            journaled: Mutex::default(),
            secrets: Some(secrets),
        }
    }

    pub(crate) fn journaled(&self) -> Vec<Journaled> {
        self.journaled
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Waits up to 2 s for `expected` to be reported.
    pub(crate) async fn wait_for(&self, expected: &Journaled) -> bool {
        for _ in 0..40 {
            if self.journaled().contains(expected) {
                return true;
            }
            time::sleep(Duration::from_millis(50)).await;
        }
        false
    }
}

impl PlaywrightHooks for RecordingHooks {
    fn field_entered(&self, page: PageId, entry: JournalEntry, by: TypedBy) {
        self.journaled
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Journaled {
                page,
                kind: entry.kind,
                value: entry.value.to_string(),
                by,
            });
    }

    fn values_swept(&self, _page: PageId, _values: &FieldValues) {}

    fn capture_allowed(&self, _page: PageId) -> Result<(), Refusal> {
        Ok(())
    }

    fn secret_paths(&self) -> Option<Vec<PathBuf>> {
        self.secrets.clone()
    }
}

/// One CDP connection with a `Playwright` over it.
pub(crate) struct LiveBrowser {
    pub(crate) browser: Arc<BrowserSession>,
    pub(crate) playwright: Arc<Playwright>,
    pub(crate) hooks: Arc<RecordingHooks>,
}

impl LiveBrowser {
    pub(crate) async fn connect() -> TestResult<Self> {
        let port = env::var("BROWSEROS_CDP_PORT")
            .ok()
            .and_then(|port| port.parse().ok())
            .unwrap_or(9005);
        let client: Arc<dyn CdpConnection> =
            Arc::new(CdpClient::connect(ConnectOptions::new(port)).await?);
        let browser = BrowserSession::new(client, BrowserSessionHooks::default());
        let hooks = Arc::new(RecordingHooks::default());
        let playwright = Playwright::new(
            Arc::clone(&browser),
            Arc::clone(&hooks) as Arc<dyn PlaywrightHooks>,
        );
        // The start-up read of open tabs, so a tab the test opens next counts as new.
        playwright.pages().note_open_tabs(&browser).await;
        Ok(Self {
            browser,
            playwright,
            hooks,
        })
    }

    /// Opens `url` in a new tab and waits for it to load.
    pub(crate) async fn open(&self, url: &str) -> TestResult<Tab> {
        let created = self
            .browser
            .cdp("Target.createTarget", json!({ "url": url }), None)
            .await?;
        let target = TargetId(
            created["targetId"]
                .as_str()
                .ok_or("no target id")?
                .to_owned(),
        );
        let tab = self.tab(&target).await?;
        tab.wait_for(url).await?;
        Ok(tab)
    }

    /// The tab `target` is, as this connection's core numbers it.
    pub(crate) async fn tab(&self, target: &TargetId) -> TestResult<Tab> {
        let info = self
            .browser
            .pages
            .list()
            .await?
            .into_iter()
            .find(|info| info.target_id == *target)
            .ok_or("the tab is not listed")?;
        let session = self.browser.pages.get_session(info.page_id.clone()).await?;
        Ok(Tab {
            target: target.clone(),
            page: info.page_id,
            id: info.tab_id,
            session: session.session,
        })
    }

    /// The host side of one op on this connection: an unbound view, no policies.
    pub(crate) fn env(&self) -> TestEnv {
        TestEnv::over(BrowserView::unbound(Arc::clone(&self.browser), None))
    }

    pub(crate) async fn close(&self, tab: &Tab) {
        let _closed = self
            .browser
            .cdp(
                "Target.closeTarget",
                json!({ "targetId": tab.target.as_str() }),
                None,
            )
            .await;
    }
}

/// A tab a test opened, with core's session for main-world scripts and input.
pub(crate) struct Tab {
    pub(crate) target: TargetId,
    pub(crate) page: PageId,
    /// The browser's tab id, the page brand's `tab`.
    pub(crate) id: TabId,
    pub(crate) session: ProtocolSession,
}

impl Tab {
    /// `{"$neo":"page","id":7,"tab":1234}`, as a target names this tab.
    pub(crate) fn brand(&self) -> Value {
        json!({ "$neo": "page", "id": self.page.0, "tab": self.id.0 })
    }

    /// The page's own main world, which the seal leaves alone.
    pub(crate) async fn main_world(&self, expression: &str) -> TestResult<Value> {
        let result: Value = self
            .session
            .send(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
            )
            .await?;
        Ok(result["result"]["value"].clone())
    }

    pub(crate) async fn navigate(&self, url: &str) -> TestResult {
        self.session
            .send::<_, Value>("Page.navigate", json!({ "url": url }))
            .await?;
        self.wait_for(url).await
    }

    pub(crate) async fn wait_for(&self, url: &str) -> TestResult {
        let loaded = format!(
            "document.readyState === 'complete' && location.href === {}",
            serde_json::to_string(url)?
        );
        for _ in 0..100 {
            if matches!(self.main_world(&loaded).await, Ok(Value::Bool(true))) {
                return Ok(());
            }
            time::sleep(Duration::from_millis(50)).await;
        }
        Err(format!("{url} did not load within 5 s").into())
    }
}

/// The op live tests open their scopes with; any read op works.
pub(crate) fn count_op() -> TestResult<Op> {
    Ok(Op::from_name("locator.count").ok_or("locator.count is an op")?)
}

/// The scripted browser [`one_tab`] builds, and its one tab.
pub(crate) struct OneTab {
    pub(crate) browser: Arc<BrowserSession>,
    pub(crate) page: PageId,
}

/// A browser over `connection` with one tab, tab 7 on `https://example.test/`, scripted to
/// answer what admitting the tab and preparing a capture send. A test scripts the capture
/// itself.
pub(crate) async fn one_tab(connection: &Arc<TestConnection>) -> Result<OneTab, Box<dyn Error>> {
    connection.always(
        "Browser.getTabs",
        json!({ "tabs": [{ "tabId": 7, "targetId": "target-7", "windowId": 1,
            "url": "https://example.test/", "title": "Account", "isActive": true,
            "isLoading": false, "loadProgress": 1.0, "isPinned": false, "isHidden": false }] }),
    )?;
    for setup in [
        "Page.enable",
        "DOM.enable",
        "Runtime.enable",
        "Accessibility.enable",
    ] {
        connection.always(setup, json!({}))?;
    }
    connection.always("Runtime.evaluate", json!({ "result": { "value": true } }))?;
    let cdp: Arc<dyn CdpConnection> = Arc::clone(connection) as Arc<dyn CdpConnection>;
    let browser = BrowserSession::new(cdp, BrowserSessionHooks::default());
    let page = browser
        .pages
        .list()
        .await?
        .first()
        .ok_or("the tab is listed")?
        .page_id
        .clone();
    Ok(OneTab { browser, page })
}
