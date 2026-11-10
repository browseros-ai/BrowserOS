//! THROWAWAY spike (proto-hook-pin): where does a hook-substituted fill value land when the page
//! changes while the stand-in hook holds the call? Prints one `CELL {...}` JSON line per
//! (scenario, variant). Asserts nothing about the outcome: the cells are the observation.
//!
//! Run: NEO_E2E_BROWSER=... NEO_E2E_HEADED=0 cargo test -p neo-runtime --test proto_pin -- \
//!      --test-threads=1 --nocapture
#![allow(clippy::all, clippy::pedantic, missing_docs)]

#[allow(dead_code)]
#[path = "support/browser.rs"]
mod browser;

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use browser::{Fixture, Tab};
use neo_runtime::RunOutcome;
use neo_runtime::proto_pin;
use serde_json::{Value, json};
use tokio::time;

const HOLD_MS: u64 = 2000;
const VARIANTS: [(u8, &str); 5] = [(0, "V0"), (1, "VS"), (2, "VP"), (3, "VG"), (4, "VPX")];

/// Records every `input` event into the DOM, so an isolated-world oracle can read it.
const LOG: &str = "<script>document.addEventListener('input', e => { const d = document.documentElement.dataset; d.log = (d.log || '') + e.target.id + ':' + e.target.type + ':' + e.target.value + ':' + e.inputType + ':trusted=' + e.isTrusted + ';' }, true)</script>";

fn pages() -> Vec<(String, String)> {
    let page = |path: &str, body: &str| (path.to_string(), format!("<title>{path}</title>{LOG}{body}"));
    vec![
        page("/bank", "<h1>bank</h1><input id=pw type=password>"),
        page("/evil", "<h1>evil</h1><input id=pw type=password>"),
        page("/evil-autofocus", "<h1>evil-autofocus</h1><input id=other type=text autofocus>"),
        page("/frame-a", "<h1>frame-a</h1><input id=pw type=password>"),
        page("/frame-b", "<h1>frame-b</h1><input id=pw type=password><p id=marker-b>b</p>"),
        page("/host-same", "<h1>host-same</h1><iframe src=/frame-a></iframe>"),
        page(
            "/host-oopif",
            "<h1>host-oopif</h1><iframe></iframe><script>document.querySelector('iframe').src = 'http://localhost:' + location.port + '/frame-a'</script>",
        ),
        page(
            "/host-two",
            "<h1>host-two</h1><iframe class=target src=/frame-a></iframe><iframe src=/frame-b></iframe>",
        ),
    ]
}

const DUMP: &str = "JSON.stringify({url: location.href, inputs: [...document.querySelectorAll('input')].map(i => ({id: i.id, type: i.type, value: i.value})), log: document.documentElement.dataset.log || ''})";

/// Every document the browser has, read over the test's own CDP sessions: the tab's frames
/// (in an isolated world each), then every out-of-process iframe target.
async fn oracle(tab: &Tab) -> Vec<Value> {
    let mut found = Vec::new();
    let tree = tab
        .cdp
        .send("Page.getFrameTree", json!({}), Some(&tab.driver))
        .await
        .unwrap_or_else(|error| panic!("getFrameTree: {error}"));
    let mut stack = vec![tree["frameTree"].clone()];
    while let Some(node) = stack.pop() {
        if let Some(children) = node["childFrames"].as_array() {
            stack.extend(children.iter().cloned());
        }
        let id = node["frame"]["id"].as_str().unwrap_or_default().to_string();
        let world = tab
            .cdp
            .send(
                "Page.createIsolatedWorld",
                json!({"frameId": id, "worldName": "proto-oracle"}),
                Some(&tab.driver),
            )
            .await;
        let Ok(world) = world else {
            found.push(json!({"frame": id, "error": "no world"}));
            continue;
        };
        let reply = tab
            .cdp
            .send(
                "Runtime.evaluate",
                json!({"expression": DUMP, "contextId": world["executionContextId"], "returnByValue": true}),
                Some(&tab.driver),
            )
            .await;
        found.push(parse_dump(reply.ok()));
    }
    let targets = tab
        .cdp
        .send("Target.getTargets", json!({}), None)
        .await
        .unwrap_or_else(|error| panic!("getTargets: {error}"));
    for target in targets["targetInfos"].as_array().into_iter().flatten() {
        if target["type"] != "iframe" {
            continue;
        }
        let Ok(attached) = tab
            .cdp
            .send(
                "Target.attachToTarget",
                json!({"targetId": target["targetId"], "flatten": true}),
                None,
            )
            .await
        else {
            continue;
        };
        let session = browseros_cdp::SessionId::from(attached["sessionId"].as_str().unwrap_or_default());
        let reply = tab
            .cdp
            .send(
                "Runtime.evaluate",
                json!({"expression": DUMP, "returnByValue": true}),
                Some(&session),
            )
            .await;
        let mut dump = parse_dump(reply.ok());
        dump["oopif"] = json!(true);
        found.push(dump);
        let _ = tab
            .cdp
            .send(
                "Target.detachFromTarget",
                json!({"sessionId": session.as_str()}),
                None,
            )
            .await;
    }
    found
}

