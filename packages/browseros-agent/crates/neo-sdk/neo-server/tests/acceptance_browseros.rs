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
#[path = "../../neo-playwright/tests/support/fixture_server.rs"]
#[expect(
    dead_code,
    reason = "the shared server also supports script and nested-frame fixtures"
)]
pub mod fixture_server;

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{self, Write as _};
use std::path::Path;
use std::time::Duration;

use browseros_rig::{
    CaptchaKey, Client, HOST_TOKEN, Rig, TestResult, browser_or_skip, get, outcome, run,
};
use fixture_server::{FixtureServer, Page};
use neo_trace::{RedactionLimits, Redactor, RunDetail, RunId, RunStatus, TraceStore};
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

/// The same SDK case matrix as private_failures_live, now through MCP and a real trace writer.
/// A deliberate read and caller-supplied Expected may contain a canary; page-derived error
/// messages, matcher facts and operation notes must already be safe before persistence.
#[tokio::test(flavor = "multi_thread")]
async fn private_field_values_never_reach_stored_failure_diagnostics() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let mut rig = Rig::start(&browser, CaptchaKey::Absent)?;
    match time::timeout(TEST_BUDGET, private_failure_trace(&mut rig)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(format!("{error}\n{}", rig.logs()).into()),
        Err(_elapsed) => Err(format!("private failure trace timed out\n{}", rig.logs()).into()),
    }
}

const FAILURE_CANARIES: [&str; 3] = ["AttrCanary-1", "TypedCanary-2", "TextCanary-3"];
const PRIVATE_FAILURE_HTML: &str =
    include_str!("../../neo-playwright/tests/data/private-failure-fields/page.html");
const PRIVATE_FAILURE_CASES: &str =
    include_str!("../../neo-playwright/tests/data/private-failure-fields/cases.js");

fn assert_failure_canaries_absent(text: &str) {
    for canary in FAILURE_CANARIES {
        assert!(!text.contains(canary), "failure exposes {canary}: {text}");
    }
}

async fn private_failure_trace(rig: &mut Rig) -> TestResult {
    let server = FixtureServer::start(HashMap::from([(
        "/private-failures.html".to_owned(),
        Page::html(PRIVATE_FAILURE_HTML),
    )]))
    .await?;
    rig.until_ready().await?;
    let configured = rig
        .host(
            "PATCH",
            "/v1/settings",
            Some(HOST_TOKEN),
            Some(&json!({ "trace": { "values": "on", "valueBytes": 262_144 } })),
        )
        .await?;
    assert_eq!(configured.status, 200, "{}", configured.text());
    let client = rig.client().await?;
    let script = PRIVATE_FAILURE_CASES.replace(
        "__FIXTURE_URL__",
        &json!(server.url("127.0.0.1", "/private-failures.html")).to_string(),
    );
    let result = run(
        &client,
        &script,
        &json!({ "requestId": "private-failure-trace", "taskId": "private-failure-trace" }),
    )
    .await?;
    let (id, status, _) = run_meta(&result)?;
    assert_eq!(
        status, "ok",
        "every deliberate failure was caught: {result:?}"
    );
    let returned = value(&result)?;
    let failures = returned
        .get("failures")
        .and_then(Value::as_object)
        .ok_or("the run returned its named failure matrix")?;
    let clear_templates = returned
        .get("clearTemplates")
        .and_then(Value::as_array)
        .ok_or("the two caller-supplied clear templates were rejected")?;
    assert!(
        failures.len() >= 40,
        "the complete live matrix ran through MCP"
    );
    assert_eq!(clear_templates.len(), 2);
    for field in ["failures", "clearTemplates", "snapshots"] {
        assert_failure_canaries_absent(
            &returned
                .get(field)
                .ok_or("the diagnostic field")?
                .to_string(),
        );
    }
    assert_eq!(
        returned
            .get("reads")
            .and_then(|reads| reads.get("inputValue")),
        Some(&json!("TypedCanary-2")),
        "explicit reads remain clear in the same run"
    );
    let expected_failures = failures.len() + clear_templates.len();
    // Wait for operation records and screenshots before stopping the writer. Reading the
    // database only after a clean stop proves storage, rather than the service's live cache.
    read_run(rig, &id).await?;
    client.cancel().await?;
    assert_eq!(rig.stop_neo().await?, Some(0));
    assert_stored_failure_diagnostics(&rig.trace_path(), id.parse()?, expected_failures).await
}

