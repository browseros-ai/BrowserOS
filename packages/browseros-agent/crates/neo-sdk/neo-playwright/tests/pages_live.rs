//! Page and context ops on a real browser, through `Playwright::invoke`: `goto` and its load
//! states, history steps, `waitForLoadState`, the frame tree with out-of-process frames,
//! captures without consulting the host, the context's tabs, `opener` and `close`.
//!
//! Needs a throwaway BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005), for example
//! `bun run test:browser -- cargo test -p neo-playwright --test pages_live -- --ignored`.
//! Every test opens its own tabs and closes them.

#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "the swept values serve page_world_live, and the recheck refusal, adopted tabs and \
              frame reads serve actions_live, which include this too"
)]
mod live_run;
#[path = "support/page_server.rs"]
mod page_server;

use std::future::Future;
use std::path::Path;
use std::time::{Duration, Instant};

use browseros_core::TabId;
use live_run::{EnvCall, LiveRun, TestPage, TestResult, brand, test_page};
use neo_playwright::{Answer, ErrorCode, PlaywrightError, TimedOut};
use neo_trace::ErrorOwner;
use page_server::{PageServer, SLOW};
use serde_json::{Value, json};
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

/// A host whose view is bound to a profile closes through core's own close, the one close its
/// root gate allows, and the browser's tab list still confirms it.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_view_bound_to_a_profile_closes_its_tab() -> TestResult {
    let run = LiveRun::connect_bound().await?;
    assert!(
        run.is_bound(),
        "BrowserOS binds the view to its Default profile"
    );
    let page = run.new_page().await?;
    let closed = time::timeout(
        BROWSER_ONLY,
        run.invoke("page.close", json!({ "page": brand(&page) })),
    )
    .await;
    let listed = listed_pages(&run).await?;
    if listed.iter().any(|(id, _)| *id == page) {
        run.close_page(&page).await;
    }
    closed.map_err(|_elapsed| "page.close waited")??;
    assert!(
        !listed.iter().any(|(id, _)| *id == page),
        "the tab is gone: {listed:?}"
    );
    Ok(())
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
