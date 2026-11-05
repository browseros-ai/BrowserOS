//! Milestones M1 and M2 on a real browser: the built binary, launched the way Chromium launches
//! it, against a headless BrowserOS, driven over MCP by an rmcp client, and read back over the
//! host socket the way zuzu reads it, including a host's lock on a tab that is then closed from
//! outside Neo.
//!
//! ```text
//!  BrowserOS --headless --remote-debugging-port=<cdp>  ◀── CDP ──  browseros-claw-sdk
//!                                                                   ▲ POST /mcp: run
//!  this test ── launch JSON {ports: {server, cdp}} ─────────────────┤
//!            ── GET /v1/runs/{id} on CLAW_HOST_SOCKET, bearer token ┘
//!            ── after SIGTERM: $BROWSERCLAW_DIR/claw-sdk/trace.db
//! ```
//!
//! It needs BrowserOS, not stock Chrome: core lists and opens tabs with BrowserOS-only CDP
//! (`Browser.getTabs`, `Browser.createTab`). It runs only when `CLAW_SDK_E2E_BROWSER` names the
//! browser binary, e.g. `/Applications/BrowserOS.app/Contents/MacOS/BrowserOS`, and passes
//! without doing anything otherwise, so CI without a browser stays green.
//!
//! No test here has a CAPTCHA provider key, and none reaches a provider. One Neo runs without a
//! key, as a deployment without one does. The other gets a placeholder key file, which turns the
//! solver on, and a provider URL on a closed local port: its page holds no CAPTCHA, so the solve
//! ends at detection, which is the real crate on the real browser.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::error::Error;
use std::time::Duration;

use browseros_rig::{
    CaptchaKey, Client, HOST_TOKEN, PATIENCE, Rig, TestResult, browser_or_skip, get, outcome, run,
};
use browseros_trace::{Delivery, Effect, Outcome, RunId, RunStatus, TraceStore};
use serde_json::{Value, json};
use tokio::time;

/// The whole drive. A step that hangs fails the test here, so the guards still kill both
/// processes: a killed test process runs no `Drop`.
const TEST_BUDGET: Duration = Duration::from_secs(240);
/// A page whose `<title>` the tab strip shows once it loads.
const PAGE: &str = "data:text/html,<title>Neo e2e</title><p>claw-sdk end to end</p>";
const TITLE: &str = "Neo e2e";
/// The page a host locks, then the owner closes.
const LOCKED_PAGE: &str = "data:text/html,<title>Neo e2e lock</title><p>the owner's now</p>";
const LOCKED_TITLE: &str = "Neo e2e lock";

#[tokio::test(flavor = "multi_thread")]
async fn the_playwright_api_drives_a_real_browseros_page() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let mut rig = Rig::start(&browser, CaptchaKey::Absent)?;
    let failure = match time::timeout(TEST_BUDGET, drive(&mut rig)).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => format!("the drive did not finish within {TEST_BUDGET:?}"),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

async fn drive(rig: &mut Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let code = format!(
        r"
        const p = await context.newPage();
        await p.goto({PAGE:?});
        const t = await p.title();
        await p.close();
        return t;"
    );
    let meta = json!({ "requestId": "e2e-1", "taskId": "e2e" });

    let first = run(&client, &code, &meta).await?;
    assert_eq!(
        first.structured_content,
        Some(json!({ "value": TITLE })),
        "the tab strip's title of the page BrowserOS loaded: {first:?}"
    );
    let first_meta = outcome(&first)?;
    assert_eq!(first_meta.get("status"), Some(&json!("ok")), "{first_meta}");
    let id: RunId = first_meta
        .get("id")
        .and_then(Value::as_str)
        .ok_or("every result names its run")?
        .parse()?;

    let targets = get(rig.cdp_port(), "/json/list").await?;
    assert!(
        !targets.contains(TITLE),
        "page.close closed the tab in the browser: {targets}"
    );

    let repeat = run(&client, &code, &meta).await?;
    assert_eq!(
        outcome(&repeat)?.get("id").and_then(Value::as_str),
        Some(id.to_string().as_str()),
        "a repeated requestId replays the first run instead of running again"
    );
    assert_eq!(
        repeat.structured_content, first.structured_content,
        "with its value"
    );
    solve_captcha_without_a_key(&client).await?;
    let (locked_run, locked_tab) = a_hosts_lock_ends_when_its_tab_closes(rig, &client).await?;
    client.cancel().await?;
    read_back_over_the_host_socket(rig, id).await?;

    assert_eq!(rig.stop_neo().await?, Some(0), "SIGTERM is a normal stop");
    let store = TraceStore::open(&rig.trace_path()).await?;
    let detail = store.run(id).await?.ok_or("the run is in the trace")?;
    assert_eq!(detail.summary.status, RunStatus::Ok);
    assert_eq!(
        (
            detail.summary.request_id.map(|request| request.to_string()),
            detail.summary.task_id.map(|task| task.to_string()),
        ),
        (Some("e2e-1".to_owned()), Some("e2e".to_owned())),
        "the host's ids are recorded with the run"
    );
    let ops: Vec<_> = detail
        .operations
        .iter()
        .map(|op| (op.name.as_str(), op.effect, op.outcome, op.delivery))
        .collect();
    assert_eq!(
        ops,
        [
            (
                "context.newPage",
                Effect::Write,
                Some(Outcome::Ok),
                Some(Delivery::Done)
            ),
            (
                "page.goto",
                Effect::Write,
                Some(Outcome::Ok),
                Some(Delivery::Done)
            ),
            // Playwright's goto reads the frame tree it navigated, for `page.frames()`.
            (
                "page.frames",
                Effect::Read,
                Some(Outcome::Ok),
                Some(Delivery::NotSent)
            ),
            (
                "page.title",
                Effect::Read,
                Some(Outcome::Ok),
                Some(Delivery::NotSent)
            ),
            (
                "page.close",
                Effect::Write,
                Some(Outcome::Ok),
                Some(Delivery::Done)
            ),
        ],
        "five ops, once each (the replay ran nothing): writes done once sent, reads never sent"
    );
    the_load_is_an_op_on_the_new_tab(&store, locked_run, locked_tab).await
}

