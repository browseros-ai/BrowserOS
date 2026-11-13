//! The acceptance smoke for hosted Neo: one task, driven by an agent over MCP and watched and
//! steered by a host over the host socket, on a real BrowserOS with the real neo-http,
//! -takeover and -trace behind claw-sdk. Nothing is faked.
//!
//! ```text
//!  agent (MCP run, taskId T) ──▶ browseros-claw-sdk ── CDP ──▶ BrowserOS --headless
//!  host  (bearer, host.sock) ──▶   ├ /v1/runs, /v1/operations: neo-trace's store
//!                                  ├ /v1/locks: neo-takeover, asked by the door
//!                                  └ /v1/operations/{id}/screenshot: the service's camera
//! ```
//!
//! In order: the agent opens a tab and reads its title; the host reads that run's trace; the
//! host locks the tab and the agent's next write is refused `not_sent`; the host releases it and
//! the same write lands; the tab's picture after that write appears; SIGTERM stops Neo with 0.
//!
//! Like `e2e_browseros.rs`, it runs only when `CLAW_SDK_E2E_BROWSER` names a BrowserOS binary.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::error::Error;
use std::fmt::{self, Write as _};
use std::path::Path;
use std::time::Duration;

use browseros_rig::{
    CaptchaKey, Client, HOST_TOKEN, Rig, TestResult, browser_or_skip, get, outcome, run,
};
use neo_trace::{RunId, RunStatus, TraceStore};
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
    the_tab_has_its_picture_after_the_write(rig, &landed, tab).await?;

    client.cancel().await?;
    assert_eq!(rig.stop_neo().await?, Some(0), "SIGTERM is a normal stop");
    Ok(())
}

/// Inspect persisted snapshot results, before text rendering and after a later run reads the
/// baseline. Deliberate reads and fixture source can still contain these canaries elsewhere in
/// a trace, so the invariant belongs to snapshot operations rather than the whole database.
#[tokio::test(flavor = "multi_thread")]
async fn private_field_values_never_reach_stored_snapshot_results() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let mut rig = Rig::start(&browser, CaptchaKey::Absent)?;
    match time::timeout(TEST_BUDGET, snapshot_trace(&mut rig)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(format!("{error}\n{}", rig.logs()).into()),
        Err(_elapsed) => Err(format!("snapshot trace timed out\n{}", rig.logs()).into()),
    }
}

/// Each source and attribute value is a distinct synthetic secret to scan after persistence.
const TRACE_FIELDS: [(&str, &str, &str, &str); 11] = [
    (
        "Password",
        "input",
        "type='password'",
        "Trace-Password-Canary",
    ),
    (
        "Shown",
        "input",
        "autocomplete='current-password'",
        "Trace-Shown-Canary",
    ),
    (
        "Card",
        "input",
        "autocomplete='cc-number'",
        "Trace-Card-Canary",
    ),
    (
        "Security",
        "input",
        "autocomplete='cc-csc'",
        "Trace-Security-Canary",
    ),
    (
        "Code",
        "input",
        "autocomplete='one-time-code'",
        "Trace-Code-Canary",
    ),
    (
        "Note",
        "textarea",
        "autocomplete='new-password'",
        "Trace-Note-Canary",
    ),
    (
        "Self",
        "textarea",
        "id='self' role='button' aria-labelledby='self' autocomplete='new-password'",
        "Trace-Self-Canary",
    ),
    (
        "Numeric",
        "input",
        "id='numeric' type='number' autocomplete='one-time-code' aria-valuetext='864213'",
        "864213",
    ),
    (
        "Styled checkbox",
        "input",
        "class='css-private' type='checkbox' autocomplete='one-time-code'",
        "Trace-CSS-Checkbox-Canary",
    ),
    (
        "Styled text",
        "input",
        "class='css-private' autocomplete='current-password'",
        "Trace-CSS-Text-Canary",
    ),
    (
        "Styled self",
        "input",
        "id='css-self' class='css-private-self' type='checkbox' role='button' aria-labelledby='css-self' autocomplete='one-time-code'",
        "Trace-CSS-Self-Canary",
    ),
];

fn private_trace_html() -> Result<String, fmt::Error> {
    let mut html = String::from(
        "<title>Snapshot trace</title><style>\
         .css-private{appearance:none;width:40px;height:40px}\
         .css-private::before,.css-private::after{content:attr(value)}\
         .css-private-self{content:url(data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7) / attr(value)}\
         </style><main><h1>Checkout</h1>",
    );
    for (label, tag, attributes, value) in TRACE_FIELDS {
        if tag == "textarea" {
            write!(
                html,
                "<label>{label}<textarea {attributes}>{value}</textarea></label>"
            )?;
        } else {
            write!(
                html,
                "<label>{label}<input {attributes} value='{value}'></label>"
            )?;
        }
    }
    html.push_str(
        "\
         <button aria-labelledby='self'></button><button aria-labelledby='numeric'></button>\
         <label>Expiry<input autocomplete='cc-exp' value='12/30'></label>\
         <label>Holder<input autocomplete='cc-name' value='Ada Example'></label>",
    );
    // Diff context must stay smaller than half the whole output. A small page legitimately
    // falls back to a whole snapshot even when only one value changes.
    for index in 0..20 {
        write!(html, "<button>Stable action {index}</button>")?;
    }
    html.push_str("</main>");
    Ok(html)
}

