//! Per-tab event rings against a real browser (`src/events/event_hub.rs`): a busy tab pushes out
//! only its own records. A suite of its own, so a browser of its own (`live.py` gives each suite
//! one): its untouched ad tab churns cross-site frames hard enough to slow timing-sensitive cases
//! that share a browser with it in a parallel local run.

#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "the ring row drives one page and reads the log's head; the rest serves other suites"
)]
mod live_run;
#[path = "support/page_server.rs"]
mod page_server;

use std::error::Error;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use browseros_cdp::{CdpClient, ConnectOptions};
use browseros_core::{CdpConnection, ProtocolSession, SessionId};
use live_run::{LiveRun, TestPage, TestResult, brand};
use neo_playwright::PlaywrightError;
use page_server::PageServer;
use serde_json::{Value, json};
use tokio::time;

async fn with_page<F, Fut>(body: F) -> TestResult
where
    F: FnOnce(LiveRun, PageServer, TestPage) -> Fut,
    Fut: Future<Output = (LiveRun, TestResult)>,
{
    let run = LiveRun::connect().await?;
    let server = PageServer::start().await?;
    let page = run.new_page().await?;
    let (run, result) = Box::pin(body(run, server, page.clone())).await;
    run.close_page(&page).await;
    result
}

/// `Date.now()`, as the prelude stamps an `on`.
fn now_ms() -> TestResult<u64> {
    Ok(u64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    )?)
}

async fn open(run: &LiveRun, page: &TestPage, url: &str) -> Result<Value, PlaywrightError> {
    run.invoke(
        "page.goto",
        json!({ "call": { "timeoutMs": 30_000 }, "page": brand(page), "url": url }),
    )
    .await
}

/// A background tab opened over a CDP connection of the test's own: neither the run's core nor
/// its watcher hears of this connection's sessions, so the tab is one Neo never touched. Its
/// cross-site frames still report to the watcher's own observer, as an ad page's do.
struct UntouchedTab {
    client: Arc<CdpClient>,
    target: String,
    session: ProtocolSession,
}

impl UntouchedTab {
    async fn open(url: &str) -> TestResult<Self> {
        let client = Arc::new(CdpClient::connect(ConnectOptions::new(live_run::cdp_port())).await?);
        let transport: Arc<dyn CdpConnection> = Arc::<CdpClient>::clone(&client);
        let created = transport
            .send(
                "Target.createTarget",
                json!({ "url": url, "background": true }),
                None,
            )
            .await;
        let target = match created.map(|created| {
            created
                .get("targetId")
                .and_then(Value::as_str)
                .map(str::to_owned)
        }) {
            Ok(Some(target)) => target,
            Ok(None) => {
                client.disconnect().await;
                return Err("no target id".into());
            }
            Err(error) => {
                client.disconnect().await;
                return Err(error.into());
            }
        };
        let attached = async {
            let attached = transport
                .send(
                    "Target.attachToTarget",
                    json!({ "targetId": target, "flatten": true }),
                    None,
                )
                .await?;
            let session = attached
                .get("sessionId")
                .and_then(Value::as_str)
                .ok_or("no session id")?;
            Ok::<_, Box<dyn Error>>(ProtocolSession::for_session(
                Arc::clone(&transport),
                SessionId::from(session.to_owned()),
            ))
        }
        .await;
        let session = match attached {
            Ok(session) => session,
            Err(error) => {
                // The tab exists: close it, so the browser later cases share stays clean.
                let _closed = transport
                    .send("Target.closeTarget", json!({ "targetId": target }), None)
                    .await;
                client.disconnect().await;
                return Err(error);
            }
        };
        Ok(Self {
            client,
            target,
            session,
        })
    }

    async fn eval(&self, expression: &str) -> TestResult<Value> {
        let reply = self
            .session
            .send_value(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
            )
            .await?;
        Ok(reply
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// Closes the tab, then the connection: dropping a `CdpClient` does not stop its tasks.
    async fn close(self) {
        let _closed = self
            .client
            .send(
                "Target.closeTarget",
                json!({ "targetId": self.target }),
                None,
            )
            .await;
        self.client.disconnect().await;
    }
}

/// How far past one ring (`LOG_CAPACITY`, 1,024 records) the untouched tab must move the log.
const PAST_ONE_RING: u64 = 1_200;

/// A busy tab pushes out only its own records (`event_hub.rs`). A page listener on the run's
/// tab is not pumped while an untouched tab's cross-site ad frames write more than one ring's
/// worth of records; the pump then still delivers the run's tab's navigation, with no loss. On
/// one shared ring the navigation was pushed out and the pump reported the loss.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_busy_untouched_tab_does_not_push_out_a_listened_tabs_events() -> TestResult {
    with_page(|run, server, page| async move {
        let result = async {
            run.invoke(
                "events.listen",
                json!({ "page": brand(&page), "event": "framenavigated", "enabled": true,
                        "since": now_ms()? }),
            )
            .await?;
            let url = server.url("/child.html?listened");
            open(&run, &page, &url).await?;
            let ads = UntouchedTab::open(&server.url("/ad-host.html")).await?;
            let churned = async {
                let mut loaded = false;
                for _ in 0..200 {
                    if ads.eval("document.readyState").await? == json!("complete") {
                        loaded = true;
                        break;
                    }
                    time::sleep(Duration::from_millis(50)).await;
                }
                if !loaded {
                    return Err("the ad host did not load within 10 s".into());
                }
                let head = run.event_head();
                let deadline = Instant::now() + Duration::from_secs(20);
                while run.event_head() < head + PAST_ONE_RING {
                    if Instant::now() > deadline {
                        let wrote = run.event_head() - head;
                        return Err(format!("the ad frames wrote {wrote} records in 20 s").into());
                    }
                    ads.eval("burst(100)").await?;
                    time::sleep(Duration::from_millis(100)).await;
                }
                Ok::<_, Box<dyn Error>>(run.event_head() - head)
            }
            .await;
            ads.close().await;
            let wrote = churned?;
            let pumped = run.invoke("events.next", json!({})).await?;
            let delivered = pumped
                .get("events")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .any(|event| event.pointer("/frame/url") == Some(&json!(url)));
            assert!(
                delivered,
                "the listened tab's navigation is gone after {wrote} records of an untouched tab: {pumped}"
            );
            assert_eq!(pumped.get("lost"), Some(&json!(0)), "{pumped}");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}