/// The trace of the run that opened the locked tab: `context.newPage()` on no tab, then the load
/// as `page.goto` on the new tab, where the door's rules and the trace see the tab.
async fn the_load_is_an_op_on_the_new_tab(store: &TraceStore, run: RunId, tab: i64) -> TestResult {
    let opened = store
        .run(run)
        .await?
        .ok_or("the run that opened the locked tab is in the trace")?;
    let ops: Vec<(&str, Option<i64>, bool)> = opened
        .operations
        .iter()
        .map(|op| {
            (
                op.name.as_str(),
                op.tab.map(|tab| tab.0),
                op.parent.is_some(),
            )
        })
        .collect();
    assert_eq!(
        ops,
        [
            ("context.newPage", None, false),
            ("page.goto", Some(tab), false),
            ("page.frames", Some(tab), false),
        ],
        "the load is the script's own op on the new tab, where the door's rules see it"
    );
    Ok(())
}

/// `neo.solveCaptcha` on a Neo with no provider key: the real prelude's `neo`, refused for the
/// host to fix, before anything reaches the page.
async fn solve_captcha_without_a_key(client: &Client) -> TestResult {
    let code = r"
        try {
          await neo.solveCaptcha(page);
          return 'solved without a key';
        } catch (e) {
          return { code: e.code, owner: e.owner, delivery: e.delivery };
        }";
    let result = run(client, code, &json!({ "taskId": "e2e-captcha-off" })).await?;
    assert_eq!(
        result.structured_content,
        Some(json!({ "value": {
            "code": "captcha_unavailable",
            "owner": "host",
            "delivery": "not_sent",
        } })),
        "{result:?}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn solve_captcha_detects_a_clear_page_on_a_real_browseros() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let rig = Rig::start(&browser, CaptchaKey::Placeholder)?;
    let failure = match time::timeout(TEST_BUDGET, solve_on_a_clear_page(&rig)).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => format!("the drive did not finish within {TEST_BUDGET:?}"),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

/// One solve on a page with no CAPTCHA: the solver's detection runs through claw-sdk's door on
/// the real browser and answers `already_clear` as a value, without a provider.
async fn solve_on_a_clear_page(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let code = format!(
        r"
        const p = await context.newPage();
        await p.goto({PAGE:?});
        const solved = await neo.solveCaptcha(p);
        await p.close();
        return solved;"
    );
    let result = run(&client, &code, &json!({ "taskId": "e2e-captcha" })).await?;
    let solved = result
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .ok_or_else(|| format!("the solve answered a value: {result:?}"))?;
    let field = |name: &str| solved.get(name).cloned().unwrap_or(Value::Null);
    assert_eq!(
        (
            field("verdict"),
            field("reason"),
            field("type"),
            field("paidTasks")
        ),
        (
            json!("already_clear"),
            json!("no_captcha"),
            Value::Null,
            json!(0)
        ),
        "nothing blocks the page, and no provider was asked: {solved}"
    );
    assert_eq!(
        field("message"),
        json!("No CAPTCHA blocks the page (no CAPTCHA was found); carry on with the task.")
    );
    client.cancel().await?;
    Ok(())
}

/// A host locks a tab the agent opened with `context.newPage()` and loaded, the owner closes it from
/// outside Neo (Chromium's own `/json/close`), and the lock ends with the tab: the close reached
/// browseros-service from BrowserOS's target events. Returns the run that opened the tab and
/// the tab, for the trace check after the stop.
async fn a_hosts_lock_ends_when_its_tab_closes(
    rig: &Rig,
    client: &Client,
) -> Result<(RunId, i64), Box<dyn Error>> {
    let code = format!(
        r"
        const p = await context.newPage();
        await p.goto({LOCKED_PAGE:?});
        return p;"
    );
    let opened = run(client, &code, &json!({ "taskId": "e2e-lock" })).await?;
    let page = opened
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .ok_or_else(|| format!("the run returns its page: {opened:?}"))?;
    let tab = page
        .get("tab")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("a page carries its tab: {page}"))?;
    let run_id: RunId = outcome(&opened)?
        .get("id")
        .and_then(Value::as_str)
        .ok_or("every result names its run")?
        .parse()?;

    let take = json!({ "tabId": tab, "ttlSeconds": 600 });
    let locked = rig
        .host_call("POST", "/v1/locks", Some(HOST_TOKEN), Some(&take))
        .await?;
    let (head, body) = locked.split_once("\r\n\r\n").ok_or("an HTTP response")?;
    assert!(
        head.starts_with("HTTP/1.1 200"),
        "the host locks the tab: {locked}"
    );
    let lock: Value = serde_json::from_str(body)?;
    let id = lock
        .get("id")
        .and_then(Value::as_str)
        .ok_or("a lock has an id")?;

    let listed = get(rig.cdp_port(), "/json/list").await?;
    let (_, targets) = listed.split_once("\r\n\r\n").ok_or("an HTTP response")?;
    let targets: Value = serde_json::from_str(targets)?;
    let target = targets
        .as_array()
        .and_then(|targets| {
            targets
                .iter()
                .find(|target| target.get("title").and_then(Value::as_str) == Some(LOCKED_TITLE))
        })
        .and_then(|target| target.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("the locked page is a target: {listed}"))?;
    let closed = get(rig.cdp_port(), &format!("/json/close/{target}")).await?;
    assert!(
        closed.starts_with("HTTP/1.1 200"),
        "Chromium closes the tab: {closed}"
    );

    time::timeout(PATIENCE, async {
        loop {
            let read = rig
                .host_get(&format!("/v1/locks/{id}"), Some(HOST_TOKEN))
                .await?;
            let (_, body) = read.split_once("\r\n\r\n").ok_or("an HTTP response")?;
            let lock: Value = serde_json::from_str(body)?;
            if lock.get("releasedAt").is_some_and(Value::is_i64) {
                return Ok::<_, Box<dyn Error>>(());
            }
            time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_elapsed| format!("the lock did not end within {PATIENCE:?} of its tab closing"))??;
    Ok((run_id, tab))
}

/// The run as zuzu reads it over the host socket: refused without the bearer token, and with it
/// the run the trace recorded, with its five ops.
async fn read_back_over_the_host_socket(rig: &Rig, id: RunId) -> TestResult {
    let refused = rig.host_get(&format!("/v1/runs/{id}"), None).await?;
    assert!(
        refused.starts_with("HTTP/1.1 401"),
        "the host socket wants the bearer token: {refused}"
    );
    // `wait` holds the answer until the run ended and every screenshot job it made was
    // answered, so `complete` says browseros-service's camera took the writer's jobs here.
    let recorded = rig
        .host_get(&format!("/v1/runs/{id}?wait=30"), Some(HOST_TOKEN))
        .await?;
    let (head, body) = recorded.split_once("\r\n\r\n").ok_or("an HTTP response")?;
    assert!(head.starts_with("HTTP/1.1 200"), "{recorded}");
    let host_run: Value = serde_json::from_str(body)?;
    let field = |name: &str| host_run.get(name).cloned().unwrap_or(Value::Null);
    assert_eq!(
        (field("id"), field("status"), field("complete")),
        (json!(id.to_string()), json!("ok"), json!(true)),
        "the host reads the run as the trace recorded it: {host_run}"
    );
    let names: Vec<&str> = host_run
        .get("operations")
        .and_then(Value::as_array)
        .ok_or("a run lists its operations")?
        .iter()
        .filter_map(|op| op.get("name").and_then(Value::as_str))
        .collect();
    assert_eq!(
        names,
        [
            "context.newPage",
            "page.goto",
            "page.frames",
            "page.title",
            "page.close"
        ]
    );
    Ok(())
}