/// Trace messages have a fixed text budget independent of valueBytes. Preserve and scan the
/// complete captured failures in the run result, then require each stored message to equal
/// their bounded projection so truncation cannot turn an unsafe diagnostic into a passing test.
fn complete_stored_failure_messages(
    detail: &RunDetail,
    expected_failures: usize,
) -> Result<HashSet<String>, Box<dyn Error>> {
    assert!(
        !detail.result_truncated,
        "the full diagnostic matrix was stored"
    );
    let result = detail.result.as_ref().ok_or("the stored run result")?;
    for field in ["failures", "clearTemplates", "snapshots"] {
        assert_failure_canaries_absent(
            &result
                .get(field)
                .ok_or("the complete diagnostic field")?
                .to_string(),
        );
    }
    let failures = result
        .get("failures")
        .and_then(Value::as_object)
        .ok_or("the complete failure map")?;
    let templates = result
        .get("clearTemplates")
        .and_then(Value::as_array)
        .ok_or("the caller templates")?;
    assert_eq!(failures.len() + templates.len(), expected_failures);
    let redactor = Redactor::new(RedactionLimits::default());
    let mut messages = HashSet::new();
    for failure in failures.values() {
        let message = failure
            .get("message")
            .and_then(Value::as_str)
            .ok_or("the complete failure message")?;
        assert!(
            !message.contains("…[+"),
            "the source failure was retained whole"
        );
        messages.insert(redactor.text(message));
    }
    Ok(messages)
}

async fn assert_stored_failure_diagnostics(
    path: &Path,
    id: RunId,
    expected_failures: usize,
) -> TestResult {
    let store = TraceStore::open(path).await?;
    let detail = store
        .run(id)
        .await?
        .ok_or("the failure run was persisted")?;
    assert_eq!(detail.summary.status, RunStatus::Ok);
    let safe_messages = complete_stored_failure_messages(&detail, expected_failures)?;
    let (mut failed, mut caller_templates, mut markers, mut public, mut notes) = (0, 0, 0, 0, 0);
    for operation in detail.operations {
        for note in &operation.notes {
            assert_failure_canaries_absent(note);
            assert!(
                !note.contains("…[+"),
                "the diagnostic note was stored whole"
            );
            notes += 1;
        }
        let Some(error) = operation.error else {
            continue;
        };
        failed += 1;
        assert!(
            !operation.args_truncated,
            "the matcher inputs were stored whole"
        );
        assert!(
            !operation.result_truncated,
            "the matcher result was stored whole"
        );
        let args = operation
            .args
            .as_ref()
            .ok_or("the failed operation kept its inputs")?;
        let caller_expected =
            args.get("expected")
                .and_then(Value::as_str)
                .is_some_and(|expected| {
                    FAILURE_CANARIES
                        .iter()
                        .any(|canary| expected.contains(canary))
                });
        if caller_expected {
            // These two templates contain a canary supplied by the script. The shared
            // script separately scans their actual/log/ariaSnapshot facts; stored notes
            // above remain covered without erasing the caller's Expected line.
            caller_templates += 1;
        } else {
            assert_failure_canaries_absent(&error.message);
            assert!(
                safe_messages.contains(&error.message),
                "{} kept the bounded form of its complete safe failure: {}",
                operation.name,
                error.message,
            );
        }
        if let Some(result) = operation.result {
            assert_failure_canaries_absent(&result.to_string());
        }
        markers += usize::from(
            error
                .message
                .lines()
                .any(|line| line.starts_with("Received") && line.contains(" characters>\"")),
        );
        public += usize::from(error.message.contains("Received: \"Public"));
    }
    assert_eq!(
        failed, expected_failures,
        "every caught failure reached the trace"
    );
    assert_eq!(
        caller_templates, 2,
        "only caller-supplied Expected text is exempt"
    );
    assert!(
        markers >= 10,
        "real value comparisons stored received-length markers"
    );
    assert!(
        public >= 4,
        "public twins retain their real received values"
    );
    assert!(
        notes > failed,
        "the trace retained the original operation notes"
    );
    Ok(())
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

/// Closed roots must contribute their own masked fields, not merely hide every canary.
const CLOSED_TRACE_SCOPES: [&str; 3] = ["Closed", "Nested", "CrossClosed"];

fn closed_trace_fields(scope: &str) -> String {
    format!(
        "<section aria-label='{scope} fields'>\
         <label>{scope} password<input type='password' value='Trace-{scope}-Password-Canary'></label>\
         <label>{scope} textarea<textarea autocomplete='new-password'>Trace-{scope}-Textarea-Canary</textarea></label>\
         <label>{scope} public<input value='Public-{scope}-Readable'></label></section>"
    )
}

/// Page scripts retain roots only in their own closures. Neo must discover them through its
/// first AI walk; the child is served on another site so it owns a separate probe world.
fn private_trace_pages() -> Result<HashMap<String, Page>, fmt::Error> {
    let top = format!(
        r"{}<script>(() => {{
const closed = document.querySelector('#closed').attachShadow({{ mode: 'closed' }});
closed.innerHTML = {};
closed.querySelector('#nested').attachShadow({{ mode: 'closed' }}).innerHTML = {};
document.querySelector('#cross').src = `http://localhost:${{location.port}}/private-child.html`;
}})();</script>",
        private_trace_html()?,
        json!(format!(
            "{}<div id='nested'></div>",
            closed_trace_fields("Closed")
        )),
        json!(closed_trace_fields("Nested")),
    );
    let child = format!(
        r#"<!doctype html><html><body><div id="cross-closed"></div><script>(() => {{
const closed = document.querySelector('#cross-closed').attachShadow({{ mode: 'closed' }});
closed.innerHTML = {};
}})();</script></body></html>"#,
        json!(closed_trace_fields("CrossClosed")),
    );
    Ok(HashMap::from([
        ("/private.html".to_owned(), Page::html(&top)),
        ("/private-child.html".to_owned(), Page::html(&child)),
    ]))
}

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
    html.push_str(
        "<div id='closed'></div><iframe id='cross' title='Cross closed fields'></iframe></main>",
    );
    Ok(html)
}