fn assert_returned_snapshot_values(first: &CallToolResult) -> TestResult {
    let snapshots = first
        .structured_content
        .as_ref()
        .and_then(|value| value.pointer("/value/snapshots"))
        .and_then(Value::as_array)
        .ok_or_else(|| format!("the first run returns its snapshots: {first:?}"))?;
    assert_eq!(snapshots.len(), 7);
    for snapshot in snapshots {
        let printed = snapshot.to_string();
        for (_, _, _, canary) in TRACE_FIELDS {
            assert!(
                !printed.contains(canary),
                "snapshot exposes {canary}: {printed}"
            );
        }
        assert!(
            printed.contains("Ada Example"),
            "holder remains readable: {printed}"
        );
        assert!(
            printed.contains("12/30"),
            "expiry remains readable: {printed}"
        );
    }
    Ok(())
}

async fn assert_stored_snapshot_values(trace_path: &Path, ids: [RunId; 2]) -> TestResult {
    let store = TraceStore::open(trace_path).await?;
    let mut snapshot_count = 0;
    let mut baseline_count = 0;
    for id in ids {
        let detail = store.run(id).await?.ok_or("the run was persisted")?;
        assert_eq!(detail.summary.status, RunStatus::Ok);
        for operation in detail.operations {
            if !matches!(
                operation.name.as_str(),
                "page.ariaSnapshot"
                    | "locator.ariaSnapshot"
                    | "page.snapshotBaseline"
                    | "page.snapshotPending"
            ) {
                continue;
            }
            assert!(
                !operation.args_truncated,
                "snapshot inputs were stored whole"
            );
            assert!(
                !operation.result_truncated,
                "snapshot results were stored whole"
            );
            let args = operation
                .args
                .as_ref()
                .map(Value::to_string)
                .unwrap_or_default();
            for (_, _, _, canary) in TRACE_FIELDS {
                assert!(
                    !args.contains(canary),
                    "{} persisted {canary} in snapshot or pending-baseline input: {args}",
                    operation.name
                );
            }
            let result = operation
                .result
                .ok_or("the snapshot operation has a result")?;
            let result = result.to_string();
            for (_, _, _, canary) in TRACE_FIELDS {
                assert!(
                    !result.contains(canary),
                    "{} persisted {canary}: {result}",
                    operation.name
                );
            }
            if operation.name == "page.snapshotBaseline" {
                baseline_count += 1;
                assert!(
                    result.contains('•'),
                    "the stored baseline contains masked values"
                );
            }
            if operation.name.ends_with(".ariaSnapshot") {
                snapshot_count += 1;
                assert!(
                    result.contains('•'),
                    "the stored tree contains masked values"
                );
            }
        }
    }
    assert_eq!(snapshot_count, 9);
    assert_eq!(baseline_count, 1);
    Ok(())
}

async fn snapshot_trace(rig: &mut Rig) -> TestResult {
    let html = private_trace_html()?;
    rig.until_ready().await?;
    // Keep the entire fixture through the host's current settings contract. Clipped traces
    // could hide an unmasked field and make the canary scan pass without proving this boundary.
    let configured = rig
        .host(
            "PATCH",
            "/v1/settings",
            Some(HOST_TOKEN),
            Some(&json!({ "trace": { "valueBytes": 262_144 } })),
        )
        .await?;
    assert_eq!(configured.status, 200, "{}", configured.text());
    let client = rig.client().await?;
    // Commit the final v1 read as the baseline, with focus already stable for the next fill.
    // A changed output form or focus state could otherwise legitimately return a whole page.
    let first = run(
        &client,
        &format!(
            "const p = await context.newPage();\n\
             await p.goto('data:text/html,' + encodeURIComponent({}));\n\
             const snapshots = [\n\
               await p.ariaSnapshot(), await p.ariaSnapshot({{ text: true }}),\n\
               await p.ariaSnapshot({{ text: false }}),\n\
               await p.ariaSnapshot({{ mode: 'full' }}),\n\
               await p.ariaSnapshot({{ mode: 'default' }}),\n\
               await p.ariaSnapshotJSON(),\n\
               await p.locator('main').ariaSnapshotJSON()\n\
             ];\n\
             await p.getByLabel('Password', {{ exact: true }}).focus();\n\
             const baseline = await p.ariaSnapshot({{ text: false }});\n\
             return {{ page: p, snapshots, baseline }};",
            json!(html)
        ),
        &json!({ "requestId": "mask-trace-1", "taskId": "mask-trace" }),
    )
    .await?;
    let first_id: RunId = outcome(&first)?["id"]
        .as_str()
        .ok_or("the first run has an id")?
        .parse()?;
    assert_returned_snapshot_values(&first)?;
    let changed = run(
        &client,
        "await page.getByLabel('Password', { exact: true }).fill('Trace-Password-Canary!');\n\
         return await page.ariaSnapshot({ delta: true, text: false });",
        &json!({ "requestId": "mask-trace-2", "taskId": "mask-trace" }),
    )
    .await?;
    let second_id: RunId = outcome(&changed)?["id"]
        .as_str()
        .ok_or("the second run has an id")?
        .parse()?;
    let delta = changed
        .structured_content
        .as_ref()
        .and_then(|value| value.get("value"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("the second run returns its delta: {changed:?}"))?;
    let baseline = first
        .structured_content
        .as_ref()
        .and_then(|value| value.pointer("/value/baseline"))
        .and_then(Value::as_str);
    assert!(
        delta.starts_with("# changes since"),
        "a stored baseline is read: {delta}\nprevious: {baseline:?}"
    );
    assert!(delta.contains(&"•".repeat("Trace-Password-Canary!".len())));
    client.cancel().await?;
    assert_eq!(rig.stop_neo().await?, Some(0));
    assert_stored_snapshot_values(&rig.trace_path(), [first_id, second_id]).await
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
    let tab = page
        .get("tab")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("the brand carries the browser's tab: {page}"))?;
    assert_eq!(
        page,
        &json!({ "$neo": "page", "tab": tab }),
        "a page leaves a run with only its brand and the browser's tab"
    );
    let (id, status, effects) = run_meta(&result)?;
    assert_eq!(
        (status.as_str(), effects.as_str()),
        ("ok", "done"),
        "{result:?}"
    );
    Ok((id, tab))
}

