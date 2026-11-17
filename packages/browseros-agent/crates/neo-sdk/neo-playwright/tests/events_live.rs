//! Page and context events on a real browser, through `Playwright::invoke`: popups of every kind
//! with their opener, a popup window refused, the context's new tabs, dialogs for a listener,
//! for a waiter and for nobody (dismissed after the grace), a dialog whose answer dropped before
//! the browser heard it (still dismissed), tab closes, and console messages and page errors,
//! which BrowserOS sends only outside hosted mode.
//!
//! Needs a throwaway BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005). Set
//! `BROWSERCLAW_HOSTED_MODE=1` when testing hosted mode; then the console test checks that
//! nothing arrives. Run with `bun run test:browser -- cargo test -p neo-playwright --test
//! events_live -- --ignored`. Every test opens and closes its own tabs.
//! The hidden-iframe download regression has its own command: `bun run test:browser -- cargo
//! test -p neo-playwright --test events_live
//! tests_that_a_hidden_iframe_download_reaches_its_page_and_finishes -- --ignored --exact`.

#[path = "support/js_runner.rs"]
#[expect(
    dead_code,
    reason = "the shared runner also serves selector and detector tests"
)]
mod js_runner;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "the capture refusal and the call record serve pages_live, which includes this too"
)]
mod live_run;
#[path = "support/page_server.rs"]
mod page_server;

use std::future::{self, Future};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs, process, thread};

use browseros_cdp::{CdpClient, CdpError, CdpEvent};
use browseros_core::{CdpConnection, SessionId};
use futures_util::future::BoxFuture;
use js_runner::OpHost;
use live_run::{LiveRun, TestPage, TestResult, brand, test_page};
use neo_playwright::{ErrorCode, PlaywrightError};
use neo_trace::ErrorOwner;
use page_server::PageServer;
use serde_json::{Value, json};
use tokio::fs::{read_to_string, remove_file};
use tokio::runtime::Handle;
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

/// Linux's native thread identity correlates opt-in observations with its `/proc` task data.
fn native_thread_id() -> Option<u32> {
    fs::read_link("/proc/thread-self")
        .ok()
        .and_then(|path| path.file_name()?.to_str()?.parse().ok())
}

/// Opt-in phase receipts tie this test runtime to the browser and external CPU observations.
#[expect(
    clippy::print_stderr,
    reason = "opt-in live fixture diagnostic receipts"
)]
fn trace_console_phase(started: Instant, phase: &str) {
    if env::var_os("NEO_LIVE_DIAGNOSTICS").is_some() {
        eprintln!(
            "LIVE_CONSOLE {}",
            json!({ "phase": phase, "wallMs": now_ms().ok(),
                "elapsedMs": started.elapsed().as_millis(),
                "pid": process::id(), "tid": native_thread_id(),
                "thread": format!("{:?}", thread::current().id()),
                "runtime": format!("{:?}", Handle::current().runtime_flavor()) })
        );
    }
}