fn assert_trace_canaries_absent(text: &str) {
    for (_, _, _, canary) in TRACE_FIELDS {
        assert!(!text.contains(canary), "snapshot exposes {canary}: {text}");
    }
    for scope in CLOSED_TRACE_SCOPES {
        for kind in ["Password", "Textarea"] {
            let canary = format!("Trace-{scope}-{kind}-Canary");
            assert!(!text.contains(&canary), "snapshot exposes {canary}: {text}");
        }
    }
}

/// Stored op results wrap trees and baseline text differently. Search the decoded JSON, so
/// field values are checked at their own node or line before and after persistence.
fn trace_field_node<'tree>(tree: &'tree Value, name: &str) -> Option<&'tree Value> {
    match tree {
        Value::Array(nodes) => nodes.iter().find_map(|node| trace_field_node(node, name)),
        Value::Object(node) if node.get("name").and_then(Value::as_str) == Some(name) => Some(tree),
        Value::Object(node) => node
            .values()
            .find_map(|value| trace_field_node(value, name)),
        _ => None,
    }
}

fn trace_field_text<'tree>(tree: &'tree Value, label: &str) -> Option<&'tree str> {
    match tree {
        Value::String(text) if text.contains(label) => Some(text),
        Value::Array(nodes) => nodes.iter().find_map(|node| trace_field_text(node, label)),
        Value::Object(node) => node
            .values()
            .find_map(|value| trace_field_text(value, label)),
        _ => None,
    }
}

fn assert_trace_field_value(snapshot: &Value, label: &str, value: &str) -> TestResult {
    if let Some(node) = trace_field_node(snapshot, label) {
        assert_eq!(node["role"].as_str(), Some("textbox"), "{node}");
        assert_eq!(node["text"].as_str(), Some(value), "{node}");
        return Ok(());
    }
    let label = format!("textbox \"{label}\"");
    let text = trace_field_text(snapshot, &label)
        .ok_or_else(|| format!("missing {label} in {snapshot}"))?;
    let line = text
        .lines()
        .find(|line| line.contains(&label))
        .ok_or("the field has its own line")?;
    assert!(
        line.ends_with(&format!(": {value}")) || line.ends_with(&format!(": \"{value}\"")),
        "{line}"
    );
    Ok(())
}

fn assert_closed_trace_visibility(snapshot: &Value, scopes: &[&str]) -> TestResult {
    for scope in scopes {
        for (name, kind) in [("password", "Password"), ("textarea", "Textarea")] {
            let canary = format!("Trace-{scope}-{kind}-Canary");
            assert_trace_field_value(
                snapshot,
                &format!("{scope} {name}"),
                &"•".repeat(canary.len()),
            )?;
        }
        assert_trace_field_value(
            snapshot,
            &format!("{scope} public"),
            &format!("Public-{scope}-Readable"),
        )?;
    }
    Ok(())
}

