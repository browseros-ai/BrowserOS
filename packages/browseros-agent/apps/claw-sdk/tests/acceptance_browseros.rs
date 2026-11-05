//! The acceptance smoke for hosted Neo: one task, driven by an agent over MCP and watched and
//! steered by a host over the host socket, on a real BrowserOS with the real browseros-service,
//! -takeover and -trace behind claw-sdk. Nothing is faked.
//!
//! ```text
//!  agent (MCP run, taskId T) ──▶ browseros-claw-sdk ── CDP ──▶ BrowserOS --headless
//!  host  (bearer, host.sock) ──▶   ├ /v1/runs, /v1/operations: browseros-trace's store
//!                                  ├ /v1/locks: browseros-takeover, asked by the door
//!                                  └ /v1/operations/{id}/screenshot: the service's camera
//! ```
//!
//! In order: the agent opens a tab and reads its title; the host reads that run's trace; the
//! host locks the tab and the agent's next write is refused `not_sent`; the host releases it and
//! the same write lands; that write's screenshot appears; SIGTERM stops Neo with 0.
//!
//! Like `e2e_browseros.rs`, it runs only when `CLAW_SDK_E2E_BROWSER` names a BrowserOS binary.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::error::Error;
use std::time::Duration;

use browseros_rig::{
    CaptchaKey, Client, HOST_TOKEN, HostResponse, PATIENCE, Rig, TestResult, browser_or_skip, get,
    outcome, run,
};
use rmcp::model::CallToolResult;
use serde_json::{Value, json};
use tokio::time;

/// The whole drive; see `e2e_browseros.rs` for why the budget sits inside the test.
const TEST_BUDGET: Duration = Duration::from_secs(240);
/// One host task. Every run carries it, so each later run's `page` is the tab the first opened:
/// the agent writes on the locked tab the way a hosted agent continues its task.
const TASK: &str = "acceptance";
const FIRST_PAGE: &str = "data:text/html,<title>Neo acceptance</title><p>opened by the agent</p>";
const FIRST_TITLE: &str = "Neo acceptance";
const SECOND_PAGE: &str =
    "data:text/html,<title>Neo acceptance, released</title><p>after the owner let go</p>";
const SECOND_TITLE: &str = "Neo acceptance, released";
/// Every JPEG starts with SOI and ends with EOI.
const JPEG_START: &[u8] = &[0xff, 0xd8, 0xff];
const JPEG_END: &[u8] = &[0xff, 0xd9];

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_and_a_host_share_one_tab_on_a_real_browseros() -> TestResult {
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

    let (opened, tab) = the_agent_opens_a_tab(&client).await?;
    the_host_reads_the_run(rig, &opened, tab).await?;
    a_lock_refuses_the_agent(rig, &client, tab).await?;
    let landed = the_write_lands_once_released(rig, &client).await?;
    the_write_has_a_screenshot(rig, &landed, tab).await?;

    client.cancel().await?;
    assert_eq!(rig.stop_neo().await?, Some(0), "SIGTERM is a normal stop");
    Ok(())
}

/// Run 1: a new tab, a navigation and a read, ending with the tab open and its handle returned.
/// Returns the run id and the tab.
async fn the_agent_opens_a_tab(client: &Client) -> Result<(String, i64), Box<dyn Error>> {
    let code = format!(
        r"
        const p = await context.newPage();
        await p.goto({FIRST_PAGE:?});
        return {{ title: await p.title(), page: p }};"
    );
    let meta = json!({ "requestId": "acceptance-1", "taskId": TASK });
    let result = run(client, &code, &meta).await?;
    let value = value(&result)?;
    assert_eq!(value.get("title"), Some(&json!(FIRST_TITLE)), "{result:?}");
    let page = value.get("page").ok_or("the run returns its page")?;
    assert_eq!(
        page.get("$neo"),
        Some(&json!("page")),
        "a page leaves a run as its brand: {page}"
    );
    assert!(
        page.get("id").is_some_and(Value::is_u64),
        "the brand carries Neo's page id: {page}"
    );
    let tab = page
        .get("tab")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("the brand carries the browser's tab: {page}"))?;
    let (id, status, effects) = run_meta(&result)?;
    assert_eq!(
        (status.as_str(), effects.as_str()),
        ("ok", "done"),
        "{result:?}"
    );
    Ok((id, tab))
}

