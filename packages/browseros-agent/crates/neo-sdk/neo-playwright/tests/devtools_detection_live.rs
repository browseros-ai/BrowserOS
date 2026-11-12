//! Whether a tab Neo drives can tell an inspector is attached, against a real browser.
//!
//! The fixture runs the checks sites use to spot an open inspector (core's
//! `devtools_detection_live` detector: `console.table` timing, getters on logged values, a
//! `debugger` pause) in three places: the top document, a cross-site iframe (an OOPIF, with its
//! own CDP session) and a dedicated worker. The test drives the tab through the real prelude and
//! `Playwright::invoke` in phases (goto, snapshots, reads, a click, a frame read, a
//! `worker.evaluate`, then a console listener) and prints the detector counts and the
//! `Runtime.enable` commands Neo sent after each phase, so a detection can be traced to the
//! phase, and session kind, that caused it. Until the script asks for console events, no session
//! (page, OOPIF or worker) may get `Runtime.enable`, and no place may detect anything.
//!
//! Needs BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005):
//! `cargo test -p neo-playwright --test devtools_detection_live -- --ignored --nocapture`.

#[path = "support/fixture_server.rs"]
#[expect(dead_code, reason = "the nested fixture serves other suites")]
mod fixture_server;
#[path = "support/js_runner.rs"]
#[expect(
    dead_code,
    reason = "the shared runner also serves selector and event tests"
)]
mod js_runner;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "other live suites use the shared action and policy helpers"
)]
mod live_run;

use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use fixture_server::{FixtureServer, Page};
use js_runner::OpHost;
use live_run::{LiveRun, TestPage, TestResult, brand};
use serde_json::{Value, json};
use tokio::time;

/// Core's detector, as a function each place runs with its own way to report.
const DETECTOR: &str = r"
function detector(report, withDom) {
  const det = { hits: {}, rounds: 0 };
  const hit = (name) => { det.hits[name] = (det.hits[name] || 0) + 1; };
  const big = Array.from({ length: 50 }, (_, i) => {
    const row = {};
    for (let k = 0; k < 400; k++) row['k' + k] = 'v' + i + '_' + k;
    return row;
  });
  setInterval(() => {
    det.rounds++;
    const t0 = performance.now();
    console.table(big);
    const tableMs = performance.now() - t0;
    const t1 = performance.now();
    console.log(big);
    const logMs = performance.now() - t1;
    if (tableMs > 5 || logMs > 5) hit('consoleTiming');
    // Load on a shared machine makes a stray 5 ms round; serialization by an attached
    // inspector makes every round slow, by tens of milliseconds.
    const ms = Math.max(tableMs, logMs);
    det.maxMs = Math.max(det.maxMs || 0, Math.round(ms));
    if (ms > 20) det.over20 = (det.over20 || 0) + 1;
    const err = new Error('probe');
    Object.defineProperty(err, 'stack', { get() { hit('errorStackGetter'); return ''; } });
    console.debug(err);
    if (withDom) {
      const div = document.createElement('div');
      Object.defineProperty(div, 'id', { get() { hit('elementIdGetter'); return 'x'; } });
      console.debug(div);
    }
    const t2 = performance.now();
    debugger;
    if (performance.now() - t2 > 50) hit('debuggerPause');
    report(det);
  }, 100);
  return det;
}
";

/// The top document on `127.0.0.1`; its iframe loads from `localhost` on the same port, another
/// site, so Chromium gives it its own process and Neo its own session.
fn top_html() -> String {
    format!(
        r#"<!doctype html><title>detector</title>
<p id="text">Resume</p><button id="save">Save</button>
<iframe id="child" title="child"></iframe>
<script>
document.getElementById('child').src = `http://localhost:${{location.port}}/child.html`;
{DETECTOR}
window.__det = detector(() => {{}}, true);
window.__frameDet = null;
window.__workerDet = null;
addEventListener('message', (event) => {{
  if (event.data && event.data.frameDet) window.__frameDet = event.data.frameDet;
}});
const source = detector.toString() + "\ndetector((det) => postMessage(JSON.parse(JSON.stringify(det))), false);";
const worker = new Worker(URL.createObjectURL(new Blob([source], {{ type: 'text/javascript' }})));
worker.onmessage = (event) => {{ window.__workerDet = event.data; }};
document.getElementById('save').addEventListener('click', () => console.error('save failed: validation'));
</script>"#
    )
}

fn child_html() -> String {
    format!(
        r#"<!doctype html><title>child</title><p id="inner">Inner text</p>
<script>
{DETECTOR}
detector((det) => parent.postMessage({{ frameDet: JSON.parse(JSON.stringify(det)) }}, '*'), true);
</script>"#
    )
}

/// The three detectors' state, read in the top document's own world.
async fn detections(run: &LiveRun, page: &TestPage) -> TestResult<Value> {
    run.eval(
        page,
        "({ main: JSON.parse(JSON.stringify(window.__det)), frame: window.__frameDet, \
         worker: window.__workerDet })",
    )
    .await
}

async fn script(run: &Arc<LiveRun>, page: &TestPage, body: &str) -> TestResult<Value> {
    let host: Arc<dyn OpHost> = Arc::clone(run) as Arc<dyn OpHost>;
    Ok(js_runner::run_script(&host, &json!({ "page": brand(page) }), body).await?)
}