fn assert_returned_snapshot_values(first: &CallToolResult) -> TestResult {
    let returned = first
        .structured_content
        .as_ref()
        .and_then(|value| value.get("value"))
        .ok_or_else(|| format!("the first run returns its snapshots: {first:?}"))?;
    assert_eq!(returned["closedRootsOpaque"], json!([true, true, true]));
    for key in ["snapshots", "childSnapshots"] {
        let snapshots = returned[key].as_array().ok_or("the collected snapshots")?;
        assert_eq!(snapshots.len(), 7);
        for (index, snapshot) in snapshots.iter().enumerate() {
            let printed = snapshot.to_string();
            assert_trace_canaries_absent(&printed);
            let scopes: &[&str] = match key {
                // Page default mode and unnamed JSON reads stay in one frame. The child
                // locator covers that mode without changing the existing traversal contract.
                "snapshots" if index >= 4 => &["Closed", "Nested"],
                "snapshots" => &CLOSED_TRACE_SCOPES,
                _ => &["CrossClosed"],
            };
            assert_closed_trace_visibility(snapshot, scopes)?;
            if key != "snapshots" {
                continue;
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
    }
    assert_trace_canaries_absent(&returned["baseline"].to_string());
    assert_closed_trace_visibility(&returned["baseline"], &CLOSED_TRACE_SCOPES)
}

async fn assert_stored_snapshot_values(trace_path: &Path, ids: [RunId; 2]) -> TestResult {
    let store = TraceStore::open(trace_path).await?;
    let mut snapshot_count = 0;
    let mut baseline_count = 0;
    let mut closed_counts = [0; 3];
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
            assert_trace_canaries_absent(&args);
            let result = operation
                .result
                .ok_or("the snapshot operation has a result")?;
            assert_trace_canaries_absent(&result.to_string());
            if operation.name == "page.snapshotBaseline" {
                baseline_count += 1;
                assert_closed_trace_visibility(&result, &CLOSED_TRACE_SCOPES)?;
            }
            if operation.name.ends_with(".ariaSnapshot") {
                snapshot_count += 1;
                assert!(
                    result.to_string().contains('•'),
                    "the stored tree contains masked values"
                );
                for (scope, count) in CLOSED_TRACE_SCOPES.iter().zip(&mut closed_counts) {
                    if trace_field_node(&result, &format!("{scope} password")).is_some() {
                        assert_closed_trace_visibility(&result, &[*scope])?;
                        *count += 1;
                    }
                }
            }
        }
    }
    assert_eq!(snapshot_count, 16);
    assert_eq!(baseline_count, 1);
    // The top roots occur in eight page reads and the main locator. The child root is absent
    // from page default mode and both unnamed JSON reads, and present in all seven child reads.
    assert_eq!(closed_counts, [9, 9, 13]);
    Ok(())
}

const PRIVATE_TRACE_SNAPSHOT_SCRIPT: &str = r"
const p = await context.newPage();
await p.goto(__FIXTURE_URL__);
const child = p.locator('#cross').contentFrame().locator('body');
await child.locator('#cross-closed').waitFor();
// The first page AI walk opens closed roots before any locator enters them.
const snapshots = [
  await p.ariaSnapshot(), await p.ariaSnapshot({ text: true }),
  await p.ariaSnapshot({ text: false }),
  await p.ariaSnapshot({ mode: 'full' }),
  await p.ariaSnapshot({ mode: 'default' }),
  await p.ariaSnapshotJSON(), await p.locator('main').ariaSnapshotJSON(),
];
const childSnapshots = [
  await child.ariaSnapshot(), await child.ariaSnapshot({ text: true }),
  await child.ariaSnapshot({ text: false }),
  await child.ariaSnapshot({ mode: 'full' }),
  await child.ariaSnapshot({ mode: 'default' }),
  await child.ariaSnapshotJSON({ mode: 'ai' }), await child.ariaSnapshotJSON({ mode: 'default' }),
];
const closedRootsOpaque = await Promise.all([
  p.locator('#closed'), p.locator('#closed').locator('#nested'), child.locator('#cross-closed'),
].map(root => root.evaluate(host => host.shadowRoot === null)));
await p.getByLabel('Password', { exact: true }).focus();
const baseline = await p.ariaSnapshot({ text: false });
return { page: p, snapshots, childSnapshots, closedRootsOpaque, baseline };
";