/// The host reads run 1 back: its ids, acted-on tabs, and three script operations whose
/// deliveries match what the agent did. Automatic monitor reads remain in the raw trace.
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
        ],
        [
            json!(id),
            json!("ok"),
            json!("done"),
            json!(true),
            json!(TASK),
            json!("acceptance-1"),
        ],
        "the host reads the run as the agent ran it: {host_run}"
    );
    // Listening alone does not claim a tab. Only the tab this script opened/acted on
    // belongs to the run, even when worker monitors name other tabs in the raw trace.
    assert_eq!(field("tabs"), json!([tab]));
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
    assert!(
        goto.strip_prefix(&format!("{id}."))
            .and_then(|seq| seq.parse::<u64>().ok())
            .is_some_and(|seq| seq > 0),
        "an operation id is <runId>.<positive seq>: {goto}"
    );
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
/// sent, the page stays where it was, and the trace records the refusal. The run's page reads its
/// frames before the script's first statement, and the lock, which blocks reads, refuses that
/// too; the run starts on the main frame alone.
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
        [
            op_of("page.frames", "read", "error", "not_sent", Some(tab)),
            op_of("page.goto", "write", "error", "not_sent", Some(tab)),
        ],
        "the trace records the refused read of the frames and the refused write on the locked \
         tab: {host_run}"
    );
    let goto = operation(&host_run, "page.goto")?;
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

/// Run 3's picture of its tab, as the host finds it. Each write keeps its own screenshot (the
/// trace writer no longer folds a write's picture into a later read's on the same tab), so the
/// `page.goto` holds it. The run reads initial frames before its first statement; goto returns
/// the navigated tree inline in its navigation publication instead of sending another read. The
/// run's record names that op, and its JPEG is whole.
async fn the_tab_has_its_picture_after_the_write(rig: &Rig, run: &str, tab: i64) -> TestResult {
    // Held until the run is complete: ended, and every screenshot job answered.
    let host_run = read_run(rig, run).await?;
    assert_eq!(
        operations(&host_run)?,
        [
            op_of("page.frames", "read", "ok", "not_sent", Some(tab)),
            op_of("page.goto", "write", "ok", "done", Some(tab)),
        ],
        "{host_run}"
    );
    let write = operation_id(&host_run, "page.goto")?;
    let op = operation(&host_run, "page.goto")?;
    assert_eq!(
        (op.get("id"), op.get("hasScreenshot"),),
        (Some(&json!(write)), Some(&json!(true))),
        "the write holds its own picture of the tab: {host_run}"
    );
    let shot = rig
        .host(
            "GET",
            &format!("/v1/operations/{write}/screenshot"),
            Some(HOST_TOKEN),
            None,
        )
        .await?;
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
        .filter(|op| !browseros_rig::is_worker_event_operation(op))
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

/// Background reads consume sequence numbers too; find the script's operation by its name.
fn operation<'a>(run: &'a Value, name: &str) -> Result<&'a Value, Box<dyn Error>> {
    run.get("operations")
        .and_then(Value::as_array)
        .and_then(|ops| {
            ops.iter()
                .find(|op| op.get("name").and_then(Value::as_str) == Some(name))
        })
        .ok_or_else(|| format!("the run has a {name} operation: {run}").into())
}

fn operation_id(run: &Value, name: &str) -> Result<String, Box<dyn Error>> {
    operation(run, name)?
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("the {name} operation has an id: {run}").into())
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