/// The host reads run 1 back: its ids, its tab, and three operations whose deliveries match what
/// the agent did (two writes answered, one read never sent).
async fn the_host_reads_the_run(rig: &Rig, id: &str, tab: i64) -> TestResult {
    let host_run = read_run(rig, id).await?;
    let field = |name: &str| host_run.get(name).cloned().unwrap_or(Value::Null);
    assert_eq!(
        [
            field("id"),
            field("status"),
            field("effects"),
            field("complete"),
            field("taskId"),
            field("requestId"),
            field("tabs"),
        ],
        [
            json!(id),
            json!("ok"),
            json!("done"),
            json!(true),
            json!(TASK),
            json!("acceptance-1"),
            json!([tab]),
        ],
        "the host reads the run as the agent ran it: {host_run}"
    );
    assert_eq!(
        host_run.pointer("/result/title"),
        Some(&json!(FIRST_TITLE)),
        "with the script's value: {host_run}"
    );
    assert_eq!(
        operations(&host_run)?,
        [
            op_of("context.newPage", "write", "ok", "done", None),
            op_of("page.goto", "write", "ok", "done", Some(tab)),
            op_of("page.title", "read", "ok", "not_sent", Some(tab)),
        ],
        "every op once, in start order: {host_run}"
    );

    let goto = operation_id(&host_run, "page.goto")?;
    assert_eq!(goto, format!("{id}.2"), "an operation id is <runId>.<seq>");
    let read = rig
        .host(
            "GET",
            &format!("/v1/operations/{goto}"),
            Some(HOST_TOKEN),
            None,
        )
        .await?;
    assert_eq!(read.status, 200, "{}", read.text());
    let goto_op = read.json()?;
    assert_eq!(
        (
            goto_op.get("runId"),
            goto_op.pointer("/page/tabId"),
            goto_op.pointer("/args/url"),
        ),
        (
            Some(&json!(id)),
            Some(&json!(tab)),
            Some(&json!(FIRST_PAGE))
        ),
        "one operation, with the page it acted on and its arguments: {goto_op}"
    );
    Ok(())
}

/// The host locks the tab; run 2 of the task writes on it and is refused before anything is
/// sent, the page stays where it was, and the trace records the refusal.
async fn a_lock_refuses_the_agent(rig: &Rig, client: &Client, tab: i64) -> TestResult {
    let take = json!({ "tabId": tab, "ttlSeconds": 600 });
    let taken = rig
        .host("POST", "/v1/locks", Some(HOST_TOKEN), Some(&take))
        .await?;
    assert_eq!(
        taken.status,
        200,
        "the host locks the tab: {}",
        taken.text()
    );
    let lock = taken.json()?;
    assert_eq!(
        (
            lock.get("tabId"),
            lock.get("reads"),
            lock.get("settled"),
            lock.get("releasedAt"),
        ),
        (
            Some(&json!(tab)),
            Some(&json!("block")),
            Some(&json!(true)),
            None
        ),
        "held, with nothing of the agent's in flight: {lock}"
    );
    let lock_id = lock
        .get("id")
        .and_then(Value::as_str)
        .ok_or("a lock has an id")?;

    let refused = run(client, &goto_and_report(), &json!({ "taskId": TASK })).await?;
    assert_eq!(
        value(&refused)?,
        &json!({ "code": "owner_has_control", "owner": "host", "delivery": "not_sent" }),
        "the door refuses the write for the host: {refused:?}"
    );
    let (refused_id, status, effects) = run_meta(&refused)?;
    assert_eq!(
        (status.as_str(), effects.as_str()),
        ("ok", "none"),
        "the script caught it, and no write was sent: {refused:?}"
    );

    let host_run = read_run(rig, &refused_id).await?;
    assert_eq!(
        operations(&host_run)?,
        [op_of("page.goto", "write", "error", "not_sent", Some(tab))],
        "the trace records the refused write on the locked tab: {host_run}"
    );
    let goto = host_run.pointer("/operations/0").ok_or("one operation")?;
    assert_eq!(
        (goto.pointer("/error/owner"), goto.pointer("/error/code")),
        (Some(&json!("host")), Some(&json!("owner_has_control"))),
        "{goto}"
    );
    assert_eq!(
        tab_titles(rig).await?,
        [FIRST_TITLE, "about:blank"],
        "the locked tab never navigated"
    );

    let released = rig
        .host(
            "DELETE",
            &format!("/v1/locks/{lock_id}"),
            Some(HOST_TOKEN),
            None,
        )
        .await?;
    assert_eq!(released.status, 200, "{}", released.text());
    let released = released.json()?;
    assert!(
        released.get("releasedAt").is_some_and(Value::is_i64),
        "the host released the lock: {released}"
    );
    Ok(())
}