async fn snapshot_trace(rig: &mut Rig) -> TestResult {
    let server = FixtureServer::start(private_trace_pages()?).await?;
    rig.until_ready().await?;
    // Keep the entire fixture through the host's current settings contract. Clipped traces
    // could hide an unmasked field and make the canary scan pass without proving this boundary.
    let configured = rig
        .host(
            "PATCH",
            "/v1/settings",
            Some(HOST_TOKEN),
            Some(&json!({ "trace": { "values": "on", "valueBytes": 262_144 } })),
        )
        .await?;
    assert_eq!(configured.status, 200, "{}", configured.text());
    let client = rig.client().await?;
    // Commit the final v1 read as the baseline, with focus already stable for the next fill.
    // A changed output form or focus state could otherwise legitimately return a whole page.
    let first = run(
        &client,
        &PRIVATE_TRACE_SNAPSHOT_SCRIPT.replace(
            "__FIXTURE_URL__",
            &json!(server.url("127.0.0.1", "/private.html")).to_string(),
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
    assert_trace_canaries_absent(delta);
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

/// Each page has one same-origin child. Keeping shadow children separate from light-DOM
/// children makes their zero `window.length` a real discovery case rather than a spare route.
const ADOPTED_FIELD_CASES: &str =
    include_str!("../../neo-playwright/tests/data/adopted-private-fields/cases.json");

/// The page retains its source frame and closed-root reference, as an application that moves
/// controls would. `srcdoc` inherits the parent's origin even though its URL says about:srcdoc.
/// References live outside the moved controls, so a button-scoped walk must guard the document.
const ADOPTED_FIELD_PAGE: &str =
    include_str!("../../neo-playwright/tests/data/adopted-private-fields/page.html");

/// Warm both native worlds before adoption. Otherwise Chromium may first wrap a field in its
/// destination world and accidentally turn this into an ordinary same-realm masking test.
/// Explicit reads are kept apart from snapshots because those APIs intentionally return clear values.
const ADOPTED_FIELD_SNAPSHOT_SCRIPT: &str =
    include_str!("../../neo-playwright/tests/data/adopted-private-fields/snapshots.js");

/// The next run reads the committed page baseline before changing only a private value's
/// length. The source child remains connected until both delta rendering and clear reads finish.
const ADOPTED_FIELD_DELTA_SCRIPT: &str =
    include_str!("../../neo-playwright/tests/data/adopted-private-fields/delta.js");

fn adopted_field_values(name: &str) -> [String; 4] {
    [
        format!("AdoptedPrivate-{name}-password-value🔒"),
        format!("AdoptedPrivate-{name}-textarea-value🔒"),
        format!("AdoptedPublic-{name}-input-value"),
        format!("AdoptedPublic-{name}-textarea-value"),
    ]
}

fn assert_adopted_canaries_absent(snapshot: &Value) -> TestResult {
    let encoded = serde_json::to_string(snapshot)?;
    assert!(
        !encoded.contains("AdoptedPrivate-"),
        "snapshot exposes an adopted secret: {encoded}"
    );
    Ok(())
}

/// Mixed owner content stays in JSON children. Find the owner by its own static text and
/// the control it owns, so another button or an unrelated visible field cannot satisfy it.
fn adopted_owner_node<'tree>(tree: &'tree Value, label: &str, field: &str) -> Option<&'tree Value> {
    match tree {
        Value::Array(nodes) => nodes
            .iter()
            .find_map(|node| adopted_owner_node(node, label, field)),
        Value::Object(node) => {
            let own_label = node.get("name").and_then(Value::as_str) == Some(label)
                || node.get("text").and_then(Value::as_str) == Some(label)
                || node
                    .get("children")
                    .and_then(Value::as_array)
                    .is_some_and(|children| {
                        children.iter().any(|child| {
                            child.as_str() == Some(label)
                                || (child.get("role").and_then(Value::as_str) == Some("text")
                                    && child.get("text").and_then(Value::as_str) == Some(label))
                        })
                    });
            if node.get("role").and_then(Value::as_str) == Some("button")
                && own_label
                && trace_field_node(tree, field).is_some()
            {
                Some(tree)
            } else {
                node.values()
                    .find_map(|value| adopted_owner_node(value, label, field))
            }
        }
        _ => None,
    }
}

/// External labels read masked values. Ownership instead keeps the owner's static content
/// and the moved field's bare native line. Public twins preserve values in labels and keep
/// the public textarea's default content when its own adopted line is walked.
fn assert_adopted_snapshot(snapshot: &Value, name: &str, scope: &str) -> TestResult {
    assert_adopted_canaries_absent(snapshot)?;
    let values = adopted_field_values(name);
    let password = "•".repeat(values[0].encode_utf16().count());
    let textarea = "•".repeat(values[1].encode_utf16().count());
    for (group, prefix, value) in [
        ("passwordLabel", "Password label", password.as_str()),
        ("textareaLabel", "Textarea label", textarea.as_str()),
        ("public", "Public input label", values[2].as_str()),
        ("public", "Public textarea label", values[3].as_str()),
    ] {
        if scope != "page" && scope != "destination" && scope != group {
            continue;
        }
        let label = format!("{prefix} {value}");
        if let Some(text) = snapshot
            .as_str()
            .or_else(|| trace_field_text(snapshot, &format!("button \"{label}\"")))
        {
            assert!(text.contains(&format!("button \"{label}\"")), "{text}");
        } else {
            let node = trace_field_node(snapshot, &label)
                .ok_or_else(|| format!("missing button {label} in {snapshot}"))?;
            assert_eq!(node["role"], "button", "{node}");
        }
    }
    for (group, owner, field) in [
        ("passwordOwner", "Owned password", "Moved password"),
        ("textareaOwner", "Owned textarea", "Moved textarea"),
        ("public", "Owned public input", "Public moved input"),
        ("public", "Owned public textarea", "Public moved textarea"),
    ] {
        if scope != "page" && scope != "destination" && scope != group {
            continue;
        }
        let public_content = (field == "Public moved textarea")
            .then(|| format!("AdoptedPublic-{name}-textarea-content"));
        if let Some(text) = snapshot
            .as_str()
            .or_else(|| trace_field_text(snapshot, &format!("\"{field}\"")))
        {
            assert!(
                text.contains(owner),
                "missing owner content {owner} in {text}"
            );
            let text = adopted_without_refs(text);
            let line = text
                .lines()
                .find(|line| line.contains(&format!("\"{field}\"")))
                .ok_or_else(|| format!("missing owned field {field} in {text}"))?;
            if let Some(content) = public_content {
                assert!(
                    line.ends_with(&format!(": {content}"))
                        || line.ends_with(&format!(": \"{content}\"")),
                    "{line}"
                );
            } else {
                assert!(
                    line.ends_with(&format!("\"{field}\"")),
                    "moved field is bare: {line}"
                );
            }
        } else {
            let owner_node = adopted_owner_node(snapshot, owner, field)
                .ok_or_else(|| format!("missing owner {owner} with field {field} in {snapshot}"))?;
            let node = trace_field_node(owner_node, field).ok_or("the owner includes its field")?;
            assert_eq!(
                node["role"],
                if field.ends_with("textarea") {
                    "searchbox"
                } else {
                    "textbox"
                }
            );
            if let Some(content) = public_content {
                assert_eq!(node["text"], content, "{node}");
            } else {
                assert!(node.get("text").is_none(), "moved field is bare: {node}");
            }
        }
    }
    Ok(())
}

fn assert_adopted_reads(reads: &Value, name: &str, grown: bool) -> TestResult {
    let mut values = adopted_field_values(name);
    if grown {
        values[0].push('!');
    }
    for read in reads.as_array().ok_or("the reads after the walks")? {
        let main = read.get("main").ok_or("the main-world clear reads")?;
        assert_eq!(read["direct"], json!(&values[..2]));
        assert_eq!(main["values"], json!(values));
        assert_eq!(
            main["defaults"],
            json!([
                format!("AdoptedPrivate-{name}-password-attribute"),
                format!("AdoptedPrivate-{name}-textarea-content"),
            ])
        );
        assert_eq!(
            main["attributes"],
            json!([
                format!("AdoptedPrivate-{name}-password-attribute"),
                format!("AdoptedPrivate-{name}-textarea-attribute"),
            ])
        );
        assert_eq!(
            main["content"],
            format!("AdoptedPrivate-{name}-textarea-content")
        );
        assert_eq!(main["childPresent"], true);
        if name.starts_with("closed-") {
            assert_eq!(main["closedOpaque"], true);
        }
    }
    Ok(())
}

fn assert_adopted_delta(changed: &Value, name: &str) -> TestResult {
    assert_adopted_canaries_absent(&changed["delta"])?;
    let delta = changed["delta"]
        .as_str()
        .ok_or("the changed value's delta")?;
    assert!(delta.starts_with("# changes since"), "{name}: {delta}");
    let old = "•".repeat(adopted_field_values(name)[0].encode_utf16().count());
    for mask in [&old, &format!("{old}•")] {
        assert!(
            delta.contains(&format!("Password label {mask}\"")),
            "{name}: {delta}"
        );
    }
    assert_eq!(changed["reads"].as_array().map(Vec::len), Some(1));
    assert_adopted_reads(&changed["reads"], name, true)
}

/// Returned formats prove the document guard reaches external references, while the raw
/// instance facts keep this a moved-wrapper case and preserve closed-root page semantics.
fn assert_adopted_returned_snapshots(
    opened: &Value,
    root: &str,
    destination: &str,
    name: &str,
) -> TestResult {
    assert_eq!(opened["initial"].as_array().map(Vec::len), Some(2));
    assert_adopted_canaries_absent(&opened["initial"])?;
    assert_adopted_snapshot(&opened["baseline"], name, "page")?;
    assert_eq!(
        opened["snapshots"].as_object().map(serde_json::Map::len),
        Some(6)
    );
    assert_eq!(opened["reads"].as_array().map(Vec::len), Some(6));
    let facts = opened.get("facts").ok_or("the native adoption facts")?;
    assert_eq!(facts["childPresent"], true, "{name}");
    assert_eq!(facts["childURL"], "about:srcdoc", "{name}");
    assert_eq!(facts["frameCount"], i32::from(root == "document"), "{name}");
    assert_eq!(facts["sourcePrototypes"], json!([true, true]), "{name}");
    assert_eq!(
        facts["destinationInstances"],
        json!([false, false]),
        "{name}"
    );
    for (scope, group) in opened["snapshots"]
        .as_object()
        .ok_or("the snapshot scopes")?
    {
        for format in ["texts", "trees"] {
            assert_eq!(
                group[format].as_object().map(serde_json::Map::len),
                Some(if format == "texts" { 6 } else { 2 })
            );
            for (mode, snapshot) in group[format].as_object().ok_or("the snapshot formats")? {
                assert_adopted_canaries_absent(snapshot)?;
                if scope == "page" && destination == "child" && mode == "default" {
                    continue;
                }
                assert_adopted_snapshot(snapshot, name, scope)?;
            }
        }
    }
    assert_adopted_reads(&opened["reads"], name, false)?;
    Ok(())
}

/// Strip only unstable refs before comparing a moved control's bare line. The label and
/// any printed value remain intact, so the assertion still catches accidental value output.
fn adopted_without_refs(text: &str) -> String {
    text.split(" [ref=")
        .enumerate()
        .map(|(index, part)| match (index, part.split_once(']')) {
            (0, _) | (_, None) => part.to_owned(),
            (_, Some((_, rest))) => rest.to_owned(),
        })
        .collect()
}

/// The public run returns rendered forms; the trace holds the same operations' raw trees.
/// Fill arguments and deliberate clear reads may retain canaries, so only snapshot results
/// and their persisted baseline are scanned after the server stops.
#[tokio::test(flavor = "multi_thread")]
async fn adopted_private_fields_never_reach_stored_snapshot_results() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let mut rig = Rig::start(&browser, CaptchaKey::Absent)?;
    match time::timeout(TEST_BUDGET, adopted_snapshot_trace(&mut rig)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(format!("{error}\n{}", rig.logs()).into()),
        Err(_elapsed) => Err(format!("adopted snapshot trace timed out\n{}", rig.logs()).into()),
    }
}