#[tokio::test]
#[ignore = "needs an owned BrowserOS on BROWSEROS_CDP_PORT"]
#[expect(
    clippy::print_stderr,
    reason = "the per-phase counts are the measurement"
)]
async fn reports_what_a_neo_driven_tab_detects_per_phase() -> TestResult {
    let mut pages = HashMap::new();
    pages.insert("/top.html".to_owned(), Page::html(&top_html()));
    pages.insert("/child.html".to_owned(), Page::html(&child_html()));
    let server = FixtureServer::start(pages).await?;
    let top_url = server.url("127.0.0.1", "/top.html");

    let run = Arc::new(LiveRun::connect().await?);
    // The `worker.evaluate` phase finds the detector's worker through `page.workers()`.
    run.report_workers();
    let page = run.new_page().await?;
    let mut report = Vec::new();
    let phases = phases(&top_url);
    let outcome = async {
        for (name, body) in phases {
            run.record_methods();
            let value = script(&run, &page, &body).await;
            time::sleep(Duration::from_millis(1500)).await;
            let enables = run
                .recorded_methods()
                .iter()
                .filter(|method| method.as_str() == "Runtime.enable")
                .count();
            let seen = detections(&run, &page).await?;
            let line = json!({ "phase": name, "result": value.map_err(|e| e.to_string()),
                               "runtimeEnables": enables, "detections": seen });
            eprintln!("DETECTION {line}");
            report.push(line);
        }
        Ok::<_, Box<dyn Error>>(())
    }
    .await;
    run.close_page(&page).await;
    drop(server);
    outcome?;
    judge(&report)
}

/// The phases, in order, each a script body run on the fixture's tab.
fn phases(top_url: &str) -> [(&'static str, String); 8] {
    // Counts are cumulative from the page's load. `idle` is the control: the tab sits with no
    // op running. `console listener` comes last, because asking for console events is what
    // turns reporting on for the page and its worker (Runtime leases), so its rounds are
    // expected to be slow.
    [
        (
            "goto",
            format!("await page.goto({top_url:?}); return page.url();"),
        ),
        ("idle", "return true;".to_owned()),
        (
            "snapshots",
            "for (let i = 0; i < 5; i++) await page.ariaSnapshot({ mode: 'ai' }); return true;"
                .to_owned(),
        ),
        (
            "reads",
            "for (let i = 0; i < 5; i++) await page.locator('#text').textContent(); return true;"
                .to_owned(),
        ),
        (
            "click",
            "await page.locator('#save').click(); return true;".to_owned(),
        ),
        (
            "frame read",
            "return await page.frameLocator('#child').locator('#inner').textContent();".to_owned(),
        ),
        (
            "worker evaluate",
            "const [worker] = page.workers(); let sum = 0; \
             for (let i = 0; i < 5; i++) sum += await worker.evaluate(() => 1); return sum;"
                .to_owned(),
        ),
        (
            "console listener",
            "const seen = []; page.on('console', (message) => seen.push(message.text())); \
             await page.locator('#save').click(); await page.waitForTimeout(2000); \
             return seen.filter((text) => text.startsWith('save failed')).length;"
                .to_owned(),
        ),
    ]
}

/// Fails unless every phase before the console listener left every place quiet; prints what the
/// listener phase cost.
#[expect(
    clippy::print_stderr,
    reason = "the listener's cost is the measurement"
)]
fn judge(report: &[Value]) -> TestResult {
    // One number of one place in one phase's line, 0 when absent.
    let count = |line: &Value, place: &str, key: &str| {
        line.pointer(&format!("/detections/{place}/{key}"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let phase = |line: &Value| line.get("phase").cloned().unwrap_or(Value::Null);
    // The detector's 5 ms threshold also counts a round the shared machine stalled; reporting
    // makes every round slow and fires the getters. Judge each place by that signature, over
    // every phase before the script asked for console events.
    let quiet: Vec<&Value> = report
        .iter()
        .take_while(|line| phase(line) != "console listener")
        .collect();
    for line in report {
        assert!(
            line.pointer("/result/Ok").is_some(),
            "every phase's script ran: {line}"
        );
    }
    let worker_evaluate = report
        .iter()
        .find(|line| phase(line) == "worker evaluate")
        .and_then(|line| line.pointer("/result/Ok"));
    assert_eq!(
        worker_evaluate,
        Some(&json!(5)),
        "five worker.evaluate calls answered: {report:?}"
    );
    for line in &quiet {
        assert_eq!(
            line.get("runtimeEnables"),
            Some(&json!(0)),
            "no session gets Runtime.enable before a listener asks: {line}"
        );
    }
    let last_quiet = quiet.last().ok_or("no phase ran")?;
    for place in ["main", "frame", "worker"] {
        let hits = last_quiet.pointer(&format!("/detections/{place}/hits"));
        for getter in ["errorStackGetter", "elementIdGetter", "debuggerPause"] {
            assert!(
                hits.and_then(|hits| hits.get(getter)).is_none(),
                "the {place} fired {getter}: {report:?}"
            );
        }
        let rounds = count(last_quiet, place, "rounds");
        let slow = count(last_quiet, place, "over20");
        assert!(
            rounds >= 20,
            "the {place}'s detector barely ran: {report:?}"
        );
        assert!(
            slow * 10 < rounds,
            "the {place}'s console was slow on {slow} of {rounds} rounds: {report:?}"
        );
    }
    // What asking for console events costs, per place: the listener phase's own rounds.
    if let Some(listening) = report
        .last()
        .filter(|line| phase(line) == "console listener")
    {
        let mut cost = serde_json::Map::new();
        for place in ["main", "frame", "worker"] {
            let delta = |key: &str| {
                count(listening, place, key).saturating_sub(count(last_quiet, place, key))
            };
            let hits = listening
                .pointer(&format!("/detections/{place}/hits"))
                .cloned()
                .unwrap_or(Value::Null);
            cost.insert(
                place.to_owned(),
                json!({ "rounds": delta("rounds"), "over20": delta("over20"), "hits": hits }),
            );
        }
        eprintln!("DETECTION listener cost {}", Value::Object(cost));
    }
    Ok(())
}
