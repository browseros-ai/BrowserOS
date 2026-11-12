//! A live browser for integration tests: one CDP connection with a `Playwright` over it, as
//! claw-sdk builds them, and ops run through `Playwright::invoke` with a plain `OpEnv`. Needs
//! BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005).

use std::env;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use browseros_cdp::{CdpClient, ConnectOptions};
use browseros_core::{
    BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, PageId, TabId, TargetId,
};
use neo_playwright::{Op, OpEnv, Playwright, PlaywrightError, Refusal, RunState};
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
    /// `{"$neo":"page","tab":1234}`, the brand every target carries for this tab.
    pub(crate) fn brand(&self) -> Value {
        json!({ "$neo": "page", "tab": self.tab.0 })
    }
}

/// The host side of one op: an unbound view and no policies, as a run with no takeover.
pub(crate) struct LiveEnv {
    view: BrowserView,
    run_state: Arc<RunState>,
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

    fn mark_sent(&self) {}

    fn note(&self, _line: String) {}
}

/// One CDP connection with a `Playwright` over it.
pub(crate) struct LiveBrowser {
    pub(crate) browser: Arc<BrowserSession>,
    pub(crate) playwright: Arc<Playwright>,
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
        let playwright = Playwright::new(Arc::clone(&browser));
        let live = Self {
            browser,
            playwright,
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

    /// Evaluates `expression` in the page's own main world.
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
        self.invoke_in_run(Arc::default(), op, input).await
    }

    /// Reuses one agent run's caches across native operations, as the server's host does.
    pub(crate) async fn invoke_in_run(
        &self,
        run_state: Arc<RunState>,
        op: &str,
        input: Value,
    ) -> TestResult<Result<Value, PlaywrightError>> {
        let op = Op::from_name(op).ok_or(format!("{op} is not an op"))?;
        let env = LiveEnv {
            view: BrowserView::unbound(Arc::clone(&self.browser), None),
            run_state,
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