async fn adopted_snapshot_trace(rig: &mut Rig) -> TestResult {
    let server = FixtureServer::start(HashMap::from([(
        "/adopted.html".to_owned(),
        Page::html(ADOPTED_FIELD_PAGE),
    )]))
    .await?;
    rig.until_ready().await?;
    let configured = rig
        .host(
            "PATCH",
            "/v1/settings",
            Some(HOST_TOKEN),
            Some(&json!({ "trace": { "values": "on", "valueBytes": 262_144 } })),
        )
        .await?;
    assert_eq!(configured.status, 200, "{}", configured.text());
    let client = rig.client().await?;
    let mut runs = Vec::new();
    for (root, destination) in serde_json::from_str::<Vec<(&str, &str)>>(ADOPTED_FIELD_CASES)? {
        let name = format!("{root}-{destination}");
        let task = format!("adopted-{name}");
        let url = server.url(
            "127.0.0.1",
            &format!("/adopted.html?root={root}&destination={destination}"),
        );
        let script = format!("const p = await context.newPage();\n{ADOPTED_FIELD_SNAPSHOT_SCRIPT}")
            .replace("__FIXTURE_URL__", &json!(url).to_string())
            .replace("__DESTINATION__", &json!(destination).to_string())
            .replace("__CASE_NAME__", &json!(name).to_string());
        let first = run(
            &client,
            &script,
            &json!({ "requestId": format!("{task}-1"), "taskId": task }),
        )
        .await?;
        let first_id: RunId = outcome(&first)?["id"]
            .as_str()
            .ok_or("the first run id")?
            .parse()?;
        let returned = first
            .structured_content
            .as_ref()
            .and_then(|value| value.get("value"))
            .ok_or_else(|| format!("the run returns its snapshots: {first:?}"))?;
        assert_adopted_returned_snapshots(returned, root, destination, &name)?;
        let changed = run(
            &client,
            &ADOPTED_FIELD_DELTA_SCRIPT.replace("__DESTINATION__", &json!(destination).to_string()),
            &json!({ "requestId": format!("{task}-2"), "taskId": task }),
        )
        .await?;
        let second_id: RunId = outcome(&changed)?["id"]
            .as_str()
            .ok_or("the second run id")?
            .parse()?;
        let returned = changed
            .structured_content
            .as_ref()
            .and_then(|value| value.get("value"))
            .ok_or_else(|| format!("the run returns its delta: {changed:?}"))?;
        assert_adopted_delta(returned, &name)?;
        runs.push((root, destination, name, first_id, second_id));
    }
    client.cancel().await?;
    assert_eq!(rig.stop_neo().await?, Some(0));
    let store = TraceStore::open(&rig.trace_path()).await?;
    for (root, destination, name, first, second) in runs {
        assert_adopted_stored_snapshots(&store, [first, second], root, destination, &name).await?;
    }
    Ok(())
}