/// Run 3 of the task, after the release: the same write lands on the same tab. Returns the
/// run's id.
async fn the_write_lands_once_released(
    rig: &Rig,
    client: &Client,
) -> Result<String, Box<dyn Error>> {
    let landed = run(client, &goto_and_report(), &json!({ "taskId": TASK })).await?;
    assert_eq!(value(&landed)?, &json!("navigated"), "{landed:?}");
    let (id, status, effects) = run_meta(&landed)?;
    assert_eq!(
        (status.as_str(), effects.as_str()),
        ("ok", "done"),
        "{landed:?}"
    );
    assert_eq!(
        tab_titles(rig).await?,
        [SECOND_TITLE, "about:blank"],
        "the task's tab navigated"
    );
    Ok(id)
}

/// Run 3's write, as the host finds it: its screenshot polled from the moment the run returned
/// until the camera stored it, a whole JPEG, then the run's record. The write is its run's only
/// operation on the tab, so its own job is the only one for the tab and nothing coalesces it
/// away.
async fn the_write_has_a_screenshot(rig: &Rig, run: &str, tab: i64) -> TestResult {
    let op = format!("{run}.1");
    let path = format!("/v1/operations/{op}/screenshot");
    let polled = time::timeout(PATIENCE, async {
        loop {
            let read = rig.host("GET", &path, Some(HOST_TOKEN), None).await?;
            // 404 until the camera's answer reaches the writer's next batch.
            if read.status != 404 {
                return Ok::<HostResponse, Box<dyn Error>>(read);
            }
            time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    let Ok(shot) = polled else {
        // The camera answers `Dropped` for a capture past its timeout (browseros-service's
        // CAPTURE_TIMEOUT) and never retries; neo.log, appended below, names the cause.
        let record = rig
            .host(
                "GET",
                &format!("/v1/operations/{op}"),
                Some(HOST_TOKEN),
                None,
            )
            .await?;
        return Err(format!(
            "no screenshot for {op} within {PATIENCE:?}; the operation reads {}",
            record.text()
        )
        .into());
    };
    let shot = shot?;
    assert_eq!(
        (shot.status, shot.header("content-type")),
        (200, Some("image/jpeg")),
        "{}",
        shot.head
    );
    assert!(
        shot.body.starts_with(JPEG_START) && shot.body.ends_with(JPEG_END),
        "a whole JPEG, {} bytes",
        shot.body.len()
    );

    let host_run = read_run(rig, run).await?;
    assert_eq!(
        operations(&host_run)?,
        [op_of("page.goto", "write", "ok", "done", Some(tab))],
        "{host_run}"
    );
    assert_eq!(
        (
            host_run.pointer("/operations/0/id"),
            host_run.pointer("/operations/0/hasScreenshot"),
        ),
        (Some(&json!(op)), Some(&json!(true))),
        "the run's record says the write has its screenshot: {host_run}"
    );
    Ok(())
}

/// The task's `page` goes to the second page; a refusal comes back as a value, so the run ends
/// `ok` and the trace keeps the refused op.
fn goto_and_report() -> String {
    format!(
        r"
        try {{
          await page.goto({SECOND_PAGE:?});
          return 'navigated';
        }} catch (e) {{
          return {{ code: e.code, owner: e.owner, delivery: e.delivery }};
        }}"
    )
}

/// The titles of the browser's tabs, sorted: the agent's tab and the launch's `about:blank`.
async fn tab_titles(rig: &Rig) -> Result<Vec<String>, Box<dyn Error>> {
    let listed = get(rig.cdp_port(), "/json/list").await?;
    let (_, body) = listed.split_once("\r\n\r\n").ok_or("an HTTP response")?;
    let targets: Vec<Value> = serde_json::from_str(body)?;
    let mut titles: Vec<String> = targets
        .iter()
        .filter(|target| target.get("type") == Some(&json!("page")))
        .filter_map(|target| target.get("title").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    titles.sort();
    Ok(titles)
}

/// The host's read of a run, held until it is complete (ended, every screenshot job answered).
async fn read_run(rig: &Rig, id: &str) -> Result<Value, Box<dyn Error>> {
    let read = rig
        .host(
            "GET",
            &format!("/v1/runs/{id}?wait=30"),
            Some(HOST_TOKEN),
            None,
        )
        .await?;
    assert_eq!(read.status, 200, "{}", read.text());
    let run = read.json()?;
    assert_eq!(run.get("complete"), Some(&json!(true)), "{run}");
    Ok(run)
}

/// An operation as these assertions compare it: name, effect, outcome, delivery, tab.
type Op = (String, String, String, String, Option<i64>);

fn op_of(name: &str, effect: &str, outcome: &str, delivery: &str, tab: Option<i64>) -> Op {
    (
        name.to_owned(),
        effect.to_owned(),
        outcome.to_owned(),
        delivery.to_owned(),
        tab,
    )
}

fn operations(run: &Value) -> Result<Vec<Op>, Box<dyn Error>> {
    let text = |op: &Value, name: &str| {
        op.get(name)
            .and_then(Value::as_str)
            .unwrap_or("<absent>")
            .to_owned()
    };
    Ok(run
        .get("operations")
        .and_then(Value::as_array)
        .ok_or("a run lists its operations")?
        .iter()
        .map(|op| {
            (
                text(op, "name"),
                text(op, "effect"),
                text(op, "outcome"),
                text(op, "delivery"),
                op.get("tabId").and_then(Value::as_i64),
            )
        })
        .collect())
}

fn operation_id(run: &Value, name: &str) -> Result<String, Box<dyn Error>> {
    run.get("operations")
        .and_then(Value::as_array)
        .and_then(|ops| {
            ops.iter()
                .find(|op| op.get("name").and_then(Value::as_str) == Some(name))
        })
        .and_then(|op| op.get("id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("the run has a {name} operation: {run}").into())
}

/// The run's returned value.
fn value(result: &CallToolResult) -> Result<&Value, Box<dyn Error>> {
    result
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .ok_or_else(|| format!("the run returned a value: {result:?}").into())
}

/// `_meta["com.browseros.neo/run"]`'s id, status and effects.
fn run_meta(result: &CallToolResult) -> Result<(String, String, String), Box<dyn Error>> {
    let meta = outcome(result)?;
    let text = |name: &str| {
        meta.get(name)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("the run's _meta has {name}: {meta}"))
    };
    Ok((text("id")?, text("status")?, text("effects")?))
}
