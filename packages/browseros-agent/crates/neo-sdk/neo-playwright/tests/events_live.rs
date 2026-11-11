//! Page and context events on a real browser, through `Playwright::invoke`: popups of every kind
//! with their opener, a popup window refused, the context's new tabs, dialogs for a listener,
//! for a waiter and for nobody (dismissed after the grace), a dialog whose answer dropped before
//! the browser heard it (still dismissed), tab closes, and console messages and page errors,
//! which BrowserOS sends only outside hosted mode.
//!
//! Needs a throwaway BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005). Set
//! `BROWSERCLAW_HOSTED_MODE=1` when that browser runs in hosted mode, as `spikes/eval/browser.sh`
//! starts it: then the console test checks that nothing arrives. Every test opens its own tabs
//! and closes them.

#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "the capture refusal and the call record serve pages_live, which includes this too"
)]
mod live_run;
#[path = "support/page_server.rs"]
mod page_server;

use std::env;
use std::future::{self, Future};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use browseros_cdp::{CdpClient, CdpError, CdpEvent, ObservedEvents};
use browseros_core::{CdpConnection, SessionId};
use futures_util::future::BoxFuture;
use live_run::{LiveRun, TestPage, TestResult, brand, test_page};
use neo_playwright::{ErrorCode, PlaywrightError};
use neo_trace::ErrorOwner;
use page_server::PageServer;
use serde_json::{Value, json};
use tokio::sync::{Notify, broadcast, mpsc};
use tokio::time;

/// Opens a tab, runs `body` on it, and closes the tab whatever happened.
async fn with_page<F, Fut>(body: F) -> TestResult
where
    F: FnOnce(LiveRun, PageServer, TestPage) -> Fut,
    Fut: Future<Output = (LiveRun, TestResult)>,
{
    let run = LiveRun::connect().await?;
    let server = PageServer::start().await?;
    let page = run.new_page().await?;
    // Boxed: two ops raced in one test make a future too large for the stack lint.
    let (run, result) = Box::pin(body(run, server, page.clone())).await;
    run.close_page(&page).await;
    result
}

/// `Date.now()`, as the prelude stamps a `waitForEvent` or an `on`.
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

/// `page.waitForEvent(event)` stamped at `since`, with a 5 s timeout.
async fn wait_for(
    run: &LiveRun,
    page: Option<&TestPage>,
    event: &str,
    since: u64,
) -> Result<Value, PlaywrightError> {
    let mut input = json!({ "call": { "api": "page.waitForEvent", "timeoutMs": 5000 },
                            "event": event, "since": since });
    if let (Some(page), Some(input)) = (page, input.as_object_mut()) {
        input.insert("page".to_owned(), brand(page));
    }
    run.invoke("events.wait", input).await
}