/// Snapshot requests run sequentially in the script. Their known scope order pairs each raw
/// stored result with the public request that produced it, including source-world warmups.
/// This keeps pre-adoption fields' valid masked values distinct from post-adoption bare lines.
async fn assert_adopted_stored_snapshots(
    store: &TraceStore,
    ids: [RunId; 2],
    root: &str,
    destination: &str,
    name: &str,
) -> TestResult {
    const SCOPES: [&str; 6] = [
        "page",
        "destination",
        "passwordLabel",
        "textareaLabel",
        "passwordOwner",
        "textareaOwner",
    ];
    let mut snapshot_count = 0;
    let mut baseline_count = 0;
    for (phase, id) in ids.into_iter().enumerate() {
        let detail = store
            .run(id)
            .await?
            .ok_or("the adopted run was persisted")?;
        assert_eq!(detail.summary.status, RunStatus::Ok, "{name}");
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
                !operation.result_truncated,
                "{name}: stored snapshot results are whole"
            );
            let result = operation.result.ok_or("the snapshot has a stored result")?;
            assert_adopted_canaries_absent(&result)?;
            if operation.name == "page.snapshotBaseline" {
                baseline_count += 1;
                assert_adopted_snapshot(&result, name, "page")?;
            }
            if !operation.name.ends_with(".ariaSnapshot") {
                continue;
            }
            if phase == 0 && (2..50).contains(&snapshot_count) {
                let scope = SCOPES
                    .get((snapshot_count - 2) / 8)
                    .copied()
                    .ok_or("the snapshot scope index")?;
                let format = (snapshot_count - 2) % 8;
                // Default mode stays in one frame. AI and full page reads stitch the child.
                if !(scope == "page" && destination == "child" && matches!(format, 5 | 7)) {
                    assert_adopted_snapshot(&result, name, scope)?;
                }
            } else if phase == 0 && snapshot_count == 50 {
                assert_adopted_snapshot(&result, name, "page")?;
            }
            snapshot_count += 1;
        }
    }
    assert_eq!(
        snapshot_count, 52,
        "{root}-{destination}: every requested snapshot was stored"
    );
    assert_eq!(
        baseline_count, 1,
        "{name}: the later run read its persisted baseline"
    );
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
