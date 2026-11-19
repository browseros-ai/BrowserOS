//! Page and context ops on a real browser, through `Playwright::invoke`: `goto` and its load
//! states, history steps, `waitForLoadState`, the frame tree with out-of-process frames,
//! captures without consulting the host, the context's tabs, `opener` and `close`.
//!
//! Needs a throwaway BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005), for example
//! `bun run test:browser -- cargo test -p neo-playwright --test pages_live -- --ignored`.
//! Every test opens its own tabs and closes them.

#[path = "support/js_runner.rs"]
#[expect(
    dead_code,
    reason = "this suite runs one script through the real prelude; the runner's gestures and \
              refusals serve prelude_live"
)]
mod js_runner;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "the swept values serve page_world_live, and the recheck refusal, adopted tabs and \
              frame reads serve actions_live, which include this too"
)]
mod live_run;
#[path = "support/page_server.rs"]
mod page_server;

use std::error::Error;
use std::fs;
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use browseros_cdp::{CdpClient, CdpEvent, ConnectOptions};
use browseros_core::{CdpConnection, ProtocolSession, SessionId, TabId};
use futures_util::future::BoxFuture;
use js_runner::OpHost;
use live_run::{EnvCall, LiveRun, TestPage, TestResult, brand, test_page};
use neo_playwright::{Answer, ErrorCode, PlaywrightError, TimedOut};
use neo_trace::ErrorOwner;
use page_server::{PageServer, SLOW};
use serde_json::{Value, json};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::TryRecvError;
use tokio::sync::{Notify, Semaphore};
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
    let (run, result) = body(run, server, page.clone()).await;
    run.close_page(&page).await;
    result
}