/// A trusted click that starts once a concurrent wait is surely listening, as a script's
/// `const p = page.waitForEvent(..); await page.click(..)` would.
async fn click_soon(run: &LiveRun, page: &TestPage, selector: &str) -> TestResult {
    time::sleep(Duration::from_millis(50)).await;
    run.click(page, selector).await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_every_popup_kind_names_its_opener_and_a_window_is_refused() -> TestResult {
    with_page(|run, server, page| async move {
        let result = async {
            open(&run, &page, &server.url("/popups.html")).await?;
            // Every way a page opens a tab: each is a popup of this page, noopener ones
            // included, as upstream reads `openerId`.
            for (kind, path) in [
                ("#tab", "/child.html?tab"),
                ("#noopener", "/child.html?noopener"),
                ("#blank", "/child.html?blank"),
                ("#blanknoopener", "/child.html?blank-noopener"),
            ] {
                let since = now_ms()?;
                let (popup, clicked) = tokio::join!(
                    wait_for(&run, Some(&page), "popup", since),
                    click_soon(&run, &page, kind)
                );
                clicked?;
                let popup = popup?;
                assert_eq!(popup["event"], json!("popup"), "{kind}: {popup}");
                assert_eq!(popup["page"], brand(&page), "{kind}: on the opener's page");
                assert_eq!(popup["popup"]["url"], json!(server.url(path)), "{kind}");
                let tab = test_page(&popup["popup"]["page"])?;
                let opener = run
                    .invoke("page.opener", json!({ "page": brand(&tab) }))
                    .await;
                let listed = run.invoke("context.pages", json!({})).await;
                // A popup is a tab core did not open, so it has no focus emulation of its own.
                let shot = time::timeout(
                    Duration::from_secs(5),
                    run.invoke("page.screenshot", json!({ "page": brand(&tab) })),
                )
                .await;
                run.close_page(&tab).await;
                // Upstream's opener() reads `openerId` too, which noopener popups also carry.
                assert_eq!(opener?["opener"], brand(&page), "{kind}: page.opener()");
                let shot =
                    shot.map_err(|_elapsed| format!("{kind}: the popup's capture hung"))??;
                assert_eq!(shot["$neo"], json!("bytes"), "{kind}: a popup tab captures");
                let listed = listed?;
                assert!(
                    listed["pages"].as_array().is_some_and(|pages| pages
                        .iter()
                        .any(|entry| entry["page"] == brand(&tab))),
                    "{kind}: the popup is one of the context's pages"
                );
            }
            // window.open with window features opens a separate popup window (measured, with
            // hosted mode on and off alike), which Neo cannot drive.
            for kind in ["#open", "#empty"] {
                let since = now_ms()?;
                let (popup, clicked) = tokio::join!(
                    wait_for(&run, Some(&page), "popup", since),
                    click_soon(&run, &page, kind)
                );
                clicked?;
                let error = popup
                    .err()
                    .ok_or(format!("{kind}: a popup window is refused"))?;
                assert_eq!(
                    error.error_code(),
                    Some(ErrorCode::Unsupported),
                    "{kind}: {error}"
                );
                assert_eq!(error.owner(), ErrorOwner::Neo);
                assert!(
                    error
                        .message()
                        .starts_with("page.waitForEvent: the popup (")
                        && error.message().contains("opened as a separate window"),
                    "{error}"
                );
            }
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_context_reports_new_tabs_and_a_page_reports_its_close() -> TestResult {
    with_page(|run, _server, page| async move {
        let result = async {
            let since = now_ms()?;
            let (opened, other) = tokio::join!(wait_for(&run, None, "page", since), async {
                time::sleep(Duration::from_millis(50)).await;
                run.new_page().await
            });
            let other = other?;
            let mut opened = opened?;
            // A context reports every tab that opens, and a fresh browser can open its own
            // startup tab after the wait began. Skip those, as a script's predicate would.
            for _ in 0..4 {
                if opened["page"] == brand(&other) {
                    break;
                }
                let after = opened["seq"].clone();
                let input = json!({ "call": { "api": "context.waitForEvent", "timeoutMs": 5000 },
                                        "event": "page", "since": since, "after": after });
                opened = run.invoke("events.wait", input).await?;
            }
            assert_eq!(opened["event"], json!("page"));
            assert_eq!(opened["page"], brand(&other), "{opened}");
            assert_eq!(opened["opener"], Value::Null, "Neo's own tab has no opener");

            let since = now_ms()?;
            let (closed, closing) =
                tokio::join!(wait_for(&run, Some(&other), "close", since), async {
                    time::sleep(Duration::from_millis(50)).await;
                    run.invoke("page.close", json!({ "page": brand(&other) }))
                        .await
                });
            closing?;
            let closed = closed?;
            let until = now_ms()?;
            assert_eq!(closed["event"], json!("close"));
            assert_eq!(closed["page"], brand(&other));
            assert!(
                closed["at"]
                    .as_u64()
                    .is_some_and(|at| since <= at && at <= until),
                "events.wait stamps the close with its record time: {closed}"
            );

            let error = wait_for(&run, Some(&page), "load", now_ms()?)
                .await
                .err()
                .ok_or("page.waitForEvent('load') is not a Neo event")?;
            assert_eq!(error.error_code(), Some(ErrorCode::Unsupported));
            assert!(
                error.message().contains("page.waitForLoadState('load')"),
                "{error}"
            );
            let error = wait_for(&run, Some(&page), "close", now_ms()?)
                .await
                .err()
                .ok_or("an open tab does not close by itself")?;
            assert_eq!(error.error_code(), Some(ErrorCode::WaitTimeout));
            assert_eq!(
                error.message(),
                "page.waitForEvent: Timeout 5000ms exceeded while waiting for event \"close\""
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_dialog_reaches_its_listener_and_takes_its_answer() -> TestResult {
    with_page(|run, server, page| async move {
        let result = async {
            open(&run, &page, &server.url("/popups.html")).await?;
            let since = now_ms()?;
            run.invoke(
                "events.listen",
                json!({ "page": brand(&page), "event": "dialog", "on": true, "since": since }),
            )
            .await?;
            // The pump, as the prelude runs it for page.on('dialog'), answers while the click
            // that opened the dialog is still pending.
            let pumped = async {
                let pumped = run.invoke("events.next", json!({})).await?;
                let dialog = &pumped["events"][0];
                run.invoke(
                    "dialog.accept",
                    json!({ "page": brand(&page), "dialog": dialog["dialog"]["id"],
                            "promptText": "Grace" }),
                )
                .await?;
                Ok::<_, PlaywrightError>(pumped)
            };
            let (pumped, clicked) = tokio::join!(pumped, click_soon(&run, &page, "#prompt"));
            clicked?;
            let pumped = pumped?;
            let until = now_ms()?;
            let dialog = &pumped["events"][0];
            assert_eq!(dialog["event"], json!("dialog"), "{pumped}");
            assert!(
                dialog["at"]
                    .as_u64()
                    .is_some_and(|at| since <= at && at <= until),
                "events.next stamps the dialog with its record time: {pumped}"
            );
            assert_eq!(dialog["page"], brand(&page));
            assert_eq!(dialog["dialog"]["type"], json!("prompt"));
            assert_eq!(dialog["dialog"]["message"], json!("Name?"));
            assert_eq!(dialog["dialog"]["defaultValue"], json!("Ada"));
            assert_eq!(run.eval(&page, "window.__answer").await?, json!("Grace"));

            let again = run
                .invoke(
                    "dialog.dismiss",
                    json!({ "page": brand(&page), "dialog": dialog["dialog"]["id"] }),
                )
                .await
                .err()
                .ok_or("a dialog is answered once")?;
            assert_eq!(
                again.message(),
                "dialog.dismiss: Cannot dismiss dialog which is already handled!"
            );

            // A waitForEvent holds a dialog the same way.
            run.invoke(
                "events.listen",
                json!({ "page": brand(&page), "event": "dialog", "on": false }),
            )
            .await?;
            let since = now_ms()?;
            let waited = async {
                let dialog = wait_for(&run, Some(&page), "dialog", since).await?;
                run.invoke(
                    "dialog.dismiss",
                    json!({ "page": brand(&page), "dialog": dialog["dialog"]["id"] }),
                )
                .await?;
                Ok::<_, PlaywrightError>(dialog)
            };
            let (waited, clicked) = tokio::join!(waited, click_soon(&run, &page, "#confirm"));
            clicked?;
            assert_eq!(waited?["dialog"]["type"], json!("confirm"));
            assert_eq!(run.eval(&page, "window.__answer").await?, json!(false));
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_dialog_nobody_listens_for_is_dismissed_after_the_grace() -> TestResult {
    with_page(|run, server, page| async move {
        let result = async {
            open(&run, &page, &server.url("/popups.html")).await?;
            run.eval(&page, "window.__answer = 'unanswered'").await?;
            let started = Instant::now();
            run.click(&page, "#confirm").await?;
            // The page's main thread is held by the dialog, so this read returns once Neo
            // closed it.
            let answer = run.eval(&page, "window.__answer").await?;
            let took = started.elapsed();
            assert_eq!(
                answer,
                json!(false),
                "a confirm nobody answers is dismissed"
            );
            assert!(
                took >= Duration::from_millis(250) && took < Duration::from_secs(2),
                "dismissed after the 250 ms grace, not at once: {took:?}"
            );

            // Alerts use the same unanswered-dialog path as confirms in an SDK session.
            let started = Instant::now();
            run.click(&page, "#alert").await?;
            assert_eq!(run.eval(&page, "window.__answer").await?, json!("alerted"));
            assert!(
                started.elapsed() >= Duration::from_millis(250)
                    && started.elapsed() < Duration::from_secs(2),
                "an unanswered alert is dismissed after the grace"
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// The crate's CDP connection, which can hold one `Page.handleJavaScriptDialog` short of the
/// browser: once armed, the next one's send stays pending until the op's future drops, so the
/// browser never hears that answer, as with an op dropped mid-send. Every other call passes,
/// Neo's own answers from the grace and the reap included.
struct HoldsOneAnswer {
    client: CdpClient,
    armed: AtomicBool,
    holding: Notify,
}

impl HoldsOneAnswer {
    fn new(client: CdpClient) -> Self {
        Self {
            client,
            armed: AtomicBool::new(false),
            holding: Notify::new(),
        }
    }

    /// Holds the next dialog answer.
    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }

    /// Resolves once the armed answer is held.
    async fn holding(&self) {
        self.holding.notified().await;
    }

    /// Whether a call to `method` is the armed answer, which it then holds.
    fn holds(&self, method: &str) -> bool {
        let held =
            method == "Page.handleJavaScriptDialog" && self.armed.swap(false, Ordering::SeqCst);
        if held {
            self.holding.notify_one();
        }
        held
    }
}

impl CdpConnection for HoldsOneAnswer {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        if self.holds(method) {
            return Box::pin(future::pending());
        }
        CdpConnection::send(&self.client, method, params, session)
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        CdpConnection::send_raw_json(&self.client, method, params_json, session)
    }

    fn send_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        if self.holds(method) {
            return Box::pin(future::pending());
        }
        CdpConnection::send_at_epoch(&self.client, expected_epoch, method, params, session)
    }

    fn send_raw_json_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        CdpConnection::send_raw_json_at_epoch(
            &self.client,
            expected_epoch,
            method,
            params_json,
            session,
        )
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        CdpConnection::events(&self.client)
    }

    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        CdpConnection::events_targeted(&self.client, method)
    }

    fn observe_events(&self, methods: &[&str]) -> Option<ObservedEvents> {
        CdpConnection::observe_events(&self.client, methods)
    }

    fn is_connected(&self) -> bool {
        CdpConnection::is_connected(&self.client)
    }

    fn connection_epoch(&self) -> u64 {
        CdpConnection::connection_epoch(&self.client)
    }
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_dialog_whose_answer_dropped_mid_send_is_still_dismissed() -> TestResult {
    let mut holder = None;
    let mut run = LiveRun::connect_through(|client| {
        let connection = Arc::new(HoldsOneAnswer::new(client));
        holder = Some(Arc::clone(&connection));
        connection as Arc<dyn CdpConnection>
    })
    .await?;
    let connection = holder.ok_or("the run connects through the holding connection")?;
    let server = PageServer::start().await?;
    let page = run.new_page().await?;
    let result = dropped_answers(&mut run, &connection, &server, &page).await;
    run.close_page(&page).await;
    result
}

/// A listening run answers a confirm, the answer drops while the browser has not heard it, and
/// the run ends. Dropped in the grace, the grace dismisses the dialog; dropped after it, the reap.
async fn dropped_answers(
    run: &mut LiveRun,
    connection: &HoldsOneAnswer,
    server: &PageServer,
    page: &TestPage,
) -> TestResult {
    open(run, page, &server.url("/popups.html")).await?;
    for (held_for, closer) in [
        (Duration::ZERO, "the grace"),
        (Duration::from_millis(600), "the reap"),
    ] {
        run.eval(page, "window.__answer = 'unanswered'").await?;
        run.invoke(
            "events.listen",
            json!({ "page": brand(page), "event": "dialog", "on": true, "since": now_ms()? }),
        )
        .await?;
        // From a timer, so this returns before the confirm blocks the page.
        run.eval(
            page,
            "setTimeout(() => document.querySelector('#confirm').click(), 50)",
        )
        .await?;
        let pumped = run.invoke("events.next", json!({})).await?;
        let dialog = pumped.pointer("/events/0/dialog");
        assert_eq!(
            dialog.and_then(|dialog| dialog.get("type")),
            Some(&json!("confirm")),
            "{closer}: {pumped}"
        );
        connection.arm();
        let mut answer = Box::pin(run.invoke(
            "dialog.accept",
            json!({ "page": brand(page), "dialog": dialog.and_then(|dialog| dialog.get("id")) }),
        ));
        tokio::select! {
            answered = &mut answer => {
                return Err(format!("{closer}: the held answer ended: {answered:?}").into());
            }
            () = connection.holding() => {}
        }
        time::sleep(held_for).await;
        drop(answer);
        run.next_run();
        // The page's main thread is held by the dialog, so this read returns once it closed.
        let answer = time::timeout(Duration::from_secs(5), run.eval(page, "window.__answer"))
            .await
            .map_err(|_elapsed| {
                format!("{closer}: the dialog stayed open after its answer dropped")
            })??;
        assert_eq!(answer, json!(false), "{closer} dismissed the confirm");
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_console_and_page_errors_reach_listeners_outside_hosted_mode() -> TestResult {
    let hosted = env::var("BROWSERCLAW_HOSTED_MODE").is_ok_and(|mode| mode == "1");
    with_page(|run, server, page| async move {
        let result = async {
            open(&run, &page, &server.url("/popups.html")).await?;
            for event in ["console", "pageerror"] {
                run.invoke(
                    "events.listen",
                    json!({ "page": brand(&page), "event": event, "on": true, "since": now_ms()? }),
                )
                .await?;
            }
            run.click(&page, "#log").await?;
            let mut seen = Vec::new();
            while seen.len() < 2 {
                let Ok(pumped) = time::timeout(
                    Duration::from_millis(1500),
                    run.invoke("events.next", json!({})),
                )
                .await
                else {
                    break;
                };
                seen.extend(pumped?["events"].as_array().cloned().unwrap_or_default());
            }
            if hosted {
                assert!(
                    seen.is_empty(),
                    "hosted mode sends no console or exception notifications: {seen:?}"
                );
                return Ok(());
            }
            let console = seen
                .iter()
                .find(|event| event["event"] == "console")
                .ok_or(format!("a console event: {seen:?}"))?;
            assert_eq!(console["message"]["type"], json!("log"));
            assert_eq!(console["message"]["text"], json!("clicked 7"));
            let error = seen
                .iter()
                .find(|event| event["event"] == "pageerror")
                .ok_or(format!("a pageerror event: {seen:?}"))?;
            assert_eq!(error["error"]["name"], json!("Error"));
            assert_eq!(error["error"]["message"], json!("boom"));
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}