fn parse_dump(reply: Option<Value>) -> Value {
    reply
        .and_then(|reply| reply["result"]["value"].as_str().map(str::to_string))
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({"error": "unreadable"}))
}

/// `path#id(type)` of every input holding the secret, with the port elided.
fn landed(dumps: &[Value]) -> Vec<String> {
    let mut out = Vec::new();
    for dump in dumps {
        let url = dump["url"].as_str().unwrap_or("?");
        let short = url
            .replace("http://127.0.0.1:", "127:")
            .replace("http://localhost:", "localhost:");
        let short = short
            .split_once('/')
            .map_or(short.clone(), |(host, path)| {
                let origin = host.split(':').next().unwrap_or(host);
                format!("{origin}/{path}")
            });
        for input in dump["inputs"].as_array().into_iter().flatten() {
            if input["value"] == proto_pin::SECRET {
                out.push(format!(
                    "{short}#{}({}){}",
                    input["id"].as_str().unwrap_or(""),
                    input["type"].as_str().unwrap_or(""),
                    if dump["oopif"] == true { "[oopif]" } else { "" }
                ));
            }
        }
    }
    out
}

struct Scenario {
    name: &'static str,
    start: &'static str,
    script: &'static str,
    after_focus: bool,
}

const FILL_THEN: &str = "const p = TARGET.fill('{{password}}').then(() => 'ok', e => 'threw: ' + e.message.split('\\n')[0]);\nawait page.waitForFunction(() => window.__held === 1, undefined, { polling: 10 });\n";

fn script(target: &str, mutate: &str) -> String {
    format!(
        "{}{mutate}\nconst mutated = Date.now();\nreturn {{ r: await p, mutated }};",
        FILL_THEN.replace("TARGET", target)
    )
}

async fn run_cell(tab: &Tab, scenario: &Scenario, variant: (u8, &str)) -> Value {
    tab.navigate(scenario.start).await;
    // Let child frames load before the run starts.
    time::sleep(Duration::from_millis(300)).await;
    proto_pin::VARIANT.store(variant.0, Ordering::SeqCst);
    if scenario.after_focus {
        proto_pin::HOLD_MS.store(0, Ordering::SeqCst);
        proto_pin::AFTER_FOCUS_HOLD_MS.store(HOLD_MS, Ordering::SeqCst);
    } else {
        proto_pin::HOLD_MS.store(HOLD_MS, Ordering::SeqCst);
        proto_pin::AFTER_FOCUS_HOLD_MS.store(0, Ordering::SeqCst);
    }
    proto_pin::HELD.store(0, Ordering::SeqCst);
    proto_pin::AFTER_FOCUS_HELD.store(0, Ordering::SeqCst);
    proto_pin::ENABLED.store(true, Ordering::SeqCst);
    let code = scenario.script.to_string();
    let running = tab.start_run(code).await;
    let counter = if scenario.after_focus {
        &proto_pin::AFTER_FOCUS_HELD
    } else {
        &proto_pin::HELD
    };
    let waited = Instant::now();
    let mut gated = false;
    while waited.elapsed() < Duration::from_secs(15) {
        if counter.load(Ordering::SeqCst) > 0 {
            gated = true;
            break;
        }
        if running.is_finished() {
            break;
        }
        time::sleep(Duration::from_millis(5)).await;
    }
    let held_at = now_ms();
    if gated {
        tab.evaluate("window.__held = 1").await;
    }
    let outcome: RunOutcome = running.await.expect("run task");
    proto_pin::ENABLED.store(false, Ordering::SeqCst);
    let dumps = oracle(tab).await;
    let fill = outcome
        .calls
        .iter()
        .find(|call| call.method == "fill")
        .map(|call| {
            json!({
                "result": format!("{:?}", call.result),
                "ms": call.duration.as_millis() as u64,
                "started": call.started_at_ms,
                "delivery": format!("{:?}", call.delivery),
                "log": call.log,
            })
        });
    let mutated = outcome.value.as_ref().and_then(|value| value["mutated"].as_u64());
    let cell = json!({
        "scenario": scenario.name,
        "variant": variant.1,
        "landed": landed(&dumps),
        "script": outcome.value.as_ref().map(|value| value["r"].clone()),
        "runStatus": format!("{:?}", outcome.status),
        "runError": outcome.error.as_ref().map(|error| error.message.clone()),
        "gated": gated,
        "heldAtMs": held_at,
        "mutatedAtMs": mutated,
        "fill": fill,
        "dumps": dumps,
    });
    println!("CELL {cell}");
    cell
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                pages: pages(),
                ..Fixture::default()
            },
            "/bank",
        )
        .await,
    )
}