async fn goto(
    run: &LiveRun,
    page: &TestPage,
    url: &str,
    wait_until: &str,
) -> Result<Value, PlaywrightError> {
    run.invoke(
        "page.goto",
        json!({ "call": { "timeoutMs": 30_000 }, "page": brand(page), "url": url,
                "waitUntil": wait_until }),
    )
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_goto_returns_at_each_load_state() -> TestResult {
    with_page(|run, server, page| async move {
        let result = async {
            let slow = server.url("/slow.html");
            // (waitUntil, at least, under): the image holds `load` for SLOW, and Chromium's
            // network idle comes 500 ms after the last connection.
            let cases = [
                ("commit", Duration::ZERO, SLOW),
                ("domcontentloaded", Duration::ZERO, SLOW),
                ("load", SLOW, SLOW * 3),
                ("networkidle", SLOW + Duration::from_millis(400), SLOW * 4),
            ];
            for (wait_until, at_least, under) in cases {
                let started = Instant::now();
                let result = goto(&run, &page, &slow, wait_until).await?;
                let took = started.elapsed();
                assert!(
                    took >= at_least && took < under,
                    "waitUntil {wait_until} returned after {took:?}"
                );
                assert_eq!(
                    result["response"]["status"],
                    json!(200),
                    "{wait_until}: {result}"
                );
                assert_eq!(result["response"]["statusText"], json!("OK"));
                assert_eq!(result["response"]["ok"], json!(true));
                assert_eq!(result["response"]["url"], json!(slow));
                assert_eq!(result["response"]["headers"], json!({}));
                assert_eq!(result["url"], json!(slow), "the prelude's page.url()");
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
async fn tests_that_goto_reports_statuses_net_errors_timeouts_and_the_referer() -> TestResult {
    with_page(|run, server, page| async move {
        let result = async {
            let missing = goto(&run, &page, &server.url("/missing.html"), "load").await?;
            assert_eq!(missing["response"]["status"], json!(404));
            assert_eq!(missing["response"]["statusText"], json!("Not Found"));
            assert_eq!(missing["response"]["ok"], json!(false));

            run.clear_calls();
            let unknown_host = "http://nonexistent.invalid/";
            let error = goto(&run, &page, unknown_host, "load")
                .await
                .err()
                .ok_or("an unknown host must fail")?;
            assert_eq!(error.error_code(), Some(ErrorCode::NavigationFailed));
            assert_eq!(error.owner(), ErrorOwner::Website);
            assert!(
                error
                    .message()
                    .starts_with("page.goto: net::ERR_NAME_NOT_RESOLVED at http://nonexistent.invalid/"),
                "{error}"
            );
            assert_eq!(error.answer(), Answer::Definite, "Neo saw the navigation end");
            assert!(
                run.calls().contains(&EnvCall::MarkSent),
                "the navigation was sent, so a failure is not not_sent"
            );

            let slow = server.url("/slow.html");
            let error = run
                .invoke(
                    "page.goto",
                    json!({ "call": { "timeoutMs": 400 }, "page": brand(&page), "url": slow }),
                )
                .await
                .err()
                .ok_or("a 400 ms goto of a 1.5 s page must time out")?;
            assert_eq!(error.error_code(), Some(ErrorCode::WaitTimeout), "{error}");
            assert_eq!(error.timed_out(), TimedOut::Yes);
            assert_eq!(
                error.message(),
                format!(
                    "page.goto: Timeout 400ms exceeded.\nCall log:\n  - navigating to \"{slow}\", waiting until \"load\"\n"
                )
            );

            let echo = server.url("/echo.html");
            run.invoke(
                "page.goto",
                json!({ "page": brand(&page), "url": echo, "referer": "https://referrer.test/from" }),
            )
            .await?;
            let referer = run
                .eval(&page, "document.getElementById('referer').textContent")
                .await?;
            assert_eq!(referer, json!("https://referrer.test/from"));
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_history_steps_and_reload_wait_for_their_documents() -> TestResult {
    with_page(|run, server, page| async move {
        let result = async {
            run.clear_calls();
            let back = run
                .invoke("page.goBack", json!({ "page": brand(&page) }))
                .await?;
            assert_eq!(
                back["response"],
                Value::Null,
                "a fresh tab has nowhere to go"
            );
            assert!(
                !run.calls().contains(&EnvCall::MarkSent),
                "nothing was sent, so nothing is marked"
            );

            let first = server.url("/child.html?1");
            let second = server.url("/child.html?2");
            goto(&run, &page, &first, "load").await?;
            goto(&run, &page, &second, "load").await?;
            let back = run
                .invoke("page.goBack", json!({ "page": brand(&page) }))
                .await?;
            assert_eq!(back["url"], json!(first));
            assert_eq!(back["response"]["status"], json!(200));
            let forward = run
                .invoke("page.goForward", json!({ "page": brand(&page) }))
                .await?;
            assert_eq!(forward["url"], json!(second));
            let reload = run
                .invoke(
                    "page.reload",
                    json!({ "page": brand(&page), "waitUntil": "domcontentloaded" }),
                )
                .await?;
            assert_eq!(reload["url"], json!(second));
            assert_eq!(reload["response"]["status"], json!(200));

            run.eval(&page, "location.hash = 'x'; true").await?;
            let back = run
                .invoke("page.goBack", json!({ "page": brand(&page) }))
                .await?;
            assert_eq!(
                back["response"],
                Value::Null,
                "a same-document step has no response"
            );
            assert_eq!(back["url"], json!(second));
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_wait_for_load_state_waits_for_the_current_document() -> TestResult {
    with_page(|run, server, page| async move {
        let result = async {
            goto(&run, &page, &server.url("/slow.html"), "commit").await?;
            let state = run.eval(&page, "document.readyState").await?;
            assert_ne!(
                state,
                json!("complete"),
                "the image still holds the load event"
            );
            run.invoke(
                "page.waitForLoadState",
                json!({ "call": { "timeoutMs": 10_000 }, "page": brand(&page), "state": "load" }),
            )
            .await?;
            assert_eq!(
                run.eval(&page, "document.readyState").await?,
                json!("complete")
            );

            let started = Instant::now();
            run.invoke(
                "page.waitForLoadState",
                json!({ "page": brand(&page), "state": "domcontentloaded" }),
            )
            .await?;
            assert!(
                started.elapsed() < Duration::from_millis(500),
                "a state the document already reached returns at once"
            );

            goto(&run, &page, &server.url("/frames.html"), "load").await?;
            let frames = run
                .invoke("page.frames", json!({ "page": brand(&page) }))
                .await?;
            for name in ["same", "cross", "deep"] {
                let frame = frame_named(&frames, name)?;
                run.invoke(
                    "page.waitForLoadState",
                    json!({ "page": brand(&page), "frame": frame["id"], "state": "load" }),
                )
                .await?;
            }

            let error = run
                .invoke(
                    "page.waitForLoadState",
                    json!({ "call": { "timeoutMs": 300 }, "page": brand(&page),
                            "frame": "no-such-frame" }),
                )
                .await
                .err()
                .ok_or("an unknown frame must fail")?;
            assert_eq!(error.error_code(), Some(ErrorCode::TargetClosed), "{error}");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

fn frame_named<'a>(frames: &'a Value, name: &str) -> TestResult<&'a Value> {
    frames
        .get("frames")
        .and_then(Value::as_array)
        .and_then(|frames| {
            frames
                .iter()
                .find(|frame| frame.get("name").and_then(Value::as_str) == Some(name))
        })
        .ok_or_else(|| format!("no frame {name} in {frames}").into())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_frames_lists_out_of_process_frames_under_their_embedders() -> TestResult {
    with_page(|run, server, page| async move {
        let result = async {
            let url = server.url("/frames.html");
            goto(&run, &page, &url, "load").await?;
            let frames = run
                .invoke("page.frames", json!({ "page": brand(&page) }))
                .await?;
            let listed = frames["frames"].as_array().ok_or("a list")?;
            assert_eq!(listed.len(), 4, "{frames}");
            let main = &listed[0];
            assert_eq!(main["parentId"], Value::Null, "the main frame comes first");
            assert_eq!(main["url"], json!(url));
            let same = frame_named(&frames, "same")?;
            let cross = frame_named(&frames, "cross")?;
            let deep = frame_named(&frames, "deep")?;
            assert_eq!(same["parentId"], main["id"]);
            assert_eq!(same["url"], json!(server.url("/child.html")));
            assert_eq!(cross["parentId"], main["id"]);
            assert!(
                cross["url"]
                    .as_str()
                    .is_some_and(|url| url.starts_with("http://localhost:")),
                "{cross}"
            );
            assert_eq!(
                deep["parentId"], cross["id"],
                "a frame inside the out-of-process frame hangs under it"
            );
            assert_eq!(deep["url"], json!(server.url("/child.html?deep")));
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// Chromium executes the script before reporting `ERR_ABORTED`. The op must mark the send
/// and preserve the browser error: a failed navigation can still have changed the document.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_goto_executes_a_javascript_url_before_reporting_the_browser_error() -> TestResult
{
    with_page(|run, server, page| async move {
        let result = async {
            goto(&run, &page, &server.url("/child.html"), "load").await?;
            let frames = run
                .invoke("page.frames", json!({ "page": brand(&page) }))
                .await?;
            for frame in [Value::Null, frames["frames"][0]["id"].clone()] {
                for url in [
                    "javascript:void(document.title = 'ran')",
                    " JavaScript:void(document.title = 'ran')",
                    "java\tscript:void(document.title = 'ran')",
                ] {
                    run.eval(&page, "document.title = 'before'").await?;
                    run.clear_calls();
                    let error = run
                        .invoke(
                            "page.goto",
                            json!({
                                "page": brand(&page), "frame": frame, "url": url,
                            }),
                        )
                        .await
                        .err()
                        .ok_or("Chromium aborts after executing the script")?;
                    assert_eq!(
                        error.error_code(),
                        Some(ErrorCode::NavigationFailed),
                        "{error}"
                    );
                    assert!(
                        error.message().contains("net::ERR_ABORTED at javascript:"),
                        "{error}"
                    );
                    assert!(
                        run.calls().contains(&EnvCall::MarkSent),
                        "the script reached Chromium"
                    );
                    assert_eq!(run.eval(&page, "document.title").await?, json!("ran"));
                }
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
async fn tests_that_goto_loads_a_local_file() -> TestResult {
    with_page(|run, _server, page| async move {
        let result = async {
            let file =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/local-navigation.html");
            let url = url::Url::from_file_path(&file)
                .map_err(|()| "a local file URL")?
                .to_string();
            let frames = run
                .invoke("page.frames", json!({ "page": brand(&page) }))
                .await?;
            for frame in [Value::Null, frames["frames"][0]["id"].clone()] {
                let reply = run
                    .invoke(
                        "page.goto",
                        json!({
                            "page": brand(&page), "frame": frame, "url": url,
                        }),
                    )
                    .await?;
                assert_eq!(reply["url"], json!(url));
                assert_eq!(reply["response"]["status"], json!(200), "{reply}");
                assert_eq!(reply["response"]["statusText"], json!("OK"));
                assert_eq!(reply["response"]["ok"], json!(true));
                assert_eq!(
                    run.eval(&page, "document.title").await?,
                    json!("Local navigation")
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
async fn tests_that_captures_return_their_bytes() -> TestResult {
    with_page(|run, server, page| async move {
        let result = async {
            goto(&run, &page, &server.url("/child.html"), "load").await?;
            let png = run
                .invoke("page.screenshot", json!({ "page": brand(&page) }))
                .await?;
            assert_eq!(png["$neo"], json!("bytes"));
            assert!(base64_of(&png)?.starts_with("iVBORw0KGgo"), "PNG magic");
            let jpeg = run
                .invoke(
                    "page.screenshot",
                    json!({ "page": brand(&page), "type": "jpeg", "quality": 40, "fullPage": true }),
                )
                .await?;
            assert!(base64_of(&jpeg)?.starts_with("/9j/"), "JPEG magic");
            let clip = run
                .invoke(
                    "page.screenshot",
                    json!({ "page": brand(&page),
                            "clip": { "x": 0, "y": 0, "width": 40, "height": 30 } }),
                )
                .await?;
            assert!(base64_of(&clip)?.len() < base64_of(&png)?.len(), "a clip is smaller");
            let error = run
                .invoke("page.screenshot", json!({ "page": brand(&page), "quality": 40 }))
                .await
                .err()
                .ok_or("quality on a PNG must fail")?;
            assert_eq!(error.error_code(), Some(ErrorCode::InvalidInput));

            let pdf = run.invoke("page.pdf", json!({ "page": brand(&page),
                "width": "960px", "height": "10in",
                "margin": {"top":"96px", "right":"1cm", "bottom":"0px", "left":"1mm"},
            })).await?;
            assert!(base64_of(&pdf)?.starts_with("JVBERi0"), "%PDF- magic");

            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// The five public commands translate back to the unchanged probe action protocol.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_storage_commands_keep_their_public_results() -> TestResult {
    with_page(|run, server, page| async move {
        let result = async {
            goto(&run, &page, &server.url("/child.html"), "load").await?;
            for kind in ["local", "session"] {
                let area = json!({"page":brand(&page), "kind":kind});
                assert_eq!(
                    run.invoke("webStorage.clear", area.clone()).await?,
                    Value::Null
                );
                assert_eq!(
                    run.invoke("webStorage.items", area.clone()).await?,
                    json!([])
                );
                let mut key = area.clone();
                key["name"] = json!("key");
                assert_eq!(
                    run.invoke("webStorage.getItem", key.clone()).await?,
                    Value::Null
                );
                let mut set = key.clone();
                set["value"] = json!("value");
                assert_eq!(run.invoke("webStorage.setItem", set).await?, Value::Null);
                assert_eq!(
                    run.invoke("webStorage.getItem", key.clone()).await?,
                    json!("value")
                );
                assert_eq!(
                    run.invoke("webStorage.items", area.clone()).await?,
                    json!([{"name":"key","value":"value"}])
                );
                assert_eq!(
                    run.invoke("webStorage.removeItem", key.clone()).await?,
                    Value::Null
                );
                assert_eq!(run.invoke("webStorage.getItem", key).await?, Value::Null);
            }
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

fn base64_of(bytes: &Value) -> TestResult<&str> {
    Ok(bytes
        .get("base64")
        .and_then(Value::as_str)
        .ok_or("a base64 string")?)
}

/// Every tab handle in `context.pages()`.
async fn listed_pages(run: &LiveRun) -> TestResult<Vec<(TestPage, String)>> {
    let listing = run.invoke("context.pages", json!({})).await?;
    let entries = listing
        .get("pages")
        .and_then(Value::as_array)
        .ok_or("a list")?;
    let mut pages = Vec::new();
    for entry in entries {
        let page = test_page(entry.get("page").ok_or("an entry names its page")?)?;
        let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        pages.push((page, url.to_owned()));
    }
    Ok(pages)
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_context_lists_every_tab_and_forgets_a_closed_one() -> TestResult {
    with_page(|run, _server, page| async move {
        let result = async {
            let other = run.new_page().await?;
            let listed = listed_pages(&run).await?;
            assert!(listed.iter().any(|(id, _)| *id == page), "{listed:?}");
            assert!(
                listed
                    .iter()
                    .any(|(id, url)| *id == other && url == "about:blank")
            );
            run.invoke("page.close", json!({ "page": brand(&other) }))
                .await?;
            let listed = listed_pages(&run).await?;
            assert!(
                !listed.iter().any(|(id, _)| *id == other),
                "a closed tab is not listed: {listed:?}"
            );
            assert!(listed.iter().any(|(id, _)| *id == page));
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// How long the stuck tab's close and opener may take. Each is a few browser round trips; a
/// page command would wait for the CDP request timeout, 60 s.
const BROWSER_ONLY: Duration = Duration::from_secs(10);

/// While a committed page's navigation is in flight, Chromium holds every session's page
/// commands until it commits or fails (measured on BrowserOS: `Page.enable`, `DOM.enable`,
/// `Runtime.evaluate` and `Page.getFrameTree`, from a new session and the old one alike), so a
/// tab whose navigation never commits answers none. Closing it and naming its opener are the
/// browser's business: a Neo that never attached the tab, as after its own restart or a new
/// browser connection, does both at once, so a host can always clear such a tab through `run`.
/// That neither sends the page a command is `page_op.rs`'s unit test: here the fresh
/// connection's own watcher attaches to every tab in the background, other tests' too.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_tab_stuck_mid_navigation_closes_at_once() -> TestResult {
    with_page(|run, server, page| async move {
        let result = async {
            goto(&run, &page, &server.url("/child.html"), "load").await?;
            let stall = server.url("/stall.html");
            run.eval(
                &page,
                &format!("setTimeout(() => {{ location.href = {stall:?}; }})"),
            )
            .await?;
            stuck(&run, &page).await?;

            let fresh = LiveRun::connect().await?;
            let handle = json!({ "page": brand(&page) });
            let opener =
                time::timeout(BROWSER_ONLY, fresh.invoke("page.opener", handle.clone())).await;
            let closed = time::timeout(BROWSER_ONLY, fresh.invoke("page.close", handle)).await;
            let opener = opener.map_err(|_elapsed| "page.opener waited on the stuck tab")??;
            assert_eq!(opener["opener"], Value::Null, "Neo opened it, no page did");
            closed.map_err(|_elapsed| "page.close waited on the stuck tab")??;
            let listed = listed_pages(&fresh).await?;
            assert!(
                !listed.iter().any(|(id, _)| *id == page),
                "the stuck tab is gone: {listed:?}"
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// Upstream's default close runs no beforeunload: a page that asks to stay, after the user
/// interacted with it (Chromium asks only then), still closes, with no dialog left open.
/// Red until the default close stops going through core's `Browser.closeTab`, which runs
/// beforeunload (measured on BrowserOS; `Target.closeTarget` does not).
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_close_runs_no_beforeunload() -> TestResult {
    with_page(|run, server, page| async move {
        let result = async {
            goto(&run, &page, &server.url("/beforeunload.html"), "load").await?;
            run.click(&page, "body").await?;
            assert_eq!(
                run.eval(&page, "navigator.userActivation.hasBeenActive")
                    .await?,
                json!(true),
                "the page may ask to stay"
            );
            time::timeout(
                BROWSER_ONLY,
                run.invoke("page.close", json!({ "page": brand(&page) })),
            )
            .await
            .map_err(|_elapsed| "page.close waited on the page")??;
            let listed = listed_pages(&run).await?;
            assert!(
                !listed.iter().any(|(id, _)| *id == page),
                "the tab closed, beforeunload notwithstanding: {listed:?}"
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// Waits until the browser reports `page` loading, then checks that it answers no page command
/// for a second: the state a tab is in while its navigation waits on a server that never answers.
async fn stuck(run: &LiveRun, page: &TestPage) -> TestResult {
    for _ in 0..50 {
        let info = run
            .browser()
            .cdp("Browser.getTabInfo", json!({ "tabId": page.tab.0 }), None)
            .await?;
        if info.pointer("/tab/isLoading") == Some(&json!(true)) {
            break;
        }
        time::sleep(Duration::from_millis(100)).await;
    }
    match time::timeout(Duration::from_secs(1), run.eval(page, "1")).await {
        Ok(answer) => Err(format!("the tab still answers page commands: {answer:?}").into()),
        Err(_) => Ok(()),
    }
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_popup_names_its_opener() -> TestResult {
    with_page(|run, server, page| async move {
        let result = async {
            goto(&run, &page, &server.url("/popups.html"), "load").await?;
            run.click(&page, "#tab").await?;
            let popup_url = server.url("/child.html?tab");
            let mut popup = None;
            for _ in 0..30 {
                popup = listed_pages(&run)
                    .await?
                    .into_iter()
                    .find(|(_, url)| *url == popup_url)
                    .map(|(id, _)| id);
                if popup.is_some() {
                    break;
                }
                time::sleep(Duration::from_millis(100)).await;
            }
            let popup = popup.ok_or("the popup tab is listed")?;
            let opener = run
                .invoke("page.opener", json!({ "page": brand(&popup) }))
                .await;
            let own = run
                .invoke("page.opener", json!({ "page": brand(&page) }))
                .await;
            run.close_page(&popup).await;
            assert_eq!(opener?["opener"], brand(&page));
            assert_eq!(
                own?["opener"],
                Value::Null,
                "a tab opened by Neo has no opener"
            );
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// An unknown native tab is refused before any input is sent, after one listing.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_an_unknown_tab_is_refused_as_closed() -> TestResult {
    with_page(|run, _server, page| async move {
        let result = async {
            let stale = TestPage {
                tab: TabId(i64::MAX),
            };
            run.clear_calls();
            let error = run
                .invoke("page.frames", json!({ "page": brand(&stale) }))
                .await
                .err()
                .ok_or("a handle for another tab must fail")?;
            assert_eq!(error.error_code(), Some(ErrorCode::TargetClosed), "{error}");
            assert!(error.message().contains("tab was closed"), "{error}");
            assert!(run.calls().is_empty(), "refused before the host was asked");
            run.invoke("page.frames", json!({ "page": brand(&page) }))
                .await?;
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// A fresh SDK instance has no memory of the original core page ID. A serialized native
/// tab handle must still reach the same surviving browser tab.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn a_tab_handle_survives_a_playwright_restart() -> TestResult {
    with_page(|run, server, page| async move {
        let result = async {
            goto(&run, &page, &server.url("/child.html"), "load").await?;
            run.eval(&page, "document.title = 'surviving native tab'")
                .await?;
            let saved = brand(&page);
            assert_eq!(saved, json!({ "$neo": "page", "tab": page.tab.0 }));
            let restarted = LiveRun::connect().await?;
            let title = restarted
                .invoke("page.title", json!({ "page": saved }))
                .await?;
            assert_eq!(title, "surviving native tab");
            Ok(())
        }
        .await;
        (run, result)
    })
    .await
}

/// A fixture progress bound, independent of operation deadlines.
const CRASH_PROGRESS_WATCHDOG: Duration = Duration::from_secs(30);
const CRASH_COMMAND_REPLY_WAIT: Duration = Duration::from_secs(1);

/// Observes native crashes independently of the SDK watcher. Receiving this event is not a
/// watcher barrier: the test's first fresh-run read must still expose any watcher/core race.
#[expect(
    clippy::print_stderr,
    reason = "the native crash and command results diagnose a real renderer crash"
)]
async fn crash_renderer(
    run: &LiveRun,
    page: &TestPage,
    events: &mut Receiver<CdpEvent>,
    crashes: &mut Vec<Value>,
) -> TestResult<String> {
    let core_page = run.core_page(page).await?;
    let attached = run.browser().pages.get_session(core_page).await?;
    let target = attached.target_id;
    let observed = async {
        loop {
            let event = events.recv().await?;
            if event.method == "Target.targetCrashed" {
                let ours =
                    event.params.get("targetId").and_then(Value::as_str) == Some(target.as_str());
                crashes.push(event.params);
                if ours {
                    return Ok::<(), Box<dyn Error>>(());
                }
            }
        }
    };
    // Page.crash can lose its reply. A native event proves a crashed renderer even then.
    let (crash_event, crash_reply) = tokio::join!(
        time::timeout(CRASH_PROGRESS_WATCHDOG, observed),
        time::timeout(
            CRASH_COMMAND_REPLY_WAIT,
            attached.session.send_value("Page.crash", json!({}))
        ),
    );
    eprintln!(
        "CRASH_RECOVERY {}",
        json!({"stage": "native_crash", "target": target.as_str(), "nativeCrashes": crashes,
               "observation": format!("{crash_event:?}"), "reply": format!("{crash_reply:?}")})
    );
    crash_event??;
    Ok(target.as_str().to_owned())
}

/// These owned witnesses exercise the sites other parallel rows use. Read both the SDK title
/// and the renderer itself, so a surviving tab entry alone cannot pass the isolation check.
#[expect(
    clippy::print_stderr,
    reason = "witness results identify collateral renderer crashes before a later assertion"
)]
async fn check_crash_neighbours(
    run: &LiveRun,
    neighbours: &[(TestPage, &str)],
    stage: &str,
) -> TestResult {
    for (page, expected) in neighbours {
        let title = time::timeout(
            CRASH_PROGRESS_WATCHDOG,
            run.invoke(
                "page.title",
                json!({"page": brand(page), "call": {"timeoutMs": 5_000}}),
            ),
        )
        .await;
        let evaluated = time::timeout(
            CRASH_PROGRESS_WATCHDOG,
            run.eval(page, "[document.title, 6 * 7]"),
        )
        .await;
        eprintln!(
            "CRASH_RECOVERY {}",
            json!({"stage": stage, "tab": page.tab.0, "expectedTitle": expected,
                   "title": format!("{title:?}"), "evaluate": format!("{evaluated:?}")})
        );
        let title = title??;
        let evaluated = evaluated??;
        if title != json!(expected) || evaluated != json!([expected, 42]) {
            return Err(format!(
                "{stage}: neighbour {} changed: title={title}, evaluate={evaluated}",
                page.tab.0
            )
            .into());
        }
    }
    Ok(())
}

/// Keep every native crash, including targets owned by other rows. This drains observations
/// already delivered to the fixture; it sends no command and does not synchronize the watcher.
#[expect(
    clippy::print_stderr,
    reason = "all observed native target IDs distinguish collateral crashes from SDK errors"
)]
fn check_crash_scope(
    target: &str,
    events: &mut Receiver<CdpEvent>,
    crashes: &mut Vec<Value>,
    stage: &str,
) -> TestResult {
    let drained = loop {
        match events.try_recv() {
            Ok(event) if event.method == "Target.targetCrashed" => crashes.push(event.params),
            Ok(_) => {}
            Err(TryRecvError::Empty) => break Ok(()),
            Err(error) => break Err(error),
        }
    };
    eprintln!(
        "CRASH_RECOVERY {}",
        json!({"stage": stage, "target": target, "nativeCrashes": crashes,
               "eventsError": drained.as_ref().err().map(ToString::to_string)})
    );
    drained?;
    if crashes
        .iter()
        .any(|event| event.get("targetId").and_then(Value::as_str) != Some(target))
    {
        return Err(format!("{stage}: a renderer outside {target} crashed: {crashes:?}").into());
    }
    Ok(())
}

/// Follows the error's SDK recovery guidance and owns the replacement through cleanup.
async fn recover_crashed_page(run: &LiveRun, page: &TestPage, url: &str) -> TestResult {
    run.invoke("page.close", json!({"page": brand(page)}))
        .await?;
    let replacement = run.new_page().await?;
    let recovered = async {
        goto(run, &replacement, url, "load").await?;
        let title = run
            .invoke("page.title", json!({"page": brand(&replacement)}))
            .await?;
        if title != json!("child") {
            return Err(format!("the named recovery must leave a usable renderer: {title}").into());
        }
        Ok(())
    }
    .await;
    run.close_page(&replacement).await;
    recovered
}

/// The crash diagnostic must name a recovery a script can complete while neighbouring sites
/// survive. Crash a dedicated site, then follow the message through the SDK.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
#[expect(
    clippy::print_stderr,
    reason = "the first fresh-run read must retain its underlying error and protocol methods"
)]
async fn target_crashed_message_recovery_closes_and_opens_a_working_page() -> TestResult {
    with_page(|mut run, server, page| async move {
        let mut neighbours = Vec::new();
        let result = async {
            run.bind(&[page.tab]);
            for (url, title) in [
                (server.url("/child.html"), "child"),
                ("about:blank".to_owned(), ""),
            ] {
                let neighbour = run.new_page().await?;
                neighbours.push((neighbour.clone(), title));
                goto(&run, &neighbour, &url, "load").await?;
            }
            check_crash_neighbours(&run, &neighbours, "before_crash").await?;
            // Renderer ownership follows a site, not a tab or an ephemeral server port.
            // With desktop site isolation, this stable fixture URL gives Page.crash its own
            // renderer, apart from the parallel rows' 127.0.0.1 and blank/data documents.
            let crash_url = server
                .url("/child.html")
                .replacen("127.0.0.1", "crash-recovery.localhost", 1);
            goto(&run, &page, &crash_url, "load").await?;
            let mut events = run.browser().cdp_events();
            let mut crashes = Vec::new();
            let target = crash_renderer(&run, &page, &mut events, &mut crashes).await?;
            run.next_run();
            run.record_methods();
            let failed = run
                .invoke(
                    "page.title",
                    json!({"page": brand(&page), "call":{"timeoutMs": 5_000}}),
                )
                .await;
            let methods = run.recorded_methods();
            eprintln!(
                "CRASH_RECOVERY {}",
                json!({"stage": "fresh_run_read", "target": target, "nativeCrashes": crashes,
                       "result": format!("{failed:?}"), "error": failed.as_ref().err().map(ToString::to_string),
                       "methods": methods})
            );
            let isolated =
                check_crash_scope(&target, &mut events, &mut crashes, "after_fresh_run_read");
            let surviving = check_crash_neighbours(&run, &neighbours, "after_crash").await;
            check_crash_scope(&target, &mut events, &mut crashes, "after_crash")?;
            isolated?;
            surviving?;
            let error = failed
                .err()
                .ok_or("a crashed renderer must fail the read")?;
            if error.error_code() != Some(ErrorCode::TargetCrashed) {
                return Err(format!("expected TargetCrashed: {error}; {error:?}").into());
            }
            let message = error.to_string();
            if !message.contains("page.close()")
                || !message.contains("context.newPage()")
                || message.contains("reload")
            {
                return Err(format!("the script needs executable recovery guidance: {message}").into());
            }
            recover_crashed_page(&run, &page, &crash_url).await?;
            let surviving = check_crash_neighbours(&run, &neighbours, "after_recovery").await;
            check_crash_scope(&target, &mut events, &mut crashes, "after_recovery")?;
            surviving
        }
        .await;
        // Ordinary assertion failures return errors above so all owned witnesses close too.
        for (neighbour, _) in &neighbours {
            run.close_page(neighbour).await;
        }
        (run, result)
    })
    .await
}

/// The longest a run of `ariaSnapshot()` and one click on a healthy tab may take beside tabs whose
/// renderer is busy, in the best of [`BESIDE_BUSY_TABS_TRIES`] tries. The wait it guards is
/// deterministic: a run that waits for a busy tab's frame tree pays the native read's whole
/// budget (one second, `frame_tree.rs` `SNAPSHOT_BUDGET`), and with the start hold back a run
/// cannot go under 2.1 s, so every try fails and the best of three cannot hide it. The run itself
/// takes about 150 ms on the pool's debug build (main took 5.3 s), plus the start's one grace for
/// listed tabs (`LISTED_START_GRACE_MS` in context.ts, 300 ms, one window for all of them): three
/// busy tabs paying it one after another would come to 1,050 ms and fail too. The rest, and the
/// tries, absorb the seconds-long CPU stalls of a debug test thread on a loaded machine.
const BESIDE_BUSY_TABS_BOUND: Duration = Duration::from_millis(900);

/// Tries per case before [`BESIDE_BUSY_TABS_BOUND`] fails it. The busy tabs stay busy for 20 s,
/// longer than three tries of a run that waits for them.
const BESIDE_BUSY_TABS_TRIES: usize = 3;

/// The script of [`tests_that_a_run_beside_busy_tabs_never_waits_for_their_frames`]: two of the
/// script's own operations on the healthy tab, each followed by the prelude's settle step.
const BESIDE_BUSY_TABS_SCRIPT: &str = "const text = await page.ariaSnapshot()\n\
    await page.getByRole('button', { name: 'Frame button' }).click()\n\
    return text";

/// A run never waits for the frame tree of a tab it does not use. The run's init lists every tab,
/// as a host whose runs see the whole browser does, so the prelude reads the busy tabs' trees too;
/// their renderer answers nothing, and each read waits out its budget. The script never names
/// them, so it pays nothing for them, whether one, two or three are busy.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
#[expect(
    clippy::print_stderr,
    reason = "each case prints its tries, the measure its bound is checked against"
)]
async fn tests_that_a_run_beside_busy_tabs_never_waits_for_their_frames() -> TestResult {
    let _alone = BUSY_TABS.acquire().await?;
    let run = Arc::new(LiveRun::connect().await?);
    let server = PageServer::start().await?;
    let healthy = run.new_page().await?;
    let mut busy = Vec::new();
    let result = async {
        goto(&run, &healthy, &server.url("/child.html"), "load").await?;
        // Another site than the healthy tab's, so Chromium never puts them in its renderer.
        let busy_url = server
            .url("/busy.html")
            .replacen("127.0.0.1", "localhost", 1);
        for _ in 0..3 {
            let page = run.new_page().await?;
            busy.push(page.clone());
            goto(&run, &page, &busy_url, "load").await?;
        }
        // Each page's loop starts 300 ms after it loaded.
        time::sleep(Duration::from_millis(600)).await;
        let host: Arc<dyn OpHost> = Arc::clone(&run) as Arc<dyn OpHost>;
        for count in 1..=busy.len() {
            let mut tabs = vec![healthy.tab];
            tabs.extend(busy.iter().take(count).map(|page| page.tab));
            let init = json!({ "page": brand(&healthy), "pages": run.page_facts(tabs).await });
            let mut tries = Vec::new();
            while tries.len() < BESIDE_BUSY_TABS_TRIES {
                let started = Instant::now();
                let text = js_runner::run_script(&host, &init, BESIDE_BUSY_TABS_SCRIPT).await?;
                let took = started.elapsed();
                tries.push(took);
                assert!(
                    text.as_str()
                        .is_some_and(|text| text.contains("Frame button")),
                    "the healthy tab's snapshot: {text}"
                );
                if took < BESIDE_BUSY_TABS_BOUND {
                    break;
                }
            }
            eprintln!("BESIDE_BUSY_TABS busy={count} tries={tries:?}");
            assert!(
                tries.iter().any(|took| *took < BESIDE_BUSY_TABS_BOUND),
                "beside {count} busy tabs, every try took {BESIDE_BUSY_TABS_BOUND:?} or more: \
                 {tries:?}"
            );
        }
        Ok::<(), Box<dyn Error>>(())
    }
    .await;
    let closing = Instant::now();
    for page in &busy {
        run.close_page(page).await;
    }
    eprintln!(
        "BESIDE_BUSY_TABS closed the busy tabs in {:?}",
        closing.elapsed()
    );
    run.close_page(&healthy).await;
    result
}

/// The two rows with busy tabs, one at a time. A run sees every tab of the browser, so the other
/// row's busy tabs would cost this row's run waits that are not the ones it measures.
static BUSY_TABS: Semaphore = Semaphore::const_new(1);

/// The longest the run may take from the moment another client opened a busy tab to the end of
/// its next five statements, in the best of [`BESIDE_BUSY_TABS_TRIES`] tries. A tab the
/// run first sees mid-run holds the script for its reads until one goes unanswered, so such a tab
/// may cost the run one wait in all: one native read's whole budget (one second,
/// `frame_tree.rs` `SNAPSHOT_BUDGET`). The bound is that one wait plus the room
/// [`BESIDE_BUSY_TABS_BOUND`] gives a run. A tab that held every statement would cost five or
/// more, so every try would fail.
const MID_RUN_BUSY_TAB_BOUND: Duration = Duration::from_millis(1_900);

/// The script of [`tests_that_a_busy_tab_another_client_opens_mid_run_costs_one_wait_at_most`]:
/// it waits for the tab the owner opens from its page, then runs five statements on its own tab,
/// and returns the URLs its wait saw and the busy tab's id. It waits on its page's `popup` event:
/// a context-level `page` wait makes the target watcher try to attach to every tab of the
/// browser on its own loop, which a busy tab stalls, so that row would measure the watcher.
const MID_RUN_BUSY_TAB_SCRIPT: &str = "const seen = []\n\
    let busy\n\
    try {\n\
        busy = await page.waitForEvent('popup', {\n\
            predicate: (other) => {\n\
                seen.push(other.url())\n\
                return other.url().includes('busy.html')\n\
            },\n\
            timeout: 15000,\n\
        })\n\
    } catch (error) {\n\
        return { seen, error: String(error) }\n\
    }\n\
    for (let i = 0; i < 5; i++) await page.title()\n\
    return { seen, tab: busy.tabId }";

/// Closes every tab the browser lists at `url`. The row's busy tabs are on a URL of their own (its
/// server's port), so this finds them whatever the script returned and touches no other row's tab.
async fn close_tabs_at(run: &LiveRun, url: &str) {
    let Ok(pages) = run.browser().pages.list().await else {
        return;
    };
    for info in pages.into_iter().filter(|info| info.url.starts_with(url)) {
        run.close_page(&TestPage { tab: info.tab_id }).await;
    }
}

/// The live run's ops, with how long each `page.frames` read took and on which tab, and a signal
/// once the script's `waitForEvent` went out (`events.wait`).
struct FrameReads {
    run: Arc<LiveRun>,
    reads: Mutex<Vec<(i64, Duration)>>,
    waiting: Notify,
}

impl OpHost for FrameReads {
    fn invoke<'a>(
        &'a self,
        name: &'a str,
        input: Value,
    ) -> BoxFuture<'a, Result<Value, PlaywrightError>> {
        Box::pin(async move {
            let tab = input
                .get("page")
                .and_then(|page| page.get("tab"))
                .and_then(Value::as_i64);
            if name == "events.wait" {
                self.waiting.notify_one();
            }
            let started = Instant::now();
            let answer = OpHost::invoke(&*self.run, name, input).await;
            if name == "page.frames"
                && let Some(tab) = tab
            {
                self.reads
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push((tab, started.elapsed()));
            }
            answer
        })
    }

    fn event_head(&self) -> u64 {
        OpHost::event_head(&*self.run)
    }

    fn page_facts(&self, tabs: Vec<TabId>) -> BoxFuture<'_, Vec<Value>> {
        OpHost::page_facts(&*self.run, tabs)
    }

    fn gesture(&self, page: TestPage, expression: String) -> BoxFuture<'_, Result<Value, String>> {
        OpHost::gesture(&*self.run, page, expression)
    }
}

/// A tab another client opens while a run is going, and that never answers, costs the run one
/// wait at most, never one per statement. Here the owner opens it from the run's page, as a
/// click on a link that opens a tab would. The run first sees it mid-run, so its first read holds
/// the script; that read goes unanswered, and from then on the tab is read only in the background.
/// The row also checks that the run did read the tab while it was busy, so it cannot pass by never
/// meeting the busy tab.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
#[expect(
    clippy::print_stderr,
    reason = "each try prints its time, the measure its bound is checked against"
)]
async fn tests_that_a_busy_tab_another_client_opens_mid_run_costs_one_wait_at_most() -> TestResult {
    let _alone = BUSY_TABS.acquire().await?;
    let run = Arc::new(LiveRun::connect().await?);
    let server = PageServer::start().await?;
    let healthy = run.new_page().await?;
    let result = async {
        goto(&run, &healthy, &server.url("/child.html"), "load").await?;
        let busy_url = server
            .url("/busy.html?at=now")
            .replacen("127.0.0.1", "localhost", 1);
        let init =
            json!({ "page": brand(&healthy), "pages": run.page_facts(vec![healthy.tab]).await });
        let mut tries = Vec::new();
        while tries.len() < BESIDE_BUSY_TABS_TRIES {
            let recorder = Arc::new(FrameReads {
                run: Arc::clone(&run),
                reads: Mutex::new(Vec::new()),
                waiting: Notify::new(),
            });
            let host: Arc<dyn OpHost> = Arc::clone(&recorder) as Arc<dyn OpHost>;
            let script = async {
                let outcome = js_runner::run_script(&host, &init, MID_RUN_BUSY_TAB_SCRIPT).await;
                (outcome, Instant::now())
            };
            // The owner opens the tab once the script's `waitForEvent` went out, from the run's
            // page with a user gesture: nothing of the run's opened it. Opened sooner, its event
            // could come before the wait's start and never reach the script.
            let open = async {
                let _waiting =
                    time::timeout(Duration::from_secs(10), recorder.waiting.notified()).await;
                let opened = run
                    .eval(&healthy, &format!("window.open({busy_url:?}); ''"))
                    .await;
                (opened, Instant::now())
            };
            let ((outcome, ended), (opened, opened_at)) = tokio::join!(script, open);
            // At once and whatever the try's outcome, so its loop does not load the machine for
            // the next try or other rows, and a failed try leaves no busy tab behind.
            close_tabs_at(&run, &busy_url).await;
            opened?;
            let outcome = outcome?;
            let took = ended.saturating_duration_since(opened_at);
            let reads = recorder
                .reads
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            let unanswered = reads
                .iter()
                .filter(|(tab, read)| *tab != healthy.tab.0 && *read >= Duration::from_millis(900))
                .count();
            eprintln!("MID_RUN_BUSY_TAB took={took:?} reads={reads:?} script={outcome}");
            assert!(
                outcome.get("error").is_none(),
                "the script never met the busy tab's page event: {outcome}"
            );
            assert!(
                unanswered >= 1,
                "the run never met the busy tab unanswered, so this try tested nothing: {reads:?}"
            );
            tries.push(took);
            if took < MID_RUN_BUSY_TAB_BOUND {
                break;
            }
        }
        assert!(
            tries.iter().any(|took| *took < MID_RUN_BUSY_TAB_BOUND),
            "every try took {MID_RUN_BUSY_TAB_BOUND:?} or more: {tries:?}"
        );
        Ok::<(), Box<dyn Error>>(())
    }
    .await;
    run.close_page(&healthy).await;
    result
}

/// The longest a run that listens on its context may take beside a tab whose renderer is busy, in
/// the best of [`BESIDE_BUSY_TABS_TRIES`] tries: three new tabs, each navigated while the context
/// waits for its commit, and a read of the run's own page after each (thirteen replies). With a
/// context-level listener the target watcher attaches core's page session to every untouched tab.
/// With that attach awaited on its loop, each pass over the busy tab holds the loop for the
/// attach's whole bound (one second, `CONTEXT_ATTACH_WAIT` in `target_watcher.rs`); a pass runs on
/// every tick and every new tab, so the loop is nearly always in one, and each reply waits for it
/// (up to two seconds, `SYNCHRONIZE_BOUND`): about 1.5 s a reply, near 20 s a try (read from code,
/// not measured). The run itself takes about a second on a debug build (estimated); the rest, and
/// the tries, absorb the 2 to 3 s stalls of a debug test thread on a loaded machine.
const CONTEXT_LISTENER_BESIDE_BUSY_TAB_BOUND: Duration = Duration::from_millis(4_000);

/// How long a probe of the busy tab's renderer waits before it counts the tab as busy: a healthy
/// renderer answers `1 + 1` in milliseconds, a loaded one well within this.
const BUSY_PROBE_WAIT: Duration = Duration::from_secs(1);

/// The script of [`tests_that_a_context_listener_beside_a_busy_tab_never_waits_for_it`], over
/// `url`: a context-level `framenavigated` listener, then three new tabs, each navigated while the
/// context waits for its commit, and a read of the run's own page after each. It returns the new
/// tabs' ids and the commits the listener heard.
fn context_listener_script(url: &str) -> String {
    format!(
        "const heard = []\n\
         context.on('framenavigated', (frame) => heard.push(frame.url()))\n\
         const opened = []\n\
         try {{\n\
             for (let i = 0; i < 3; i++) {{\n\
                 const other = await context.newPage()\n\
                 opened.push(other.tabId)\n\
                 const target = {url:?} + '?n=' + i\n\
                 const committed = context.waitForEvent('framenavigated', {{\n\
                     predicate: (frame) => frame.url() === target,\n\
                     timeout: 15000,\n\
                 }})\n\
                 await other.goto(target)\n\
                 await committed\n\
                 await page.title()\n\
             }}\n\
         }} catch (error) {{\n\
             return {{ opened, heard, error: String(error) }}\n\
         }}\n\
         return {{ opened, heard }}"
    )
}

/// Opens `url` in a background tab as another client does, over a connection of the test's own,
/// and returns its target id once its renderer stopped answering. The probe's session is on that
/// other connection, which the run's watcher never hears of, so neither core nor the watcher has
/// attached a page session to the tab when this returns: only the watcher's own observer, which
/// it attached while the tab still answered (`busy.html` starts its loop 300 ms after it loads).
async fn open_busy_tab(url: &str) -> TestResult<String> {
    let client = Arc::new(CdpClient::connect(ConnectOptions::new(live_run::cdp_port())).await?);
    let transport: Arc<dyn CdpConnection> = Arc::<CdpClient>::clone(&client);
    let opened: TestResult<String> = async {
        let created = transport
            .send(
                "Target.createTarget",
                json!({ "url": url, "background": true }),
                None,
            )
            .await?;
        let target = created
            .get("targetId")
            .and_then(Value::as_str)
            .ok_or("no target id")?
            .to_owned();
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
            .ok_or("no session id")?
            .to_owned();
        let probe =
            ProtocolSession::for_session(Arc::clone(&transport), SessionId::from(session.as_str()));
        for _ in 0..100 {
            let answer = time::timeout(
                BUSY_PROBE_WAIT,
                probe.send_value("Runtime.evaluate", json!({ "expression": "1 + 1" })),
            )
            .await;
            if answer.is_err() {
                return Ok(target);
            }
            time::sleep(Duration::from_millis(50)).await;
        }
        Err(format!("{url} kept answering").into())
    }
    .await;
    // Dropping CdpClient does not stop its reader tasks; the probe's session goes with it.
    client.disconnect().await;
    opened
}

/// A run that listens on its context is not held by a busy tab it never uses, however many tabs
/// open meanwhile: the target watcher attaches core's page session to the busy tab off its loop,
/// so the run's replies and its events keep their pace. The busy tab is another client's, opened
/// before the run and busy before the run's listener goes out, so every attach of it meets a
/// renderer that does not answer. The run sees only its own tabs, as a run neo-server binds to
/// its task does: the listener's own setup attaches each tab the run sees, a wait of the op's that
/// this row does not measure.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
#[expect(
    clippy::print_stderr,
    reason = "each try prints its time, the measure its bound is checked against"
)]
async fn tests_that_a_context_listener_beside_a_busy_tab_never_waits_for_it() -> TestResult {
    let _alone = BUSY_TABS.acquire().await?;
    let mut run = Arc::new(LiveRun::connect().await?);
    let server = PageServer::start().await?;
    let healthy = run.new_page().await?;
    let result = async {
        goto(&run, &healthy, &server.url("/child.html"), "load").await?;
        run.bind(&[healthy.tab]);
        // Another site than the healthy tab's, so Chromium never puts them in one renderer.
        let busy_url = server
            .url("/busy.html")
            .replacen("127.0.0.1", "localhost", 1);
        let script = context_listener_script(&server.url("/child.html"));
        let init =
            json!({ "page": brand(&healthy), "pages": run.page_facts(vec![healthy.tab]).await });
        let mut tries = Vec::new();
        while tries.len() < BESIDE_BUSY_TABS_TRIES {
            let busy = open_busy_tab(&busy_url).await?;
            let host: Arc<dyn OpHost> = Arc::clone(&run) as Arc<dyn OpHost>;
            let started = Instant::now();
            let outcome = js_runner::run_script(&host, &init, &script).await;
            let took = started.elapsed();
            drop(host);
            let _closed = run
                .browser()
                .cdp("Target.closeTarget", json!({ "targetId": busy }), None)
                .await;
            let opened = outcome
                .as_ref()
                .ok()
                .and_then(|outcome| outcome.get("opened"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for tab in opened.iter().filter_map(Value::as_i64) {
                run.close_page(&TestPage { tab: TabId(tab) }).await;
            }
            // A new run for the next try: the script's listener, left on, would have the watcher
            // attach the next busy tab at once, before it is busy.
            Arc::get_mut(&mut run)
                .ok_or("the run is still shared")?
                .next_run();
            let outcome = outcome?;
            eprintln!("CONTEXT_LISTENER_BESIDE_BUSY_TAB took={took:?} script={outcome}");
            assert!(
                outcome.get("error").is_none() && opened.len() == 3,
                "the script did not open its three tabs and see each one's commit: {outcome}"
            );
            tries.push(took);
            if took < CONTEXT_LISTENER_BESIDE_BUSY_TAB_BOUND {
                break;
            }
        }
        assert!(
            tries
                .iter()
                .any(|took| *took < CONTEXT_LISTENER_BESIDE_BUSY_TAB_BOUND),
            "every try took {CONTEXT_LISTENER_BESIDE_BUSY_TAB_BOUND:?} or more: {tries:?}"
        );
        Ok::<(), Box<dyn Error>>(())
    }
    .await;
    run.close_page(&healthy).await;
    result
}

/// How long the run's watcher may take to show a tab another client opened: the tab's version
/// is in `frame_versions()` once the watcher attached its root observer and the observer's
/// auto-attach reported the tab's cross-site frame. That is the signal the row waits for, the
/// same on main and with every observer a lifecycle source. It covers the tab's load, the
/// watcher's next tick and the frame's attach; read from code, not measured.
const UNTOUCHED_TAB_OBSERVED_BOUND: Duration = Duration::from_secs(10);

/// How long a child frame's commit in a tab Neo never acted on may take to move the tab's frame
/// version: the child's navigation and commit, its `Page.frameNavigated` on the tab's lifecycle
/// source, the watcher's handling of it (behind a tick and a tree read in the tab's line at
/// most), and the version's write. Read from code, not measured; the row prints what it took.
const UNTOUCHED_CHILD_COMMIT_BOUND: Duration = Duration::from_secs(2);

/// A tab opened by a client of the test's own, over a connection of its own: neither the run's
/// core nor its watcher hears of that connection's session, so whatever the test does through it
/// is another client's doing, never the run's.
struct OutsideTab {
    client: Arc<CdpClient>,
    transport: Arc<dyn CdpConnection>,
    target: String,
    session: ProtocolSession,
}

impl OutsideTab {
    /// Opens `url` in a background tab and returns once it loaded. A setup that fails at any step
    /// closes the tab it created and the connection, so no tab or client outlives the row.
    async fn open(url: &str) -> TestResult<Self> {
        let client = Arc::new(CdpClient::connect(ConnectOptions::new(live_run::cdp_port())).await?);
        let transport: Arc<dyn CdpConnection> = Arc::<CdpClient>::clone(&client);
        let mut created = None;
        let opened: TestResult<Self> = async {
            let reply = transport
                .send(
                    "Target.createTarget",
                    json!({ "url": url, "background": true }),
                    None,
                )
                .await?;
            let target = reply
                .get("targetId")
                .and_then(Value::as_str)
                .ok_or("no target id")?
                .to_owned();
            created = Some(target.clone());
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
            let tab = Self {
                client: Arc::clone(&client),
                transport: Arc::clone(&transport),
                session: ProtocolSession::for_session(
                    Arc::clone(&transport),
                    SessionId::from(session),
                ),
                target,
            };
            for _ in 0..200 {
                if tab.eval("document.readyState").await? == json!("complete") {
                    return Ok(tab);
                }
                time::sleep(Duration::from_millis(50)).await;
            }
            Err(format!("{url} did not load within 10 s").into())
        }
        .await;
        if opened.is_err() {
            if let Some(target) = &created {
                let _closed = transport
                    .send("Target.closeTarget", json!({ "targetId": target }), None)
                    .await;
            }
            client.disconnect().await;
        }
        opened
    }

    /// Runs `expression` in the tab's main world, by value.
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

    /// The URL of the tab's child frame named `name`, read on this client's own session (its own
    /// `Page` on, which no session of the run's sees).
    async fn child_url(&self, name: &str) -> TestResult<String> {
        self.session.send_value("Page.enable", json!({})).await?;
        let tree = self
            .session
            .send_value("Page.getFrameTree", json!({}))
            .await?;
        let url = tree
            .pointer("/frameTree/childFrames")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|child| child.pointer("/frame/name").and_then(Value::as_str) == Some(name))
            .and_then(|child| child.pointer("/frame/url").and_then(Value::as_str))
            .ok_or_else(|| format!("no child frame named {name}"))?;
        Ok(url.to_owned())
    }

    /// Closes the tab, then the connection: dropping a `CdpClient` does not stop its tasks.
    async fn close(&self) {
        let _closed = self
            .transport
            .send(
                "Target.closeTarget",
                json!({ "targetId": self.target }),
                None,
            )
            .await;
        self.client.disconnect().await;
    }
}

/// A tab Neo never acted on reports its frames' commits. Another client opens the nested-frame
/// fixture over its own connection, and its children commit navigations while the run's core has
/// no session on the tab. Each commit moves the tab's frame version, which the production door
/// hands every reply so a run re-reads a listed tab's tree: for the same-site child, whose events
/// only a lifecycle source of the tab with `Page` on reports, and for the cross-site one, which
/// its own session reports. The tab stays untouched throughout: core never attached to it.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
#[expect(
    clippy::print_stderr,
    reason = "each child's commit prints how long the version took to move, the measure its \
              bound is checked against"
)]
async fn tests_that_a_child_commit_in_a_tab_neo_never_touched_moves_its_frame_version() -> TestResult
{
    let run = LiveRun::connect().await?;
    let server = PageServer::start().await?;
    let tab = OutsideTab::open(&server.url("/frames.html")).await?;
    let result = async {
        let info = run
            .browser()
            .pages
            .list()
            .await?
            .into_iter()
            .find(|info| info.target_id.as_str() == tab.target)
            .ok_or("the tab is not listed")?;
        let version = || {
            run.frame_versions()
                .into_iter()
                .find(|(listed, _)| *listed == info.tab_id)
                .map(|(_, version)| version)
        };
        let observing = Instant::now();
        while version().is_none() {
            if observing.elapsed() > UNTOUCHED_TAB_OBSERVED_BOUND {
                return Err(format!(
                    "the run's watcher did not show the tab within {UNTOUCHED_TAB_OBSERVED_BOUND:?}"
                )
                .into());
            }
            time::sleep(Duration::from_millis(50)).await;
        }
        let cross = server
            .url("/nested.html?moved")
            .replacen("127.0.0.1", "localhost", 1);
        for (frame, url) in [("same", server.url("/child.html?moved")), ("cross", cross)] {
            let before = version();
            tab.eval(&format!(
                "document.querySelector('iframe[name={frame}]').src = {url:?}; ''"
            ))
            .await?;
            let asked = Instant::now();
            while version() <= before {
                if asked.elapsed() > UNTOUCHED_CHILD_COMMIT_BOUND {
                    return Err(format!(
                        "the {frame} child's commit did not move the tab's frame version within \
                         {UNTOUCHED_CHILD_COMMIT_BOUND:?}"
                    )
                    .into());
                }
                time::sleep(Duration::from_millis(20)).await;
            }
            eprintln!(
                "UNTOUCHED_CHILD_COMMIT frame={frame} took={:?}",
                asked.elapsed()
            );
        }
        if run
            .browser()
            .pages
            .get_attached_session(info.page_id)
            .await
            .is_some()
        {
            return Err("the run's core attached to the tab".into());
        }
        Ok::<(), Box<dyn Error>>(())
    }
    .await;
    tab.close().await;
    result
}

/// How long a tab's frame version must stay unchanged to count as at rest after a page loaded:
/// longer than the gap between the watcher's handling of a loaded page's last frame events (its
/// observer's seed, its frames' attaches and commits, each behind a tick at most). Read from code,
/// not measured; the row prints how long the version took to settle.
const UNTOUCHED_VERSION_SETTLE: Duration = Duration::from_millis(1_000);

/// A tab another client opened on a `file:` page, which the door refuses, reports nothing of its
/// own frames: the watcher sends such a tab nothing of a page domain. Its cross-site child, on the
/// test server, still gives the tab a frame version, as the observer's auto-attach reports it
/// (the attach moves the version before any door check, and its frame session is never read);
/// then a same-process child's commit leaves that version where it was for the whole bound a
/// child's commit gets in an untouched tab. Once the same client navigates the tab to a page the
/// door allows, the watcher seeds its observer, and once the new page is loaded and its version
/// at rest, a child's commit moves the version. Core never attaches to the tab. The `file:`
/// fixture is three local files written to a fresh directory and opened by their path, as the
/// browser reads the host's own files. Every check returns an error, so the tab is always closed.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
#[expect(
    clippy::print_stderr,
    reason = "each step prints how long it waited or took, the measure its bound is checked \
              against"
)]
async fn tests_that_an_untouched_tab_on_a_file_page_reports_no_frames_until_it_leaves_it()
-> TestResult {
    let run = LiveRun::connect().await?;
    let server = PageServer::start().await?;
    let dir = tempfile::tempdir()?;
    let parent = file_parent(dir.path(), &server)?;
    let tab = OutsideTab::open(&parent).await?;
    let result = async {
        let info = run
            .browser()
            .pages
            .list()
            .await?
            .into_iter()
            .find(|info| info.target_id.as_str() == tab.target)
            .ok_or("the tab is not listed")?;
        let version = || {
            run.frame_versions()
                .into_iter()
                .find(|(listed, _)| *listed == info.tab_id)
                .map(|(_, version)| version)
        };

        // The cross-site child's attach gives the tab a version while its `Page` stays off.
        let observing = Instant::now();
        while version().is_none() {
            if observing.elapsed() > UNTOUCHED_TAB_OBSERVED_BOUND {
                return Err(format!(
                    "the file tab's cross-site child gave it no frame version within \
                     {UNTOUCHED_TAB_OBSERVED_BOUND:?}"
                )
                .into());
            }
            time::sleep(Duration::from_millis(50)).await;
        }
        eprintln!("UNTOUCHED_FILE_TAB step=observed took={:?}", observing.elapsed());
        let before = version();
        set_child_src(&tab, "same", "moved.html").await?;
        time::sleep(UNTOUCHED_CHILD_COMMIT_BOUND).await;
        let committed = tab.child_url("same").await?;
        if !committed.ends_with("/moved.html") {
            return Err(format!("the file page's child did not commit: {committed}").into());
        }
        if version() != before {
            return Err(format!(
                "a same-process child's commit on a file page moved the tab's frame version \
                 within {UNTOUCHED_CHILD_COMMIT_BOUND:?}: {before:?} to {:?}",
                version()
            )
            .into());
        }
        eprintln!(
            "UNTOUCHED_FILE_TAB step=file waited={UNTOUCHED_CHILD_COMMIT_BOUND:?} version={before:?}"
        );

        tab.session
            .send_value("Page.navigate", json!({ "url": server.url("/frames.html") }))
            .await?;
        let asked = Instant::now();
        loaded_over_http(&tab).await?;
        let rest = settled(&version, asked).await?;
        if rest <= before {
            return Err(format!(
                "the move to an allowed page left the tab's frame version at {before:?}"
            )
            .into());
        }
        eprintln!(
            "UNTOUCHED_FILE_TAB step=allowed settled={:?} version={rest:?}",
            asked.elapsed()
        );
        let child_src = server.url("/child.html?moved");
        let ChildCommit { committed, moved } =
            child_commit_moves(&tab, &child_src, &version, rest).await?;
        eprintln!("UNTOUCHED_FILE_TAB step=child committed={committed:?} moved={moved:?}");
        if run
            .browser()
            .pages
            .get_attached_session(info.page_id)
            .await
            .is_some()
        {
            return Err("the run's core attached to the tab".into());
        }
        Ok::<(), Box<dyn Error>>(())
    }
    .await;
    tab.close().await;
    result
}

/// Writes the `file:` fixture into `dir`: a parent with a same-process child named `same` (a local
/// file) and a cross-site one named `cross` (on `server`), and the local page the child moves to.
/// Returns the parent's `file://` URL.
fn file_parent(dir: &Path, server: &PageServer) -> TestResult<String> {
    fs::write(
        dir.join("parent.html"),
        format!(
            "<!doctype html><title>file parent</title>\
             <iframe name=\"same\" src=\"child.html\"></iframe>\
             <iframe name=\"cross\" src=\"{}\"></iframe>",
            server.url("/child.html")
        ),
    )?;
    for child in ["child.html", "moved.html"] {
        fs::write(
            dir.join(child),
            format!("<!doctype html><title>{child}</title>"),
        )?;
    }
    Ok(format!("file://{}", dir.join("parent.html").display()))
}

/// Waits until the tab shows the allowed page it was sent to, loaded, within
/// [`UNTOUCHED_TAB_OBSERVED_BOUND`]. The old document answers until the new one commits, and an
/// evaluate that meets the swap fails, so only the new page, complete, counts.
async fn loaded_over_http(tab: &OutsideTab) -> TestResult {
    let asked = Instant::now();
    loop {
        let state = tab
            .eval("[location.protocol, document.readyState].join(' ')")
            .await;
        if matches!(&state, Ok(state) if *state == json!("http: complete")) {
            return Ok(());
        }
        if asked.elapsed() > UNTOUCHED_TAB_OBSERVED_BOUND {
            return Err(format!(
                "the allowed page did not load within {UNTOUCHED_TAB_OBSERVED_BOUND:?}: {state:?}"
            )
            .into());
        }
        time::sleep(Duration::from_millis(50)).await;
    }
}

/// How long [`child_commit_moves`] took for each of its two waits, both from when it set the
/// child's `src`.
struct ChildCommit {
    /// Until the outside client's own tree read showed the child at its new URL.
    committed: Duration,
    /// Until the tab's frame version moved past its resting value.
    moved: Duration,
}

/// Points the tab's `same` child at `src` and waits, within [`UNTOUCHED_CHILD_COMMIT_BOUND`], for
/// the outside client's own tree read to show it committed, then for the tab's frame version to
/// move past `rest`.
async fn child_commit_moves(
    tab: &OutsideTab,
    src: &str,
    version: &impl Fn() -> Option<u64>,
    rest: Option<u64>,
) -> TestResult<ChildCommit> {
    set_child_src(tab, "same", src).await?;
    let asked = Instant::now();
    while tab.child_url("same").await? != src {
        if asked.elapsed() > UNTOUCHED_CHILD_COMMIT_BOUND {
            return Err(format!(
                "the allowed page's child did not commit within {UNTOUCHED_CHILD_COMMIT_BOUND:?}"
            )
            .into());
        }
        time::sleep(Duration::from_millis(20)).await;
    }
    let committed = asked.elapsed();
    while version() <= rest {
        if asked.elapsed() > UNTOUCHED_CHILD_COMMIT_BOUND {
            return Err(format!(
                "the child's commit on the allowed page did not move the tab's frame version \
                 within {UNTOUCHED_CHILD_COMMIT_BOUND:?}"
            )
            .into());
        }
        time::sleep(Duration::from_millis(20)).await;
    }
    Ok(ChildCommit {
        committed,
        moved: asked.elapsed(),
    })
}

/// Points the tab's child frame named `name` at `src`, from the tab's main world, and checks the
/// frame was there to take it: the page answers with the child's resolved `src`.
async fn set_child_src(tab: &OutsideTab, name: &str, src: &str) -> TestResult {
    let set = tab
        .eval(&format!(
            "(() => {{ const frame = document.querySelector('iframe[name={name}]'); \
             if (!frame) return null; frame.src = {src:?}; return frame.src; }})()"
        ))
        .await?;
    match set.as_str() {
        Some(resolved) if resolved.ends_with(src) => Ok(()),
        _ => Err(format!("the {name} child did not take {src}: {set}").into()),
    }
}

/// The tab's frame version once it stayed unchanged for [`UNTOUCHED_VERSION_SETTLE`], waiting at
/// most [`UNTOUCHED_TAB_OBSERVED_BOUND`] from `since`.
async fn settled(version: &impl Fn() -> Option<u64>, since: Instant) -> TestResult<Option<u64>> {
    let mut seen = version();
    let mut steady = Instant::now();
    while steady.elapsed() < UNTOUCHED_VERSION_SETTLE {
        if since.elapsed() > UNTOUCHED_TAB_OBSERVED_BOUND {
            return Err(format!(
                "the tab's frame version did not rest for {UNTOUCHED_VERSION_SETTLE:?} within \
                 {UNTOUCHED_TAB_OBSERVED_BOUND:?}"
            )
            .into());
        }
        time::sleep(Duration::from_millis(50)).await;
        let now = version();
        if now != seen {
            seen = now;
            steady = Instant::now();
        }
    }
    Ok(seen)
}

/// A run lists a listed tab's whole frame tree from its first statement, as a host's observer
/// reads it: `context.pages()` names another tab, and the script walks its frames before any
/// await, cross-site ones included. Nothing in the run touched that tab; the start waits for its
/// first read within the grace for listed tabs (`LISTED_START_GRACE_MS` in context.ts), which a
/// tab that answers needs only milliseconds of.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_listed_tab_lists_its_whole_frame_tree_from_the_first_statement() -> TestResult
{
    let run = Arc::new(LiveRun::connect().await?);
    let server = PageServer::start().await?;
    let own = run.new_page().await?;
    let other = run.new_page().await?;
    let result = async {
        goto(&run, &own, &server.url("/child.html"), "load").await?;
        goto(&run, &other, &server.url("/frames.html"), "load").await?;
        let init =
            json!({ "page": brand(&own), "pages": run.page_facts(vec![own.tab, other.tab]).await });
        let host: Arc<dyn OpHost> = Arc::clone(&run) as Arc<dyn OpHost>;
        let script = format!(
            "const p = context.pages().find((q) => q.tabId === {})\n\
             return p.frames().map((frame) => frame.name()).sort()",
            other.tab.0
        );
        let names = js_runner::run_script(&host, &init, &script).await?;
        assert_eq!(
            names,
            json!(["", "cross", "deep", "same"]),
            "the other tab's whole tree at the first statement"
        );
        Ok::<(), Box<dyn Error>>(())
    }
    .await;
    run.close_page(&other).await;
    run.close_page(&own).await;
    result
}
