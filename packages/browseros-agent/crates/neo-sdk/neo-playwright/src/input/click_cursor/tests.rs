//! Cursor paint, policy checks and real device events through the public operation routes.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use browseros_core::screenshot::{ScreenshotCaptureOptions, ScreenshotFormat};

use neo_trace::ErrorOwner;
use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio::time;

use crate::test_env::EnvCall;
use crate::test_support::{ProbeAnswer, ScriptedRun, ScriptedTab, TabScript, TestResult};
use crate::{ErrorCode, Op, PlaywrightError, Refusal};

#[derive(Default)]
struct ClickPage {
    slow_expression: Option<&'static str>,
    evaluating: Notify,
    capturing: Notify,
    capture_delay: Duration,
    skipped: bool,
    covered: bool,
    armed: AtomicBool,
    /// Pixels the target moves each time the cursor glides.
    step: u32,
    shift: AtomicU32,
    events: Mutex<Vec<String>>,
}

impl ClickPage {
    fn record(&self, event: String) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    fn events(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl TabScript for ClickPage {
    fn probe(&self, _frame: &str, request: &Value) -> ProbeAnswer {
        let x = 10 + self.shift.load(Ordering::Relaxed);
        let point = json!({ "x": x, "y": 20 });
        let rect = json!({ "x": x, "y": 20, "width": 10, "height": 10 });
        let reply = match request["op"].as_str() {
            Some("act") if request["arm"].is_object() => {
                self.record("arm".to_owned());
                if self.covered {
                    json!({ "kind": "covered", "by": "<div>" })
                } else {
                    self.armed.store(true, Ordering::Relaxed);
                    json!({ "kind": "armed", "point": point, "rect": rect, "token": request["arm"]["token"] })
                }
            }
            Some("act") => {
                self.record("measure".to_owned());
                json!({ "kind": "measured", "pin": 1, "point": point, "rect": rect, "inView": true, "transformed": false })
            }
            Some("verdict") => {
                self.armed.store(false, Ordering::Relaxed);
                json!({ "kind": "verdict", "hit": "done" })
            }
            Some("resolve") => json!({ "kind": "element", "pin": 1, "preview": "<input>" }),
            Some("read") => json!({ "kind": "read", "value": {
                "checked": self.events().iter().any(|e| e.starts_with("mousePressed")),
                "radio": false
            } }),
            Some("disarm") => json!({ "kind": "done" }),
            _ => return ProbeAnswer::unexpected(),
        };
        ProbeAnswer::reply(reply)
    }

    fn delay(&self, method: &str, params: &Value) -> Duration {
        if method == "Page.captureScreenshot" {
            self.capturing.notify_one();
            return self.capture_delay;
        }
        if method == "Runtime.evaluate"
            && let (Some(fragment), Some(js)) =
                (self.slow_expression, params["expression"].as_str())
            && js.contains(fragment)
        {
            self.evaluating.notify_one();
            return Duration::from_millis(100);
        }
        Duration::ZERO
    }

    fn cdp_reply(&self, method: &str, params: &Value) -> Option<Value> {
        if method == "Page.captureScreenshot" {
            self.record("capture".to_owned());
            return Some(json!({"data": "AQID"}));
        }
        if method == "Input.dispatchMouseEvent" {
            self.record(format!(
                "{} {} {}",
                params["type"].as_str()?,
                params["x"].as_f64()?,
                params["clickCount"].as_u64().unwrap_or(0)
            ));
        }
        if method != "Runtime.evaluate" {
            return None;
        }
        let expression = params["expression"].as_str()?;
        if !expression.contains("Symbol.for('browseros.agentPointer')") {
            return None;
        }
        if expression.contains("p.hide()") {
            self.record("hide absent".to_owned());
            return Some(json!({"result": {"value": false}}));
        }
        if let Some((_, command)) = expression.trim().rsplit_once("\n})(") {
            let command: Value = serde_json::from_str(command.strip_suffix(')')?).ok()?;
            assert!(
                !self.armed.load(Ordering::Relaxed),
                "input was intercepted during travel"
            );
            self.record(format!("glide {}", command["x"].as_f64()?));
            self.shift.fetch_add(self.step, Ordering::Relaxed);
            let status = if self.skipped { "skipped" } else { "arrived" };
            return Some(json!({ "result": { "value": { "status": status } } }));
        }
        let (_, coords) = expression.split_once("p.click(")?;
        let (coords, _) = coords.split_once(')')?;
        self.record(format!("ripple {coords}"));
        Some(json!({ "result": { "value": true } }))
    }
}

async fn wait_for_request(request: &Notify) {
    assert!(
        time::timeout(Duration::from_secs(5), request.notified())
            .await
            .is_ok(),
        "the expected protocol request never arrived"
    );
}

async fn invoke(run: &ScriptedRun, name: &str, fields: Value) -> Result<Value, PlaywrightError> {
    let mut args = if name.starts_with("mouse.") {
        json!({ "page": run.page, "x": 10, "y": 20 })
    } else {
        json!({ "target": { "$neo": "locator", "page": run.page, "frame": null,
            "chunks": [{ "parts": [{ "name": "css", "body": "input", "source": "input" }] }],
            "display": "locator('input')" }, "strict": "strict", "scroll": "none" })
    };
    args["call"] = json!({ "timeoutMs": 5_000 });
    for (key, value) in fields.as_object().into_iter().flatten() {
        args[key] = value.clone();
    }
    let Some(op) = Op::from_name(name) else {
        panic!("missing operation {name}")
    };
    run.playwright.invoke(&run.env, op, args).await
}

#[tokio::test]
async fn clicks_add_only_a_glide_remeasurement_and_ripple_to_the_burst() -> TestResult {
    for (op, double) in [
        ("locator.click", false),
        ("locator.dblclick", true),
        ("mouse.click", false),
        ("mouse.click", true),
    ] {
        for enabled in [false, true] {
            let tab = ScriptedTab::new(ClickPage::default());
            let mut run = ScriptedRun::over(&tab).await?;
            run.env.draws_cursor = enabled;
            let raw = op == "mouse.click";
            let options = if raw && double {
                json!({"clickCount": 2})
            } else {
                json!({})
            };
            invoke(&run, op, options).await?;
            let mut expected = Vec::new();
            if !raw {
                expected.extend(["measure", "arm"]);
            }
            if enabled {
                expected.push("glide 10");
                if !raw {
                    expected.extend(["measure", "arm"]);
                }
                expected.push("ripple 10,20");
            }
            expected.extend(["mouseMoved 10 0", "mousePressed 10 1", "mouseReleased 10 1"]);
            if double {
                expected.extend(["mousePressed 10 2", "mouseReleased 10 2"]);
            }
            assert_eq!(
                tab.script().events(),
                expected,
                "{op} enabled={enabled} double={double}"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn a_takeover_before_either_paint_or_input_refuses_the_click() -> TestResult {
    for op in ["locator.click", "mouse.click"] {
        for passing in [0, 1, 2] {
            let tab = ScriptedTab::new(ClickPage::default());
            let mut run = ScriptedRun::over(&tab).await?;
            run.env.draws_cursor = true;
            run.env = run.env.with_refusal_after(
                passing,
                Refusal {
                    owner: ErrorOwner::Host,
                    code: "owner_has_control".to_owned(),
                    message: "tab is held".to_owned(),
                    host_code: None,
                },
            );
            let error = invoke(&run, op, json!({}))
                .await
                .err()
                .ok_or("takeover must refuse")?;
            assert_eq!(error.code(), "owner_has_control");
            let effects: Vec<_> = tab
                .script()
                .events()
                .into_iter()
                .filter(|event| event != "arm" && event != "measure")
                .collect();
            assert_eq!(effects, ["glide 10", "ripple 10,20"][..passing]);
            assert!(!run.env.calls().contains(&EnvCall::MarkSent));
        }
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn the_owner_can_move_the_mouse_during_a_locator_glide() -> TestResult {
    let tab = ScriptedTab::new(ClickPage {
        slow_expression: Some("var BrowserOSAgentCursor"),
        ..Default::default()
    });
    let mut run = ScriptedRun::over(&tab).await?;
    run.env.draws_cursor = true;
    let (click, moved) = tokio::join!(invoke(&run, "locator.click", json!({})), async {
        wait_for_request(&tab.script().evaluating).await;
        invoke(&run, "mouse.move", json!({"x": 99})).await
    });
    click?;
    moved?;
    assert!(tab.script().events().starts_with(&[
        "measure".to_owned(),
        "arm".to_owned(),
        "mouseMoved 99 0".to_owned(),
        "glide 10".to_owned()
    ]));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn abort_during_the_ripple_leaves_delivery_not_sent() -> TestResult {
    for op in ["locator.click", "mouse.click"] {
        let tab = ScriptedTab::new(ClickPage {
            slow_expression: Some("p.click("),
            ..Default::default()
        });
        let mut run = ScriptedRun::over(&tab).await?;
        run.env.draws_cursor = true;
        let (result, ()) = tokio::join!(invoke(&run, op, json!({})), async {
            wait_for_request(&tab.script().evaluating).await;
            run.env.abort("cancelled while painting");
        });
        assert_eq!(
            result
                .err()
                .ok_or("click should have aborted")?
                .error_code(),
            Some(ErrorCode::Aborted)
        );
        assert!(
            !run.env.calls().contains(&EnvCall::MarkSent),
            "{op} delivery"
        );
        assert_eq!(tab.sent("Input.dispatchMouseEvent"), 0);
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_background_capture_reads_pixels_before_the_first_glide_creates_a_canvas() -> TestResult {
    let tab = ScriptedTab::new(ClickPage {
        capture_delay: Duration::from_millis(100),
        ..Default::default()
    });
    let mut run = ScriptedRun::over(&tab).await?;
    run.env.draws_cursor = true;
    let browser = run.playwright.browser();
    let page = browser.pages.list().await?.pop().ok_or("missing tab")?;
    let capture = browser.screenshot_for_target(
        page.page_id,
        &page.target_id,
        ScreenshotCaptureOptions {
            format: Some(ScreenshotFormat::Jpeg),
            ..Default::default()
        },
    );
    let (shot, click) = tokio::join!(capture, async {
        wait_for_request(&tab.script().capturing).await;
        invoke(&run, "mouse.click", json!({})).await
    });
    assert_eq!(shot?.ok_or("target closed")?.data, "AQID");
    click?;
    assert_eq!(
        &tab.script().events()[..3],
        &["hide absent", "capture", "glide 10"]
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn double_clicks_wait_for_a_busy_screenshot_queue_within_their_budget() -> TestResult {
    // A PNG holds the same session queue as the trace JPEG without its screencast fallback.
    for (op, timeout_ms, capture_ms, skip_after_ms) in [
        ("locator.dblclick", 6_000, 1_500, None),
        ("mouse.click", 6_000, 1_500, None),
        // The 100 ms move can finish after the three-second lock wait limit.
        ("mouse.click", 6_000, 2_950, None),
        ("mouse.click", 3_000, 1_500, Some(1_000)),
        ("mouse.click", 10_000, 4_000, Some(3_000)),
    ] {
        let tab = ScriptedTab::new(ClickPage {
            capture_delay: Duration::from_millis(capture_ms),
            slow_expression: Some("var BrowserOSAgentCursor"),
            ..Default::default()
        });
        let mut run = ScriptedRun::over(&tab).await?;
        run.env.draws_cursor = true;
        let browser = run.playwright.browser();
        let page = browser.pages.list().await?.pop().ok_or("missing tab")?;
        let capture = browser.screenshot_for_target(
            page.page_id,
            &page.target_id,
            ScreenshotCaptureOptions::default(),
        );
        let (shot, click) = tokio::join!(capture, async {
            wait_for_request(&tab.script().capturing).await;
            let started = time::Instant::now();
            let mut options = json!({"call": {"timeoutMs": timeout_ms}});
            if op == "mouse.click" {
                options["clickCount"] = json!(2);
            }
            invoke(&run, op, options).await.map(|_| started.elapsed())
        });
        assert_eq!(shot?.ok_or("target closed")?.data, "AQID");
        let elapsed = click?;
        let mut events = tab.script().events();
        events.retain(|event| !matches!(event.as_str(), "measure" | "arm"));
        let mut expected = vec!["hide absent"];
        if skip_after_ms.is_none() {
            expected.extend(["capture", "glide 10", "ripple 10,20"]);
        }
        expected.extend([
            "mouseMoved 10 0",
            "mousePressed 10 1",
            "mouseReleased 10 1",
            "mousePressed 10 2",
            "mouseReleased 10 2",
        ]);
        if let Some(wait_ms) = skip_after_ms {
            expected.push("capture");
            assert!(
                elapsed.abs_diff(Duration::from_millis(wait_ms)) < Duration::from_millis(10),
                "{op} with {timeout_ms}ms budget waited {elapsed:?}"
            );
        }
        assert_eq!(events, expected, "{op} with {timeout_ms}ms budget");
    }
    Ok(())
}

#[tokio::test]
async fn a_hidden_page_or_short_budget_still_gets_one_click() -> TestResult {
    for op in ["locator.click", "mouse.click"] {
        for skipped in [false, true] {
            let tab = ScriptedTab::new(ClickPage {
                skipped,
                ..Default::default()
            });
            let mut run = ScriptedRun::over(&tab).await?;
            run.env.draws_cursor = true;
            let budget = if skipped { 5_000 } else { 1_000 };
            invoke(&run, op, json!({ "call": { "timeoutMs": budget } })).await?;
            let effects: Vec<_> = tab
                .script()
                .events()
                .into_iter()
                .filter(|event| event != "arm" && event != "measure")
                .collect();
            let mut expected = vec!["glide 10"; usize::from(skipped)];
            expected.extend(["mouseMoved 10 0", "mousePressed 10 1", "mouseReleased 10 1"]);
            assert_eq!(effects, expected);
        }
    }
    Ok(())
}

#[tokio::test]
async fn only_real_explicit_clicks_draw_outside_conformance() -> TestResult {
    for (op, options) in [
        ("locator.click", json!({ "trial": true })),
        ("locator.hover", json!({})),
        ("locator.check", json!({})),
        ("mouse.move", json!({})),
        (
            "locator.click",
            json!({ "call": { "conformance": true, "timeoutMs": 5_000 } }),
        ),
        (
            "mouse.click",
            json!({ "call": { "conformance": true, "timeoutMs": 5_000 } }),
        ),
    ] {
        let tab = ScriptedTab::new(ClickPage::default());
        let mut run = ScriptedRun::over(&tab).await?;
        run.env.draws_cursor = true;
        invoke(&run, op, options).await?;
        let events = tab.script().events();
        assert!(
            events.iter().any(|e| e.starts_with("mouse")),
            "{op} exercised input"
        );
        assert!(
            !events
                .iter()
                .any(|e| e.starts_with("glide") || e.starts_with("ripple")),
            "{op}: {events:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn travel_is_capped_and_the_ripple_and_press_use_the_fresh_point() -> TestResult {
    for (step, glides) in [(2, 1), (10, 3)] {
        let tab = ScriptedTab::new(ClickPage {
            step,
            ..Default::default()
        });
        let mut run = ScriptedRun::over(&tab).await?;
        run.env.draws_cursor = true;
        invoke(&run, "locator.click", json!({})).await?;
        let events = tab.script().events();
        let x = 10 + step * glides;
        assert_eq!(
            events.iter().filter(|e| e.starts_with("glide")).count(),
            usize::try_from(glides)?
        );
        assert!(events.ends_with(&[
            format!("ripple {x},20"),
            format!("mouseMoved {x} 0"),
            format!("mousePressed {x} 1"),
            format!("mouseReleased {x} 1")
        ]));
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_covered_target_never_draws() -> TestResult {
    let tab = ScriptedTab::new(ClickPage {
        covered: true,
        ..Default::default()
    });
    let mut run = ScriptedRun::over(&tab).await?;
    run.env.draws_cursor = true;
    let error = invoke(&run, "locator.click", json!({}))
        .await
        .err()
        .ok_or("cover must refuse")?;
    assert_eq!(error.error_code(), Some(ErrorCode::NotActionable));
    let events = tab.script().events();
    assert!(events.iter().any(|e| e == "arm"));
    assert!(events.iter().all(|e| e == "measure" || e == "arm"));
    Ok(())
}