async fn run_scenario(scenario: Scenario, variants: &[(u8, &str)]) {
    let Some(tab) = open().await else { return };
    for &variant in variants {
        run_cell(&tab, &scenario, variant).await;
    }
    tab.close().await;
}

fn variants_from_env() -> Vec<(u8, &'static str)> {
    match std::env::var("PROTO_VARIANTS") {
        Ok(list) => VARIANTS
            .iter()
            .copied()
            .filter(|(_, name)| list.split(',').any(|wanted| wanted == *name))
            .collect(),
        Err(_) => VARIANTS.to_vec(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn s1_main_frame_navigation() {
    run_scenario(
        Scenario {
            name: "s1-main-nav",
            start: "/bank",
            script: Box::leak(
                script(
                    "page.locator('#pw')",
                    "await page.goto(page.url().replace('/bank', '/evil'));",
                )
                .into_boxed_str(),
            ),
            after_focus: false,
        },
        &variants_from_env(),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn s1b_navigation_between_focus_and_insert_text() {
    run_scenario(
        Scenario {
            name: "s1b-nav-after-focus",
            start: "/bank",
            script: Box::leak(
                script(
                    "page.locator('#pw')",
                    "await page.goto(page.url().replace('/bank', '/evil-autofocus'));",
                )
                .into_boxed_str(),
            ),
            after_focus: true,
        },
        &variants_from_env(),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn s2_same_document_swap() {
    run_scenario(
        Scenario {
            name: "s2-swap",
            start: "/bank",
            script: Box::leak(
                script(
                    "page.locator('#pw')",
                    "await page.evaluate(() => { const o = document.querySelector('#pw'); const n = document.createElement('input'); n.id = 'pw'; n.type = 'text'; o.replaceWith(n); });",
                )
                .into_boxed_str(),
            ),
            after_focus: false,
        },
        &variants_from_env(),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn s3a_same_origin_child_frame_navigation() {
    run_scenario(
        Scenario {
            name: "s3a-child-nav-same-origin",
            start: "/host-same",
            script: Box::leak(
                script(
                    "page.frameLocator('iframe').locator('#pw')",
                    "await page.evaluate(() => { const f = document.querySelector('iframe'); f.src = new URL('/frame-b', f.src).href; });\nawait page.frameLocator('iframe').locator('#marker-b').waitFor();",
                )
                .into_boxed_str(),
            ),
            after_focus: false,
        },
        &variants_from_env(),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn s3b_oopif_navigation_within_its_site() {
    run_scenario(
        Scenario {
            name: "s3b-oopif-nav-same-site",
            start: "/host-oopif",
            script: Box::leak(
                script(
                    "page.frameLocator('iframe').locator('#pw')",
                    "await page.evaluate(() => { const f = document.querySelector('iframe'); f.src = new URL('/frame-b', f.src).href; });\nawait page.frameLocator('iframe').locator('#marker-b').waitFor();",
                )
                .into_boxed_str(),
            ),
            after_focus: false,
        },
        &variants_from_env(),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn s3c_oopif_navigation_back_to_the_parent_site() {
    run_scenario(
        Scenario {
            name: "s3c-oopif-nav-to-parent-site",
            start: "/host-oopif",
            script: Box::leak(
                script(
                    "page.frameLocator('iframe').locator('#pw')",
                    "await page.evaluate(() => { document.querySelector('iframe').src = location.origin + '/frame-b'; });\nawait page.frameLocator('iframe').locator('#marker-b').waitFor();",
                )
                .into_boxed_str(),
            ),
            after_focus: false,
        },
        &variants_from_env(),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn s4_selector_moves_between_frames() {
    run_scenario(
        Scenario {
            name: "s4-selector-moves-frames",
            start: "/host-two",
            script: Box::leak(
                script(
                    "page.frameLocator('iframe.target').locator('#pw')",
                    "await page.evaluate(() => { const [a, b] = document.querySelectorAll('iframe'); a.classList.remove('target'); b.classList.add('target'); });",
                )
                .into_boxed_str(),
            ),
            after_focus: false,
        },
        &variants_from_env(),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn s5_nothing_changes() {
    run_scenario(
        Scenario {
            name: "s5-control",
            start: "/bank",
            script: Box::leak(script("page.locator('#pw')", "").into_boxed_str()),
            after_focus: false,
        },
        &variants_from_env(),
    )
    .await;
}

/// Cost: 50 fills in the control with no hold, per variant, plus today's fill with an ordinary
/// value (no proto code at all). Per-call durations from the call records; CDP commands from the
/// runtime's send tally, a 51-fill run minus a 1-fill run.
#[tokio::test(flavor = "multi_thread")]
async fn cost_fifty_fills() {
    let Some(tab) = open().await else { return };
    let mut rows = vec![(255_u8, "baseline")];
    rows.extend(variants_from_env());
    for (variant, name) in rows {
        let value = if variant == 255 { "plain" } else { "{{password}}" };
        proto_pin::VARIANT.store(variant.min(4), Ordering::SeqCst);
        proto_pin::HOLD_MS.store(0, Ordering::SeqCst);
        proto_pin::AFTER_FOCUS_HOLD_MS.store(0, Ordering::SeqCst);
        proto_pin::ENABLED.store(variant != 255, Ordering::SeqCst);
        let mut tallies = Vec::new();
        let mut durations = Vec::new();
        for fills in [1, 51] {
            tab.navigate("/bank").await;
            proto_pin::take_sends();
            let code = format!(
                "for (let i = 0; i < {fills}; i++) await page.fill('#pw', '{value}'); return await page.inputValue('#pw')"
            );
            let outcome = tab.start_run(code).await.await.expect("run task");
            let sends = proto_pin::take_sends();
            assert_eq!(
                format!("{:?}", outcome.status),
                "Ok",
                "{name}: {:?}",
                outcome.error
            );
            tallies.push(sends);
            if fills == 51 {
                durations = outcome
                    .calls
                    .iter()
                    .filter(|call| call.method == "fill")
                    .skip(1)
                    .map(|call| call.duration.as_secs_f64() * 1000.0)
                    .collect();
            }
        }
        let mut per_fill = serde_json::Map::new();
        let mut total = 0.0;
        for (method, count) in &tallies[1] {
            let one = tallies[0].get(method).copied().unwrap_or(0);
            let extra = (*count as f64 - one as f64) / 50.0;
            if extra.abs() > 0.001 {
                per_fill.insert(method.clone(), json!((extra * 100.0).round() / 100.0));
                total += extra;
            }
        }
        durations.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pick = |q: f64| durations[((durations.len() - 1) as f64 * q).round() as usize];
        let row = json!({
            "variant": name,
            "n": durations.len(),
            "medianMs": (pick(0.5) * 10.0).round() / 10.0,
            "p10Ms": (pick(0.1) * 10.0).round() / 10.0,
            "p90Ms": (pick(0.9) * 10.0).round() / 10.0,
            "minMs": (durations[0] * 10.0).round() / 10.0,
            "maxMs": (durations[durations.len() - 1] * 10.0).round() / 10.0,
            "cdpPerFill": (total * 100.0).round() / 100.0,
            "cdpPerFillByMethod": per_fill,
        });
        println!("COST {row}");
    }
    proto_pin::ENABLED.store(false, Ordering::SeqCst);
    tab.close().await;
}
