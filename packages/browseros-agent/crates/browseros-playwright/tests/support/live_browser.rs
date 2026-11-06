//! A live browser for integration tests: one CDP connection with a `Playwright` over it, as
//! claw-sdk builds them, ops run through `Playwright::invoke` with a plain `OpEnv`, and hooks
//! that record what the journal reports. Needs BrowserOS with CDP on `BROWSEROS_CDP_PORT`
//! (default 9005).

use std::env;
use std::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use browseros_cdp::{CdpClient, ConnectOptions};
use browseros_core::sensitive_fields::{FieldValues, JournalEntry, SensitiveKind};
use browseros_core::{
    BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, PageId, TabId, TargetId,
};
use browseros_playwright::{
    Op, OpEnv, Playwright, PlaywrightError, PlaywrightHooks, Refusal, RunState, TypedBy,
};
use serde_json::{Value, json};
use tokio::time;

pub(crate) type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// A tab a test opened, as a page brand names it: core's page id and the browser's tab id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TestPage {
    pub(crate) id: PageId,
    pub(crate) tab: TabId,
}

impl TestPage {
    /// `{"$neo":"page","id":7,"tab":1234}`, the brand every target carries for this tab.
    pub(crate) fn brand(&self) -> Value {
        json!({ "$neo": "page", "id": self.id.0, "tab": self.tab.0 })
    }
}

/// One value the journal reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Journaled {
    pub(crate) page: PageId,
    pub(crate) kind: SensitiveKind,
    pub(crate) value: String,
    pub(crate) by: TypedBy,
}

/// One value an op swept from a page and reported to the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Swept {
    pub(crate) page: PageId,
    pub(crate) value: String,
}

/// Hooks that remember what the journal reported and which values ops swept.
#[derive(Default)]
pub(crate) struct RecordingHooks {
    journaled: Mutex<Vec<Journaled>>,
    swept: Mutex<Vec<Swept>>,
}

impl RecordingHooks {
    pub(crate) fn journaled(&self) -> Vec<Journaled> {
        self.journaled
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Every value `values_swept` reported, with its page.
    pub(crate) fn swept(&self) -> Vec<Swept> {
        self.swept
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

    fn values_swept(&self, page: PageId, values: &FieldValues) {
        let mut swept = self.swept.lock().unwrap_or_else(PoisonError::into_inner);
        swept.extend(values.registrable().map(|value| Swept {
            page: page.clone(),
            value: value.to_owned(),
        }));
    }

    fn capture_allowed(&self, _page: PageId) -> Result<(), Refusal> {
        Ok(())
    }
}

/// The host side of one op: an unbound view and no policies, as a run with no takeover.
pub(crate) struct LiveEnv {
    view: BrowserView,
    run_state: Arc<RunState>,
    sensitive_marks: Arc<AtomicUsize>,
}

impl OpEnv for LiveEnv {
    fn view(&self) -> &BrowserView {
        &self.view
    }

    fn run_state(&self) -> &Arc<RunState> {
        &self.run_state
    }

    fn recheck(&self) -> Result<(), Refusal> {
        Ok(())
    }

    /// The owner holds no tab, so the rules allow every tab of a page's group too.
    fn recheck_reach(&self, _tabs: &[TabId]) -> Result<(), Refusal> {
        Ok(())
    }

    fn mark_sent(&self) {}

    fn mark_sensitive_target(&self) {
        self.sensitive_marks.fetch_add(1, Ordering::Relaxed);
    }

    fn note(&self, _line: String) {}
}

/// One CDP connection with a `Playwright` over it.
pub(crate) struct LiveBrowser {
    pub(crate) browser: Arc<BrowserSession>,
    pub(crate) playwright: Arc<Playwright>,
    pub(crate) hooks: Arc<RecordingHooks>,
    /// How many ops so far told the host their target is a password, card or one-time-code
    /// field (`OpEnv::mark_sensitive_target`).
    pub(crate) sensitive_marks: Arc<AtomicUsize>,
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
        let live = Self {
            browser,
            playwright,
            hooks,
            sensitive_marks: Arc::default(),
        };
        // `Playwright::new` reads the open tabs in the background, and that read decides
        // where a tab's seqs start. An op on a tab that is already open waits for it, so a
        // tab the test opens afterwards counts as new and prints bare refs, as a real agent
        // tab does.
        if let Some(open) = live.browser.pages.list().await?.into_iter().next() {
            let html = json!({ "parts": [{
                "name": "css",
                "body": [{ "simples": [{ "selector": { "css": "html", "functions": [] }, "combinator": "" }] }],
                "source": "html",
            }] });
            let page = TestPage {
                id: open.page_id,
                tab: open.tab_id,
            };
            let target = json!({ "$neo": "locator", "page": page.brand(), "chunks": [html], "display": "html" });
            live.invoke("locator.count", json!({ "target": target }))
                .await??;
        }
        Ok(live)
    }

    /// Opens `url` in a new tab, waits for it to load, and returns the tab's page.
    pub(crate) async fn open(&self, url: &str) -> TestResult<TestPage> {
        let created = self
            .browser
            .cdp("Target.createTarget", json!({ "url": url }), None)
            .await?;
        let target = TargetId(
            created
                .get("targetId")
                .and_then(Value::as_str)
                .ok_or("no target id")?
                .to_owned(),
        );
        let info = self
            .browser
            .pages
            .list()
            .await?
            .into_iter()
            .find(|info| info.target_id == target)
            .ok_or("the tab is not listed")?;
        let page = TestPage {
            id: info.page_id,
            tab: info.tab_id,
        };
        let loaded = format!(
            "document.readyState === 'complete' && location.href === {}",
            serde_json::to_string(url)?
        );
        for _ in 0..100 {
            if self.main_world(&page, &loaded).await? == Value::Bool(true) {
                return Ok(page);
            }
            time::sleep(Duration::from_millis(50)).await;
        }
        Err(format!("{url} did not load within 5 s").into())
    }

    /// Evaluates `expression` in the page's own main world, which the seal leaves alone.
    pub(crate) async fn main_world(&self, page: &TestPage, expression: &str) -> TestResult<Value> {
        let session = self.browser.pages.get_session(page.id.clone()).await?;
        let result: Value = session
            .session
            .send(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
            )
            .await?;
        Ok(result
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// Runs `op` through `Playwright::invoke`, as claw-sdk's handler does.
    pub(crate) async fn invoke(
        &self,
        op: &str,
        input: Value,
    ) -> TestResult<Result<Value, PlaywrightError>> {
        let op = Op::from_name(op).ok_or(format!("{op} is not an op"))?;
        let env = LiveEnv {
            view: BrowserView::unbound(Arc::clone(&self.browser), None),
            run_state: Arc::default(),
            sensitive_marks: Arc::clone(&self.sensitive_marks),
        };
        Ok(self.playwright.invoke(&env, op, input).await)
    }

    pub(crate) async fn close(&self, page: &TestPage) -> TestResult {
        let info = self
            .browser
            .pages
            .get_info(page.id.clone())
            .await
            .ok_or("no such page")?;
        self.browser
            .cdp(
                "Target.closeTarget",
                json!({ "targetId": info.target_id.as_str() }),
                None,
            )
            .await?;
        Ok(())
    }
}