/// Libtest otherwise reports an unavailable console row's early Ok return as a pass.
#[expect(
    clippy::print_stderr,
    reason = "live runner counts explicit precondition returns"
)]
fn console_skipped_in_hosted_mode() -> bool {
    let hosted = env::var("BROWSERCLAW_HOSTED_MODE").is_ok_and(|mode| mode == "1");
    if hosted {
        eprintln!("LIVE_TEST_EARLY_RETURN console events are unavailable in hosted mode");
    }
    hosted
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

/// The first `framenavigated` on `page` recorded at or after `since` whose frame is at `url`.
/// Another frame's event (the new frame's first `about:blank`, say) is skipped by asking again
/// after its `seq`, as the prelude does for a predicate that rejects an event. Each ask waits
/// `wait_for`'s 5 s.
async fn navigated_to(run: &LiveRun, page: &TestPage, since: u64, url: &str) -> TestResult<Value> {
    let mut after: Option<u64> = None;
    loop {
        let mut input = json!({ "call": { "api": "page.waitForEvent", "timeoutMs": 5000 },
                                "event": "framenavigated", "since": since, "page": brand(page) });
        if let (Some(seq), Some(fields)) = (after, input.as_object_mut()) {
            fields.insert("after".to_owned(), json!(seq));
        }
        let event = run
            .invoke("events.wait", input)
            .await
            .map_err(|error| format!("no framenavigated to {url}: {error}"))?;
        if event.pointer("/frame/url") == Some(&json!(url)) {
            return Ok(event);
        }
        after = Some(
            event
                .get("seq")
                .and_then(Value::as_u64)
                .ok_or("a frame event has a seq")?,
        );
    }
}

/// A cross-site iframe reports its own navigations only from its own session, and only once
/// `Page` is on there. Core turns it on only on tabs it created (its file chooser interception),
/// so this runs on an adopted tab, one the owner or another session opened, where until now only
/// a frames read did (`frame_tree::oopif_tree`). The frame attaches after the tab loaded and is
/// never read, so its event arrives only because the watcher turns `Page` on as the frame's
/// session attaches. It navigates once its first document loaded, inside its own process: its
/// first commit can race the enable, and the watcher's seed read, not an event, covers that one.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_cross_site_iframe_reports_its_own_navigation_without_a_frames_read()
-> TestResult {
    let run = LiveRun::connect().await?;
    let server = PageServer::start().await?;
    let page = run.open_adopted(&server.url("/child.html")).await?;
    let result = async {
        // `localhost` on the server's port is another site than `127.0.0.1`
        // (page_server.rs), so this frame is out of process.
        let cross = |query: &str| {
            server
                .url(&format!("/child.html?{query}"))
                .replacen("127.0.0.1", "localhost", 1)
        };
        let first = cross("first");
        run.eval(
            &page,
            &format!(
                "new Promise((loaded) => {{ const frame = document.createElement('iframe'); \
                 frame.id = 'late'; frame.onload = () => loaded(true); frame.src = {}; \
                 document.body.append(frame); }})",
                json!(first)
            ),
        )
        .await?;
        // The watcher turns `Page` on as the frame's session attaches, on a task of its own, so
        // a navigation started at once can still beat it. The frame navigates up to three times:
        // once `Page` is on, the next navigation is heard; without the watcher's enable, none is.
        let mut heard = None;
        for attempt in 1..=3 {
            let next = cross(&format!("next{attempt}"));
            let since = now_ms()?;
            run.eval(
                &page,
                &format!("document.getElementById('late').src = {}", json!(next)),
            )
            .await?;
            if let Ok(event) = navigated_to(&run, &page, since, &next).await {
                heard = Some(event);
                break;
            }
        }
        let navigated =
            heard.ok_or("no framenavigated for any of three navigations of the frame")?;
        assert!(
            navigated
                .pointer("/frame/parentId")
                .is_some_and(|parent| !parent.is_null()),
            "the child frame navigated, not the tab: {navigated}"
        );
        Ok(())
    }
    .await;
    run.close_page(&page).await;
    result
}

/// A trusted click that starts once a concurrent wait is surely listening, as a script's
/// `const p = page.waitForEvent(..); await page.click(..)` would.
async fn click_soon(run: &LiveRun, page: &TestPage, selector: &str) -> TestResult {
    time::sleep(Duration::from_millis(50)).await;
    run.click(page, selector).await
}

/// The attachment's iframe exists only after the trusted click, so the watcher's initial
/// frame tree cannot supply its owner. A page waiter must receive it and its saved file.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_hidden_iframe_download_reaches_its_page_and_finishes() -> TestResult {
    with_page(|run, server, page| async move {
        let result = async {
            open(&run, &page, &server.url("/child.html")).await?;
            run.eval(
                &page,
                r"(() => {
                    const button = document.createElement('button');
                    button.id = 'download';
                    button.textContent = 'Download attachment';
                    button.addEventListener('click', event => {
                        window.downloadClickTrusted = event.isTrusted;
                        const frame = document.createElement('iframe');
                        frame.hidden = true;
                        frame.src = '/attachment.txt';
                        document.body.append(frame);
                    });
                    document.body.append(button);
                })()",
            )
            .await?;
            let since = now_ms()?;
            let (download, clicked) = tokio::join!(
                wait_for(&run, Some(&page), "download", since),
                click_soon(&run, &page, "#download")
            );
            clicked?;
            assert_eq!(
                run.eval(&page, "window.downloadClickTrusted").await?,
                json!(true),
                "the iframe must be created by a trusted click"
            );
            let download = download?;
            assert_eq!(download["event"], json!("download"));
            assert_eq!(download["page"], brand(&page), "{download}");
            assert_eq!(
                download["download"]["url"],
                json!(server.url("/attachment.txt"))
            );
            assert_eq!(
                download["download"]["suggestedFilename"],
                json!("iframe-attachment.txt")
            );
            let finished = run
                .invoke(
                    "download.path",
                    json!({ "call": { "timeoutMs": 5000 },
                        "download": download["download"]["id"] }),
                )
                .await?;
            assert_eq!(finished["failure"], Value::Null, "{finished}");
            let path = finished["path"]
                .as_str()
                .ok_or("a completed download returns its saved path")?;
            assert_eq!(
                Path::new(path).file_name().and_then(|name| name.to_str()),
                Some("iframe-attachment.txt"),
                "the completed file keeps the attachment's filename"
            );
            assert_eq!(read_to_string(path).await?, "iframe attachment\n");
            remove_file(path).await?;
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// A run that sees only its family's tabs (as neo-server binds a run to its task) still hears
/// every popup its page opens, noopener ones included: the engine says each popup's opener
/// (`OpEnv::popup_opened`) before it judges the popup, so a host that lets a popup follow its
/// opener shows it. Its own new tab is its own at once (`OpEnv::opened_tab`), and a tab opened
/// outside the run stays out of its listing.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_bound_run_hears_its_pages_popups_and_lists_only_its_family() -> TestResult {
    with_page(|run, server, page| async move {
        let result = async {
            open(&run, &page, &server.url("/popups.html")).await?;
            run.bind(&[page.tab]);
            for (kind, path) in [
                ("#tab", "/child.html?tab"),
                ("#noopener", "/child.html?noopener"),
                ("#blanknoopener", "/child.html?blank-noopener"),
            ] {
                let since = now_ms()?;
                let (popup, clicked) = tokio::join!(
                    wait_for(&run, Some(&page), "popup", since),
                    click_soon(&run, &page, kind)
                );
                clicked?;
                let popup = popup?;
                assert_eq!(
                    popup["popup"]["url"],
                    json!(server.url(path)),
                    "{kind}: {popup}"
                );
                run.close_page(&test_page(&popup["popup"]["page"])?).await;
            }
            // An opener outside the run's family reads as none.
            let since = now_ms()?;
            let (popup, clicked) = tokio::join!(
                wait_for(&run, Some(&page), "popup", since),
                click_soon(&run, &page, "#tab")
            );
            clicked?;
            let popup = test_page(&popup?["popup"]["page"])?;
            run.bind(&[popup.tab]);
            let opener = run
                .invoke("page.opener", json!({ "page": brand(&popup) }))
                .await;
            run.close_page(&popup).await;
            assert_eq!(
                opener?["opener"],
                Value::Null,
                "the opener is not the run's"
            );
            run.bind(&[page.tab]);
            // A tab opened outside the run never reaches its context's `page` wait.
            let since = now_ms()?;
            let (heard, outside) = tokio::join!(wait_for(&run, None, "page", since), async {
                time::sleep(Duration::from_millis(50)).await;
                run.open_adopted(&server.url("/child.html?unheard")).await
            });
            run.close_page(&outside?).await;
            assert!(
                heard.is_err(),
                "the wait heard a tab outside the run: {heard:?}"
            );
            let own = run.new_page().await?;
            let outside = run.open_adopted(&server.url("/child.html?outside")).await?;
            let listed = run.invoke("context.pages", json!({})).await;
            run.close_page(&own).await;
            run.close_page(&outside).await;
            let listed = listed?.to_string();
            for (tab, shown) in [(&page, true), (&own, true), (&outside, false)] {
                assert_eq!(
                    listed.contains(&format!("\"tab\":{}", tab.tab.0)),
                    shown,
                    "tab {} listed: {shown}: {listed}",
                    tab.tab.0
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
    with_page(|run, server, page| async move {
        let result = async {
            let since = now_ms()?;
            let (opened, other) = tokio::join!(wait_for(&run, None, "page", since), async {
                time::sleep(Duration::from_millis(50)).await;
                run.new_page().await
            });
            let other = other?;
            let mut opened = opened?;
            // A context reports every tab that opens: a fresh browser's own startup tab, and the
            // tabs and popups this suite's other tests open at the same time on the same browser.
            // Skip those, as a script's predicate would.
            for _ in 0..16 {
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

            // Lifecycle events are supported too: subscribe before navigating so this
            // checks the emitted load rather than replaying the document already loaded.
            let since = now_ms()?;
            let (loaded, navigating) =
                tokio::join!(wait_for(&run, Some(&page), "load", since), async {
                    time::sleep(Duration::from_millis(50)).await;
                    open(&run, &page, &server.url("/popups.html")).await
                });
            navigating?;
            let loaded = loaded?;
            assert_eq!(loaded["event"], json!("load"));
            assert_eq!(loaded["page"], brand(&page));
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
                json!({ "page": brand(&page), "event": "dialog", "enabled": true, "since": since }),
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
                json!({ "page": brand(&page), "event": "dialog", "enabled": false }),
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

            // An alert is core's: accepted at once, before any listener could see it.
            let started = Instant::now();
            run.click(&page, "#alert").await?;
            assert_eq!(run.eval(&page, "window.__answer").await?, json!("alerted"));
            assert!(
                started.elapsed() < Duration::from_millis(250),
                "core accepts alerts at once"
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

    fn events_broadcast(&self) -> u64 {
        CdpConnection::events_broadcast(&self.client)
    }

    fn events_counted(&self) -> (broadcast::Receiver<CdpEvent>, u64) {
        CdpConnection::events_counted(&self.client)
    }

    fn events_targeted_counted(&self, method: &str) -> (mpsc::UnboundedReceiver<CdpEvent>, u64) {
        CdpConnection::events_targeted_counted(&self.client, method)
    }

    fn events_targeted_delivered(&self, method: &str) -> u64 {
        CdpConnection::events_targeted_delivered(&self.client, method)
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
            json!({ "page": brand(page), "event": "dialog", "enabled": true, "since": now_ms()? }),
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

/// How late [`SlowWatcher`] hands the target watcher each event.
const WATCHER_LAG: Duration = Duration::from_millis(150);

/// The crate's CDP connection with the target watcher behind the wire: `events_counted`, which
/// only the watcher calls, gives a receiver a relay feeds each event [`WATCHER_LAG`] after it
/// arrived. Commands, replies and the client's counts pass unchanged, so an evaluate's reply
/// comes well before the watcher sees the console event the evaluate caused, and only the
/// fence (`Playwright::events_before_reply`) can hold the op for it.
struct SlowWatcher {
    client: CdpClient,
}

impl CdpConnection for SlowWatcher {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
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

    fn events_broadcast(&self) -> u64 {
        CdpConnection::events_broadcast(&self.client)
    }

    fn events_counted(&self) -> (broadcast::Receiver<CdpEvent>, u64) {
        let (mut wire, before) = CdpConnection::events_counted(&self.client);
        let (late_tx, late) = broadcast::channel(4096);
        let (stamped_tx, mut stamped) = mpsc::unbounded_channel();
        // Two stages, so each event is late by the same lag rather than by the queue before it.
        tokio::spawn(async move {
            while let Ok(event) = wire.recv().await {
                if stamped_tx.send((Instant::now(), event)).is_err() {
                    return;
                }
            }
        });
        tokio::spawn(async move {
            while let Some((arrived, event)) = stamped.recv().await {
                time::sleep_until((arrived + WATCHER_LAG).into()).await;
                if late_tx.send(event).is_err() {
                    return;
                }
            }
        });
        (late, before)
    }

    fn events_targeted_counted(&self, method: &str) -> (mpsc::UnboundedReceiver<CdpEvent>, u64) {
        CdpConnection::events_targeted_counted(&self.client, method)
    }

    fn events_targeted_delivered(&self, method: &str) -> u64 {
        CdpConnection::events_targeted_delivered(&self.client, method)
    }

    fn is_connected(&self) -> bool {
        CdpConnection::is_connected(&self.client)
    }

    fn connection_epoch(&self) -> u64 {
        CdpConnection::connection_epoch(&self.client)
    }
}

/// Upstream's listener has run for the message an evaluate logged before the evaluate resolves.
/// Neo's op answers only once the pump took that message (`Playwright::events_before_reply`):
/// the pump's reply completes no later than the evaluate's, though the watcher reads the
/// console event on a task of its own, here [`WATCHER_LAG`] behind the wire.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_an_op_answers_after_the_pump_took_the_console_message_it_caused() -> TestResult
{
    if console_skipped_in_hosted_mode() {
        return Ok(());
    }
    let run = LiveRun::connect_through(|client| {
        Arc::new(SlowWatcher { client }) as Arc<dyn CdpConnection>
    })
    .await?;
    let server = PageServer::start().await?;
    let page = run.new_page().await?;
    let result = Box::pin(ordered_rounds(&run, &server, &page)).await;
    run.close_page(&page).await;
    result
}

/// [`tests_that_an_op_answers_after_the_pump_took_the_console_message_it_caused`]'s body.
async fn ordered_rounds(run: &LiveRun, server: &PageServer, page: &TestPage) -> TestResult {
    {
        open(run, page, &server.url("/popups.html")).await?;
        run.invoke(
            "events.listen",
            json!({ "page": brand(page), "event": "console", "enabled": true, "since": now_ms()? }),
        )
        .await?;
        for round in 0..5 {
            let text = format!("ordered {round}");
            let pumped = async {
                // The pump has no deadline of its own; a message that never comes must fail the
                // round, not hang the suite.
                let reply = time::timeout(
                    Duration::from_secs(10),
                    run.invoke("events.next", json!({})),
                )
                .await
                .map_err(|_elapsed| format!("round {round}: no console message in 10 s"));
                (reply, Instant::now())
            };
            let evaluated = async {
                let expression = format!("() => console.log({text:?})");
                let reply = run
                    .invoke(
                        "page.evaluate",
                        json!({ "page": brand(page), "expression": expression, "isFunction": true,
                                "arg": { "value": r#"{"v":"undefined"}"#, "handles": [] } }),
                    )
                    .await;
                (reply, Instant::now())
            };
            let ((pumped, pumped_at), (evaluated, evaluated_at)) = tokio::join!(pumped, evaluated);
            evaluated?;
            let events = pumped??
                .get("events")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            assert!(
                events
                    .iter()
                    .any(|event| event.pointer("/message/text") == Some(&json!(text))),
                "the pump's reply carries the message: {events:?}"
            );
            assert!(
                pumped_at <= evaluated_at,
                "round {round}: the evaluate answered before the pump took its message"
            );
        }
        Ok(())
    }
}

/// Core attaches pages with `Runtime` off, since a site can time the inspector's console work
/// to tell it is being inspected. A run that listens for nothing sends no `Runtime.enable` and
/// no `Log.enable`; a console listener leases `Runtime` on (`browseros_core::runtime_lease`) and
/// `Log` after it (`events/log_lease.rs`), and the run's end switches `Log` off, then `Runtime`.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_runtime_is_on_only_while_a_run_listens_for_it() -> TestResult {
    with_page(|mut run, server, page| async move {
        let result = async {
            run.record_methods();
            open(&run, &page, &server.url("/popups.html")).await?;
            run.click(&page, "#log").await?;
            run.click(&page, "#refused").await?;
            let quiet = run.recorded_methods();
            run.record_methods();
            assert!(
                !quiet.iter().any(|method| method == "Runtime.enable" || method == "Log.enable"),
                "a run that listens for nothing enabled Runtime or Log: {quiet:?}"
            );
            run.invoke(
                "events.listen",
                json!({ "page": brand(&page), "event": "console", "enabled": true, "since": now_ms()? }),
            )
            .await?;
            let listening = run.recorded_methods();
            run.record_methods();
            let enables: Vec<&String> = listening
                .iter()
                .filter(|method| method.ends_with(".enable"))
                .collect();
            assert_eq!(
                enables,
                ["Runtime.enable", "Log.enable"],
                "the listener takes one lease of each, Runtime first: {listening:?}"
            );
            run.next_run();
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut ended = Vec::new();
            while !ended.iter().any(|method| method == "Runtime.disable") {
                assert!(
                    Instant::now() < deadline,
                    "the ended run left Runtime on: {ended:?}"
                );
                time::sleep(Duration::from_millis(50)).await;
                ended.extend(run.recorded_methods());
                run.record_methods();
            }
            let disables: Vec<&String> = ended
                .iter()
                .filter(|method| method.ends_with(".disable"))
                .collect();
            assert_eq!(
                disables,
                ["Log.disable", "Runtime.disable"],
                "the ended run switched Log off before Runtime: {ended:?}"
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
async fn tests_that_console_and_page_errors_reach_listeners_outside_hosted_mode() -> TestResult {
    let hosted = env::var("BROWSERCLAW_HOSTED_MODE").is_ok_and(|mode| mode == "1");
    with_page(|run, server, page| async move {
        let result = async {
            open(&run, &page, &server.url("/popups.html")).await?;
            for event in ["console", "pageerror"] {
                run.invoke(
                    "events.listen",
                    json!({ "page": brand(&page), "event": event, "enabled": true, "since": now_ms()? }),
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

/// Upstream emits `popup` for a popup its page closed in the same task, with a page already
/// closed. The tab is gone before any listing names it, so Neo hands it out by the native tab id
/// its target carried, and the context no longer lists it.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_popup_closed_at_once_still_reaches_its_waiter() -> TestResult {
    with_page(|run, server, page| async move {
        let result = async {
            open(&run, &page, &server.url("/popups.html")).await?;
            let since = now_ms()?;
            let (popup, clicked) = tokio::join!(
                wait_for(&run, Some(&page), "popup", since),
                click_soon(&run, &page, "#closed")
            );
            clicked?;
            let popup = popup?;
            assert_eq!(popup["event"], json!("popup"), "{popup}");
            assert_eq!(popup["page"], brand(&page), "on the opener's page");
            let tab = test_page(&popup["popup"]["page"])?;
            assert_ne!(brand(&tab), brand(&page), "the popup is a page of its own");
            let listed = run.invoke("context.pages", json!({})).await?;
            assert!(
                !listed["pages"]
                    .as_array()
                    .is_some_and(|pages| pages.iter().any(|entry| entry["page"] == brand(&tab))),
                "a closed popup is not one of the context's pages: {listed}"
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// What the browser itself logs (here a `fetch` to a port it refuses) reaches a console listener
/// as upstream's `_onLogEntryAdded` emits it: the entry's level as the type, its text, no
/// arguments. An entry logged before the listener asked is the replay `Log.enable` sends, and
/// is not the listener's.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_browser_log_entry_reaches_a_console_listener_as_a_message() -> TestResult {
    if console_skipped_in_hosted_mode() {
        // Hosted mode sends no console events, so a listener leases nothing there.
        return Ok(());
    }
    with_page(|run, server, page| async move {
        let result = async {
            open(&run, &page, &server.url("/popups.html")).await?;
            run.click(&page, "#refused").await?;
            time::sleep(Duration::from_millis(300)).await;
            run.invoke(
                "events.listen",
                json!({ "page": brand(&page), "event": "console", "enabled": true, "since": now_ms()? }),
            )
            .await?;
            run.click(&page, "#refused").await?;
            let mut seen = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                let Ok(pumped) = time::timeout(
                    Duration::from_millis(1000),
                    run.invoke("events.next", json!({})),
                )
                .await
                else {
                    continue;
                };
                seen.extend(pumped?["events"].as_array().cloned().unwrap_or_default());
            }
            let refused: Vec<&Value> = seen
                .iter()
                .filter(|event| {
                    event["event"] == "console"
                        && event["message"]["text"]
                            .as_str()
                            .is_some_and(|text| text.contains("ERR_UNSAFE_PORT"))
                })
                .collect();
            assert_eq!(
                refused.len(),
                1,
                "only the entry logged after the listener asked: {seen:?}"
            );
            let message = &refused[0]["message"];
            assert_eq!(message["type"], json!("error"), "{message}");
            assert_eq!(message["args"], json!([]), "a log entry has no arguments");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// A click whose handler logs, while a script holds a console listener and the page logs large
/// tables all the while, as the `DevTools` detector's page does: the click completes in its
/// timeout, and the listener hears the handler's message exactly once. Through the prelude, so
/// the pump delivers every console message to the script as a run's would. `console.table`'s
/// preview lists every column of every row; read as a value, each such call carried 1 MB to the
/// script, and an op that ends waits for the pump in flight, so the click timed out after its
/// action and the message came too late.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
#[expect(
    clippy::print_stderr,
    reason = "opt-in phase markers correlate a lost fixture connection with its wire record"
)]
async fn tests_that_a_click_that_logs_completes_under_a_console_listener_and_is_heard_once()
-> TestResult {
    if console_skipped_in_hosted_mode() {
        // Hosted mode sends no console events, so there is nothing to hear.
        return Ok(());
    }
    let started = Instant::now();
    let trace = |phase: &str| trace_console_phase(started, phase);
    trace("connect");
    let run = Arc::new(LiveRun::connect().await?);
    let server = PageServer::start().await?;
    let page = run.new_page().await?;
    // A delayed tick distinguishes an occupied test executor from a live reader that lost
    // its socket. Start at observer creation so earlier browser setup cannot count as a
    // scheduler stall. This adds no browser traffic and never changes an op's deadline.
    let scheduler_gap = Arc::new(AtomicU64::new(0));
    let last_tick = Arc::new(AtomicU64::new(
        u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    ));
    let monitor = env::var_os("NEO_LIVE_DIAGNOSTICS").map(|_| {
        let gap = Arc::clone(&scheduler_gap);
        let tick = Arc::clone(&last_tick);
        tokio::spawn(async move {
            loop {
                time::sleep(Duration::from_millis(100)).await;
                let now = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                let previous = tick.swap(now, Ordering::Relaxed);
                let elapsed = now.saturating_sub(previous);
                gap.fetch_max(elapsed, Ordering::Relaxed);
                if elapsed > 500 {
                    eprintln!(
                        "LIVE_EXECUTOR_GAP {}",
                        json!({ "wallMs": now_ms().ok(), "startElapsedMs": previous,
                            "endElapsedMs": now, "gapMs": elapsed,
                            "pid": process::id(), "tid": native_thread_id() })
                    );
                }
            }
        })
    });
    let mut phase = "navigate";
    let result: TestResult = async {
        trace(phase);
        open(&run, &page, &server.url("/popups.html")).await?;
        phase = "start_tables";
        trace(phase);
        run.click(&page, "#tables").await?;
        // Tables stored before the listener: its lease's `Runtime.enable` replays them too.
        time::sleep(Duration::from_millis(1000)).await;
        phase = "install_delivery_counter";
        trace(phase);
        run.eval(&page, "window.__landed = 0; document.querySelector('#log').addEventListener('click', () => window.__landed++)").await?;
        let host: Arc<dyn OpHost> = Arc::clone(&run) as Arc<dyn OpHost>;
        phase = "script";
        trace(phase);
        let heard = js_runner::run_script(
            &host,
            &json!({ "page": brand(&page) }),
            "const seen = []; page.on('console', (message) => seen.push(message.text())); \
             await page.locator('#log').click({ timeout: 5000 }); await page.waitForTimeout(2000); \
             return seen.filter((text) => text === 'clicked 7').length;",
        )
        .await?;
        trace("script_returned");
        assert_eq!(heard, json!(1), "the handler's message, exactly once");
        phase = "read_delivery_counter";
        trace(phase);
        assert_eq!(run.eval(&page, "window.__landed").await?, json!(1), "the console-listener click landed exactly once");
        Ok(())
    }
    .await;
    // Include a final occupied interval even if the observer has not been polled again
    // before the operation reports its error. Snapshot before cleanup so cleanup latency
    // cannot be mistaken for the scheduling gap that preceded the original failure.
    let end = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let previous = last_tick.load(Ordering::Relaxed);
    let final_gap = end.saturating_sub(previous);
    scheduler_gap.fetch_max(final_gap, Ordering::Relaxed);
    if monitor.is_some() && final_gap > 500 {
        eprintln!(
            "LIVE_EXECUTOR_GAP {}",
            json!({ "wallMs": now_ms().ok(), "startElapsedMs": previous,
                "endElapsedMs": end, "gapMs": final_gap,
                "pid": process::id(), "tid": native_thread_id(), "phase": phase })
        );
    }
    let observed_gap = monitor.as_ref().map_or_else(
        || "not recorded".to_owned(),
        |_| format!("{} ms", scheduler_gap.load(Ordering::Relaxed)),
    );
    trace("cleanup");
    run.close_page(&page).await;
    if let Some(monitor) = monitor {
        monitor.abort();
    }
    result.map_err(|error| {
        format!(
            "console click failed during {phase}; largest executor tick gap {observed_gap}: {error}"
        )
        .into()
    })
}

/// What one timed-out `waitForFunction` left behind, as the WA-005 row reads it.
struct TimedOutWait {
    /// The wait's error code; `None` when it answered.
    code: Option<ErrorCode>,
    /// How many polls ran, by the predicate's own page stamps.
    polls: usize,
    /// Polls whose page stamp is later than the reply's wall clock.
    ran_after: usize,
    /// The poll messages the pump took before the reply, in the order it took them.
    heard_before: Vec<String>,
    /// How many poll messages the pump took only after the reply.
    heard_after: usize,
}

/// One `page.waitForFunction` that times out while its predicate logs at every 1 ms poll
/// (upstream's "should avoid side effects after timeout", WA-005), with a console listener on.
/// The pump reads until a marker logged 2 s after the reply, so every poll's message is counted
/// on one side of the reply or the other.
async fn timed_out_wait(run: &LiveRun, page: &TestPage) -> TestResult<TimedOutWait> {
    let evaluate = |expression: &str| {
        json!({ "page": brand(page), "expression": expression, "isFunction": true,
                "arg": { "value": r#"{"v":"undefined"}"#, "handles": [] } })
    };
    run.invoke(
        "page.evaluate",
        evaluate("() => { window.__n = 0; window.__t = []; }"),
    )
    .await?;
    let pump = async {
        let mut pumped: Vec<(Instant, String)> = Vec::new();
        loop {
            let reply = time::timeout(
                Duration::from_secs(10),
                run.invoke("events.next", json!({})),
            )
            .await
            .map_err(|_elapsed| "no event in 10 s")??;
            let at = Instant::now();
            let mut ended = false;
            for event in reply
                .get("events")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
            {
                let text = event
                    .pointer("/message/text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                ended |= text == "end-marker";
                pumped.push((at, text));
            }
            if ended {
                return TestResult::Ok(pumped);
            }
        }
    };
    let waited = async {
        let input = json!({
            "call": { "timeoutMs": 1000 }, "page": brand(page), "isFunction": true,
            "expression": "() => { window.__n += 1; window.__t.push(Date.now()); \
                           console.log('poll ' + window.__n); }",
            "arg": { "value": r#"{"v":"undefined"}"#, "handles": [] }, "pollingInterval": 1,
        });
        let code = run
            .invoke("page.waitForFunction", input)
            .await
            .err()
            .and_then(|error| error.error_code());
        let (replied, replied_ms) = (Instant::now(), now_ms()?);
        // Upstream's test gives the page two seconds to log more.
        time::sleep(Duration::from_secs(2)).await;
        run.invoke("page.evaluate", evaluate("() => console.log('end-marker')"))
            .await?;
        TestResult::Ok((code, replied, replied_ms))
    };
    let (pumped, waited) = tokio::join!(pump, waited);
    let (pumped, (code, replied, replied_ms)) = (pumped?, waited?);
    let stamps = run
        .invoke(
            "page.evaluate",
            evaluate("() => JSON.stringify(window.__t)"),
        )
        .await?;
    let stamps: Vec<u64> = serde_json::from_str(stamps.as_str().unwrap_or("[]"))?;
    let (before, after): (Vec<_>, Vec<_>) = pumped
        .into_iter()
        .filter(|(_, text)| text.starts_with("poll "))
        .partition(|(at, _)| *at <= replied);
    Ok(TimedOutWait {
        code,
        polls: stamps.len(),
        ran_after: stamps.iter().filter(|stamp| **stamp > replied_ms).count(),
        heard_before: before.into_iter().map(|(_, text)| text).collect(),
        heard_after: after.len(),
    })
}

/// WA-005: a `waitForFunction` that times out has no side effect a listener sees after its error.
/// No poll runs after it (Neo releases the poller before the reply), and the console events of
/// the polls that ran before it reach the listener before it, as upstream's do: its timeout awaits
/// the poller's cancel, and protocol messages arrive in order. Here the watcher reads each event
/// [`WATCHER_LAG`] behind the wire, as a loaded runner can: the timed-out reply must still wait
/// for the events before it. Each round must time out with `WaitTimeout` and hear every poll
/// that ran, in order, so it cannot pass on a wait that failed early or never polled. Fails on main
/// every time: a timed-out op answered at once, and the last polls' events came after the error.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_timed_out_wait_for_function_logs_nothing_after_its_error() -> TestResult {
    if console_skipped_in_hosted_mode() {
        return Ok(());
    }
    let run = LiveRun::connect_through(|client| {
        Arc::new(SlowWatcher { client }) as Arc<dyn CdpConnection>
    })
    .await?;
    let server = PageServer::start().await?;
    let page = run.new_page().await?;
    let result = async {
        open(&run, &page, &server.url("/popups.html")).await?;
        run.invoke(
            "events.listen",
            json!({ "page": brand(&page), "event": "console", "enabled": true, "since": now_ms()? }),
        )
        .await?;
        for round in 0..3 {
            let wait = Box::pin(timed_out_wait(&run, &page)).await?;
            assert_eq!(
                wait.code,
                Some(ErrorCode::WaitTimeout),
                "round {round}: the wait timed out"
            );
            assert!(wait.polls > 1, "round {round}: the predicate polled ({} polls)", wait.polls);
            assert_eq!(wait.ran_after, 0, "round {round}: no poll ran after the error");
            assert_eq!(wait.heard_after, 0, "round {round}: no console event after the error");
            let every_poll: Vec<String> = (1..=wait.polls).map(|n| format!("poll {n}")).collect();
            assert!(
                wait.heard_before == every_poll,
                "round {round}: the listener heard every poll, in order, before the error \
                 ({} heard, {} ran)",
                wait.heard_before.len(),
                wait.polls
            );
        }
        TestResult::Ok(())
    }
    .await;
    run.close_page(&page).await;
    result
}
