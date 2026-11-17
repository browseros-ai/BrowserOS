//! What a pointer action under a cover says when its timeout runs out (covered-click spec
//! section 2), against a real browser: the code `covered`, the cover named by ref, role and name,
//! when it came, what was pressed, its controls, the line of what to do by kind, and the `cover`
//! field. Every shape is `data/covered/covered.html` with a hash, beside a target that logs only
//! trusted clicks, so a row reads what the page itself saw. Every row chooses its timeout: when
//! a click stops waiting under the built-in one is section 1's, a later change.
//!
//! Needs BrowserOS with CDP on `BROWSEROS_CDP_PORT` (default 9005):
//! `bun run test:browser -- cargo test -p neo-playwright --test covered_live -- --ignored`.

#[path = "support/fixture_server.rs"]
#[expect(dead_code, reason = "the nested pages and headers serve other suites")]
mod fixture_server;
#[path = "support/js_runner.rs"]
#[expect(dead_code, reason = "the gesture and eval natives serve other suites")]
mod js_runner;
#[path = "support/live_run.rs"]
#[expect(dead_code, reason = "the capture and input helpers serve other suites")]
mod live_run;
#[path = "support/locator.rs"]
#[expect(dead_code, reason = "the role, text and attr parts serve other suites")]
mod locator;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fixture_server::{FixtureServer, Page};
use js_runner::OpHost;
use live_run::{EnvCall, LiveRun, TestPage, TestResult, brand};
use locator::{aria_ref, css, enter_frame, locator};
use neo_playwright::{ErrorCode, PlaywrightError, TimedOut};
use serde_json::{Value, json};
use tokio::time;

/// How long each row lets its action wait: the facts come when it runs out.
const CHOSEN_MS: u64 = 1500;

fn pages() -> HashMap<String, Page> {
    HashMap::from([
        (
            "/covered/covered.html".to_owned(),
            Page::html(include_str!("data/covered/covered.html")),
        ),
        (
            "/covered/above.html".to_owned(),
            Page::html(include_str!("data/covered/above.html")),
        ),
        (
            "/covered/banner.html".to_owned(),
            Page::html(include_str!("data/covered/banner.html")),
        ),
        (
            "/covered/frameside.html".to_owned(),
            Page::html(include_str!("data/covered/frameside.html")),
        ),
    ])
}

/// One shape of the fixture, open in its own tab.
struct Shape {
    run: Arc<LiveRun>,
    page: TestPage,
    _server: FixtureServer,
}

impl Shape {
    async fn open(path: &str) -> TestResult<Self> {
        let server = FixtureServer::start(pages()).await?;
        let run = Arc::new(LiveRun::connect().await?);
        let page = run.new_page().await?;
        run.invoke(
            "page.goto",
            json!({ "call": { "timeoutMs": 30_000 }, "page": brand(&page),
                    "url": server.url("127.0.0.1", path), "waitUntil": "load" }),
        )
        .await?;
        Ok(Self {
            run,
            page,
            _server: server,
        })
    }

    async fn shape(name: &str) -> TestResult<Self> {
        Self::open(&format!("/covered/covered.html#{name}")).await
    }

    fn at(&self, selector: &str) -> Value {
        locator(
            &self.page,
            &[css(selector)],
            &format!("locator('{selector}')"),
        )
    }

    /// A pointer op on `target` under the row's chosen timeout, with `options` beside it.
    async fn act(
        &self,
        op: &str,
        target: &Value,
        options: Value,
    ) -> Result<Value, PlaywrightError> {
        let mut input = json!({ "call": { "api": op, "timeoutMs": CHOSEN_MS },
                                "target": target, "strict": "strict" });
        if let (Some(fields), Value::Object(options)) = (input.as_object_mut(), options) {
            fields.extend(options);
        }
        self.run.invoke(op, input).await
    }

    async fn click(&self, selector: &str) -> Result<Value, PlaywrightError> {
        self.act("locator.click", &self.at(selector), json!({}))
            .await
    }

    /// Whether `reference` resolves to exactly one element, as `page.getByRef` does.
    async fn resolves(&self, reference: &str) -> TestResult<bool> {
        let target = locator(&self.page, &[aria_ref(reference)], reference);
        let count = self
            .run
            .invoke(
                "locator.count",
                json!({ "call": { "api": "locator.count" }, "target": target }),
            )
            .await?;
        Ok(count == json!(1))
    }

    async fn log(&self) -> TestResult<Vec<String>> {
        Ok(serde_json::from_value(
            self.run.eval(&self.page, "window.__log").await?,
        )?)
    }

    async fn close(&self) {
        self.run.close_page(&self.page).await;
    }
}

/// The covered error a row expects, with its `cover` field.
fn covered(answer: Result<Value, PlaywrightError>) -> TestResult<(String, Value)> {
    let error = answer.err().ok_or("the action under the cover succeeded")?;
    assert_eq!(error.error_code(), Some(ErrorCode::Covered), "{error}");
    assert_eq!(
        error.timed_out(),
        TimedOut::Yes,
        "a covered answer is a TimeoutError"
    );
    let cover = error
        .cover_details()
        .cloned()
        .ok_or("a covered error carries its cover")?;
    Ok((error.message().to_owned(), cover))
}

fn text<'a>(cover: &'a Value, field: &str) -> &'a str {
    cover.get(field).and_then(Value::as_str).unwrap_or_default()
}

/// The facts' lines come in the keeper's order, all before the call log.
fn in_order(message: &str, lines: &[&str]) -> TestResult {
    let mut from = 0;
    for line in lines {
        let at = message
            .get(from..)
            .and_then(|rest| rest.find(line))
            .ok_or_else(|| format!("{line:?} after byte {from} of:\n{message}"))?;
        from += at + line.len();
    }
    Ok(())
}

/// Row 2 and the page kind of row 8: a chosen timeout under a dialog runs out at that timeout
/// with the facts, nothing reaches the target, and the listed control clears it.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_chosen_timeout_under_a_dialog_says_what_covers_it() -> TestResult {
    let shape = Shape::shape("dialog").await?;
    let result = async {
        let started = Instant::now();
        let (message, cover) = covered(shape.click("#target").await)?;
        let took = started.elapsed();
        assert!(took < Duration::from_millis(CHOSEN_MS + 2500), "{took:?}");
        in_order(
            &message,
            &[
                "locator.click: Timeout 1500ms exceeded.",
                "\n  On top: dialog \"What You Need To Know\" [ref=",
                "\n  It has been there since this click began.",
                "\n  Nothing was pressed.",
                "\n  Its controls:",
                "button \"Accept & Continue\" [ref=",
                "button \"Dismiss\" [ref=",
                "\n  Do: answer or close it with one of its controls, then try again.",
                "\nCall log:",
                "intercepts pointer events",
            ],
        )?;
        assert_eq!(
            (
                text(&cover, "role"),
                text(&cover, "name"),
                text(&cover, "kind")
            ),
            ("dialog", "What You Need To Know", "page")
        );
        assert!(shape.resolves(text(&cover, "ref")).await?);
        assert!(shape.log().await?.is_empty(), "the target saw no click");
        let dismiss = cover
            .get("controls")
            .and_then(Value::as_array)
            .and_then(|controls| {
                controls
                    .iter()
                    .find(|control| control.get("name") == Some(&json!("Dismiss")))
            })
            .and_then(|control| control.get("ref"))
            .and_then(Value::as_str)
            .ok_or("the controls list Dismiss")?
            .to_owned();
        let by_ref = locator(&shape.page, &[aria_ref(&dismiss)], &dismiss);
        shape.act("locator.click", &by_ref, json!({})).await?;
        shape.click("#target").await?;
        assert_eq!(
            shape.log().await?,
            ["target"],
            "one trusted click, after the way out"
        );
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// Row 8, every other kind: the kind, and the line of what to do.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_each_kind_says_what_to_do() -> TestResult {
    // A press on the target's own lands by itself (the next row), unless the script chose the
    // point: the partial, label and wrapper kinds answer at a covered point the script gave.
    let at = |x: f64, y: f64| Some(json!({ "x": x, "y": y }));
    for (name, kind, line, position) in [
        ("frame", "frame", "Do: a frame covers it.", None),
        (
            "partial",
            "partial",
            "Do: only part of it is covered.",
            at(150.0, 24.0),
        ),
        (
            "label",
            "label",
            "Do: it sits under its own label. Press Space on it.",
            at(6.0, 6.0),
        ),
        (
            "wrapper",
            "wrapper",
            "Do: its own wrapper takes the pointer.",
            at(6.0, 6.0),
        ),
        (
            "labels",
            "labels",
            "Do: it labels the control on top. Act on that control:",
            None,
        ),
        (
            "nothing",
            "nothing",
            "Do: it takes no pointer at its centre.",
            None,
        ),
        (
            "folded",
            "folded",
            "Do: it is not shown where it lies",
            None,
        ),
    ] {
        let shape = Shape::shape(name).await?;
        let options = position.map_or_else(|| json!({}), |at| json!({ "position": at }));
        let result = async {
            let (message, cover) = covered(
                shape
                    .act("locator.click", &shape.at("#target"), options)
                    .await,
            )
            .map_err(|error| format!("{name}: {error}"))?;
            assert_eq!(text(&cover, "kind"), kind, "{name}: {message}");
            assert!(message.contains(line), "{name}: {message}");
            // These covers lie in the page's flow: the retries' scrolls move them in the view
            // with their control, and they still stand from the click's first check.
            if matches!(name, "partial" | "label" | "wrapper") {
                assert!(
                    message.contains("\n  It has been there since this click began."),
                    "{name}: {message}"
                );
            }
            assert!(shape.resolves(text(&cover, "ref")).await?, "{name}");
            assert!(
                shape.log().await?.is_empty(),
                "{name}: the target saw no click"
            );
            TestResult::Ok(())
        }
        .await;
        shape.close().await;
        result?;
    }
    Ok(())
}

/// The call-log lines the run's ops left, which the trace keeps as their notes.
fn notes(shape: &Shape) -> Vec<String> {
    shape
        .run
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            EnvCall::Note(line) => Some(line),
            _ => None,
        })
        .collect()
}

/// Whether one of the run's call-log lines starts with `start`.
fn noted(shape: &Shape, start: &str) -> bool {
    notes(shape).iter().any(|line| line.starts_with(start))
}

/// How many presses of the target's own the call log counts: a host counts the same match.
fn presses(shape: &Shape) -> usize {
    notes(shape)
        .iter()
        .filter(|line| line.contains("takes the press: "))
        .count()
}

/// Runs `row` on each shape, each in its own tab, and fails with every shape's failure, so one
/// failing shape does not hide another's.
async fn each_shape<F, Fut>(names: &[&str], row: F) -> TestResult
where
    F: Fn(Shape, String) -> Fut,
    Fut: Future<Output = (Shape, TestResult)>,
{
    let mut failures = Vec::new();
    for name in names {
        let (shape, result) = row(Shape::shape(name).await?, (*name).to_owned()).await;
        shape.close().await;
        if let Err(error) = result {
            failures.push(format!("{name}: {error}"));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n").into())
    }
}

/// Covered-click C: a press whose cover is the target's own lands by itself, with one trusted
/// press, and the call log says which in one fixed form. The shapes: a clear point of a button
/// half under a widget; of one whose left part is a component drawing a link, which is not its
/// own surface; of one whose first sample lies a quarter-pixel before a nested link, which pins
/// where the press lands but not the order of test and rounding, since on a whole-pixel layout
/// the old order passes too; of a card whose quick-add button shows on hover over its first clear
/// point, which the guard at delivery swallows once before the press lands on the card's own
/// part; a checkbox under its own label; a terms checkbox whose label holds two links (the press
/// goes where the label is plain, and the page stays); and a radio under its own blank wrapper
/// once that wrapper stood its four checks.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_press_on_the_targets_own_lands_by_itself() -> TestResult {
    for (name, line) in [
        ("partial", "  a clear point of it takes the press: { x: "),
        (
            "partiallink",
            "  a clear point of it takes the press: { x: ",
        ),
        (
            "partialround",
            "  a clear point of it takes the press: { x: ",
        ),
        (
            "partialhover",
            "  a clear point of it takes the press: { x: ",
        ),
        ("label", "  its own label takes the press: <label"),
        ("links", "  its own label takes the press: <label"),
        ("wrapper", "  its own wrapper takes the press: <div"),
    ] {
        let shape = Shape::shape(name).await?;
        let result = async {
            shape
                .click("#target")
                .await
                .map_err(|error| format!("{name}: {error}"))?;
            assert!(noted(&shape, line), "{name}: {:?}", notes(&shape));
            assert_eq!(presses(&shape), 1, "{name}: one press, one line");
            if name.starts_with("partial") {
                assert_eq!(shape.log().await?, ["target"], "{name}");
            } else {
                let checked = shape
                    .run
                    .eval(&shape.page, "document.getElementById('target').checked")
                    .await?;
                assert_eq!(checked, json!(true), "{name}");
            }
            assert_eq!(
                shape.run.eval(&shape.page, "location.pathname").await?,
                json!("/covered/covered.html"),
                "{name}: the page stayed"
            );
            TestResult::Ok(())
        }
        .await;
        shape.close().await;
        result?;
    }
    Ok(())
}

/// A radio under its own blank wrapper that does nothing on a press: the press lands on the
/// wrapper, the radio stays as it was, and the call log says that its state did not change. A
/// `check` through it fails, and its error names the wrapper as what took the press.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_dead_wrapper_says_the_state_did_not_change() -> TestResult {
    let shape = Shape::shape("wrapperdead").await?;
    let result = async {
        shape.click("#target").await?;
        assert!(
            noted(&shape, "  its own wrapper takes the press: <div"),
            "{:?}",
            notes(&shape)
        );
        assert!(
            noted(
                &shape,
                "  its own wrapper took the press, and the control's state did not change"
            ),
            "{:?}",
            notes(&shape)
        );
        assert_eq!(
            shape
                .run
                .eval(&shape.page, "document.getElementById('target').checked")
                .await?,
            json!(false)
        );
        // A checking action still verifies after a press of the target's own: the state did
        // not change, so `check` fails, and says which took the press.
        let checked = shape
            .act("locator.check", &shape.at("#target"), json!({}))
            .await
            .err()
            .ok_or("check through a dead wrapper must fail")?;
        assert!(
            checked
                .message()
                .contains("did not change its state: its own wrapper took the press"),
            "{checked}"
        );
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// What takes the press as a wrapper is nothing in itself: a radio slotted into a component whose
/// own root draws a Remove button over it, a sibling `span role="button"` drawing the checkbox's
/// name over it, and a radio card that is a link. Each is the control's wrapper, and none is
/// pressed: the covered answer comes with the wrapper's line, nothing saw a click, the control
/// stays as it was, and the page stays.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_wrapper_that_is_something_in_itself_is_not_pressed() -> TestResult {
    each_shape(
        &["wrappershadow", "wrapperrole", "wrapperlink"],
        |shape, name| async move {
            let result = async {
                let (message, cover) = covered(shape.click("#target").await)?;
                if text(&cover, "kind") != "wrapper" || presses(&shape) != 0 {
                    return Err(format!("{message}\n{:?}", notes(&shape)).into());
                }
                let log = shape.log().await?;
                let checked = shape
                    .run
                    .eval(&shape.page, "document.getElementById('target').checked")
                    .await?;
                let path = shape.run.eval(&shape.page, "location.pathname").await?;
                if !log.is_empty()
                    || checked != json!(false)
                    || path != json!("/covered/covered.html")
                {
                    return Err(format!("{name}: saw {log:?}, checked {checked}, at {path}").into());
                }
                TestResult::Ok(())
            }
            .await;
            (shape, result)
        },
    )
    .await
}

/// A dblclick under the target's own cover keeps the covered answer, as on main: on the
/// half-covered button and on the checkbox under its own label, nothing is pressed and no
/// `takes the press` line is logged.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_dblclick_keeps_the_covered_answer() -> TestResult {
    each_shape(&["partial", "label"], |shape, name| async move {
        let result = async {
            let answer = shape
                .act("locator.dblclick", &shape.at("#target"), json!({}))
                .await;
            let (message, _cover) = covered(answer)?;
            let log = shape.log().await?;
            if presses(&shape) != 0 || !log.is_empty() {
                return Err(format!("{name}: saw {log:?}, {message}").into());
            }
            TestResult::Ok(())
        }
        .await;
        (shape, result)
    })
    .await
}

/// A slider half under a widget: a clear point of it would take a press for a button, but a
/// slider pressed anywhere moves its value, so only the point roles go through a clear point.
/// The covered answer comes, with the kind and the clear point to act on; nothing is pressed.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_target_with_no_point_role_is_not_pressed_at_a_clear_point() -> TestResult {
    let shape = Shape::shape("partialslider").await?;
    let result = async {
        let (message, cover) = covered(shape.click("#target").await)?;
        assert_eq!(text(&cover, "kind"), "partial", "{message}");
        assert!(
            message.contains("Act on it at a clear point: { position: { x: "),
            "{message}"
        );
        assert!(
            !noted(&shape, "  a clear point of it takes the press"),
            "{message}"
        );
        assert!(shape.log().await?.is_empty(), "the slider saw no press");
        assert_eq!(
            shape
                .run
                .eval(&shape.page, "document.getElementById('target').value")
                .await?,
            json!("50")
        );
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// A button under its parent's blank veil: its own wrapper, but a button has no state to show
/// what a press on the wrapper did, so nothing is pressed and the covered answer comes.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_veil_over_a_button_is_not_pressed() -> TestResult {
    let shape = Shape::shape("veilbutton").await?;
    let result = async {
        let (message, cover) = covered(shape.click("#target").await)?;
        assert_eq!(text(&cover, "kind"), "wrapper", "{message}");
        assert!(
            !noted(&shape, "  its own wrapper takes the press"),
            "{message}"
        );
        assert!(shape.log().await?.is_empty(), "the button saw no click");
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// A checkbox under its own label that is all link takes no press: a press on a link could
/// leave the page. The covered answer comes, its line says to press Space, and the page stays.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_label_that_is_all_link_is_never_pressed() -> TestResult {
    let shape = Shape::shape("alllink").await?;
    let result = async {
        let (message, cover) = covered(shape.click("#target").await)?;
        assert_eq!(text(&cover, "kind"), "label", "{message}");
        assert!(
            message.contains("Do: it sits under its own label. Press Space on it."),
            "{message}"
        );
        assert!(
            !noted(&shape, "  its own label takes the press"),
            "{message}"
        );
        assert_eq!(
            shape.run.eval(&shape.page, "location.pathname").await?,
            json!("/covered/covered.html"),
            "the page stayed"
        );
        assert_eq!(
            shape
                .run
                .eval(&shape.page, "document.getElementById('target').checked")
                .await?,
            json!(false)
        );
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// The target's `checked`, in the document or in the open shadow root of `#host`.
const SHADOWED_TARGET_CHECKED: &str = "(document.getElementById('target') ?? \
     document.getElementById('host').shadowRoot.getElementById('target')).checked";

/// A press of the target's own label goes where the label is plain: never on a custom control
/// in it (a `span role="button"` with no tabindex runs its own click handler), nor on what a
/// component inside it draws in its own root (a link there); a label inside a container with a
/// tabindex, as a dialog's body has, still has plain points; and so has one inside an open shadow
/// root, whose hits are asked in that root. The box is checked, the label's own control saw no
/// click, and the page stayed.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_label_press_goes_where_the_label_is_plain() -> TestResult {
    each_shape(
        &["labelrole", "linksdialog", "linksshadow", "linksnested"],
        |shape, name| async move {
            let result = async {
                shape.click("#target").await?;
                if !noted(&shape, "  its own label takes the press: <label") || presses(&shape) != 1
                {
                    return Err(format!("one label press: {:?}", notes(&shape)).into());
                }
                let log = shape.log().await?;
                if log.iter().any(|what| what == "info") {
                    return Err(format!("the label's own control was pressed: {log:?}").into());
                }
                let checked = shape.run.eval(&shape.page, SHADOWED_TARGET_CHECKED).await?;
                let path = shape.run.eval(&shape.page, "location.pathname").await?;
                if checked != json!(true) || path != json!("/covered/covered.html") {
                    return Err(format!("checked {checked}, at {path}, for {name}").into());
                }
                TestResult::Ok(())
            }
            .await;
            (shape, result)
        },
    )
    .await
}

/// A label's state two: a checkbox that is not shown, whose own label is. `check` waits on the
/// label, presses it, and the box is checked; also when the label lies below the view, at the
/// foot of a long form, until the attempt scrolls it.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_hidden_checkbox_is_checked_through_its_label() -> TestResult {
    each_shape(&["hiddenbox", "hiddenboxfar"], |shape, name| async move {
        let result = async {
            shape
                .act("locator.check", &shape.at("#target"), json!({}))
                .await?;
            if !noted(&shape, "  its own label takes the press: <label") || presses(&shape) != 1 {
                return Err(format!("one label press: {:?}", notes(&shape)).into());
            }
            let checked = shape
                .run
                .eval(&shape.page, "document.getElementById('target').checked")
                .await?;
            if checked != json!(true) {
                return Err(format!("checked {checked} for {name}").into());
            }
            TestResult::Ok(())
        }
        .await;
        (shape, result)
    })
    .await
}

/// A layer that holds nine frames and then a Close link is not a cover of frames only, though
/// the eight controls the answer prints are all frames: its kind is `page`, without the frame's
/// line, as the full list of what it takes the pointer with says.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_frames_and_a_link_are_not_frames_only() -> TestResult {
    let shape = Shape::shape("manyframes").await?;
    let result = async {
        let (message, cover) = covered(shape.click("#target").await)?;
        assert_eq!(text(&cover, "kind"), "page", "{message}");
        assert!(!message.contains("its content is in a frame"), "{message}");
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// A button in a same-process frame, half under a widget of its own page, whose clear point
/// lies under a layer of the page above: the frame offers the point, the page above covers it,
/// and nothing is pressed there. The covered answer names that page's layer, which has stood
/// since the click began, and no press of the target's own is logged.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_clear_point_under_the_page_above_is_not_pressed() -> TestResult {
    let shape = Shape::open("/covered/frameside.html").await?;
    let result = async {
        let target = locator(
            &shape.page,
            &[css("#inner"), enter_frame(), css("#target")],
            "frameLocator('#inner').locator('#target')",
        );
        let (message, _cover) = covered(shape.act("locator.click", &target, json!({})).await)?;
        assert!(
            message.contains("\n  It has been there since this click began."),
            "{message}"
        );
        assert_eq!(presses(&shape), 0, "{:?}", notes(&shape));
        assert!(shape.log().await?.is_empty(), "the page above saw no click");
        let inner = shape
            .run
            .eval(
                &shape.page,
                "document.getElementById('inner').contentWindow.__log",
            )
            .await?;
        assert_eq!(inner, json!([]), "the button saw no click");
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// The plan's row: a backdrop and its dialog as two children of `<body>`. The answer names the
/// dialog and lists its buttons, not a blank layer.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_backdrop_beside_its_dialog_is_named_after_the_dialog() -> TestResult {
    let shape = Shape::shape("siblings").await?;
    let result = async {
        let (message, cover) = covered(shape.click("#target").await)?;
        assert_eq!(
            (text(&cover, "role"), text(&cover, "name")),
            ("dialog", "Sign up for deals"),
            "{message}"
        );
        assert!(message.contains("button \"No thanks\" [ref="), "{message}");
        assert!(message.contains("button \"Sign up\" [ref="), "{message}");
        // Said as it stands: the point lies under the blank layer, and the dialog is open over it.
        assert!(
            message.contains(
                "\n  On top: a blank layer <div id=\"backdrop\"></div>. Open over it: \
                              dialog \"Sign up for deals\" [ref="
            ),
            "{message}"
        );
        assert!(
            message.contains("Do: answer or close that dialog with one of its controls"),
            "{message}"
        );
        assert!(!message.contains("Do: a blank layer"), "{message}");
        assert!(shape.resolves(text(&cover, "ref")).await?);
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// A sign-in dialog with a close button of its own, whose form's frame lies over the click point:
/// the cover is the dialog, its kind is `page`, its controls are the button and the frame, and
/// the last line is the ordinary one, not the frame's; the button's ref closes it.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_dialogs_frame_at_the_point_keeps_the_dialogs_line() -> TestResult {
    let shape = Shape::shape("signin").await?;
    let result = async {
        let (message, cover) = covered(shape.click("#target").await)?;
        assert_eq!(text(&cover, "role"), "dialog", "{message}");
        assert_eq!(text(&cover, "kind"), "page", "{message}");
        let close = control_ref(&cover, "Close Sign in or Sign up")?;
        control_ref(&cover, "Login/Signup")?;
        assert!(
            message.contains("\n  Do: answer or close it with one of its controls"),
            "{message}"
        );
        assert!(!message.contains("a frame covers it"), "{message}");
        let by_ref = locator(&shape.page, &[aria_ref(&close)], &close);
        shape.act("locator.click", &by_ref, json!({})).await?;
        shape.click("#target").await?;
        assert_eq!(
            shape.log().await?,
            ["target"],
            "the dialog closed, the click lands"
        );
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// A backdrop beside a host whose shadow dialog draws its buttons through a slot: a plain button
/// and a custom element whose real button lives in its own root. The answer names the dialog,
/// lists both, and the plain one's ref presses it.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_dialogs_slotted_button_is_its_control() -> TestResult {
    let shape = Shape::shape("slotted").await?;
    let result = async {
        let (message, cover) = covered(shape.click("#target").await)?;
        assert!(
            message.contains(". Open over it: dialog \"Before you go\" [ref="),
            "{message}"
        );
        assert!(!message.contains("Do: a blank layer"), "{message}");
        let leave = control_ref(&cover, "Leave")?;
        control_ref(&cover, "Stay")?;
        let by_ref = locator(&shape.page, &[aria_ref(&leave)], &leave);
        shape.act("locator.click", &by_ref, json!({})).await?;
        shape.click("#target").await?;
        assert_eq!(
            shape.log().await?,
            ["target"],
            "the slotted button pressed, the click lands"
        );
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// A fixed backdrop and a fixed dialog drawn as siblings in one open shadow root, under a host
/// that is not fixed: the backdrop is named after the dialog standing on it, with its button.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_backdrop_in_a_shadow_root_is_named_after_its_dialog() -> TestResult {
    let shape = Shape::shape("shadowpair").await?;
    let result = async {
        let (message, cover) = covered(shape.click("#target").await)?;
        assert!(
            message.contains(". Open over it: dialog \"Your privacy\" [ref="),
            "{message}"
        );
        assert!(!message.contains("Do: a blank layer"), "{message}");
        control_ref(&cover, "Got it")?;
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// A backdrop beside a dialog whose only control is a `div` with `role="button"` and no
/// tabindex: the answer names the dialog, not a blank layer, and lists that control, whose ref
/// presses it.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_role_button_dialog_is_named_after_its_backdrop() -> TestResult {
    let shape = Shape::shape("rolebutton").await?;
    let result = async {
        let (message, cover) = covered(shape.click("#target").await)?;
        assert!(
            message.contains(". Open over it: dialog \"Rate your visit\" [ref="),
            "{message}"
        );
        assert!(
            message.contains("\n    - button \"Close\" [ref="),
            "{message}"
        );
        assert!(!message.contains("Do: a blank layer"), "{message}");
        let close = control_ref(&cover, "Close")?;
        let by_ref = locator(&shape.page, &[aria_ref(&close)], &close);
        shape.act("locator.click", &by_ref, json!({})).await?;
        shape.click("#target").await?;
        assert_eq!(
            shape.log().await?,
            ["target"],
            "the control pressed, the click lands"
        );
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// The same pair on a page of more than 20,000 elements, the dialog the last child of `<body>`
/// as a portal renders it: the backdrop is named after it, with its controls.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_backdrop_is_named_after_a_dialog_at_the_end_of_a_long_page() -> TestResult {
    let shape = Shape::shape("siblingsdeep").await?;
    let result = async {
        let (message, cover) = covered(shape.click("#target").await)?;
        assert_eq!(text(&cover, "name"), "Sign up for deals", "{message}");
        assert!(
            message.contains(". Open over it: dialog \"Sign up for deals\" [ref="),
            "{message}"
        );
        assert!(!message.contains("Do: a blank layer"), "{message}");
        assert!(message.contains("button \"No thanks\" [ref="), "{message}");
        assert!(message.contains("button \"Sign up\" [ref="), "{message}");
        let last: Value = shape
            .run
            .eval(
                &shape.page,
                "[document.querySelectorAll('*').length, document.body.lastElementChild.getAttribute('aria-label')]",
            )
            .await?;
        assert!(last[0].as_u64().unwrap_or_default() > 20_000, "{last}");
        assert_eq!(last[1], json!("Sign up for deals"), "the dialog is the last child: {last}");
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// Row 9: a blank layer is in no snapshot, so its ref is minted, and it resolves.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_blank_layer_gets_a_minted_ref_that_resolves() -> TestResult {
    let shape = Shape::shape("blank").await?;
    let result = async {
        let (message, cover) = covered(shape.click("#target").await)?;
        // No snapshot prints a blank layer, so its ref is minted, and it resolves.
        let reference = text(&cover, "ref");
        assert!(reference.starts_with('n'), "minted: {reference}");
        assert!(message.contains(&format!("[ref={reference}]")), "{message}");
        assert!(shape.resolves(reference).await?);
        assert!(message.contains("Do: a blank layer."), "{message}");
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// Row 10: a cover in the page above the target's frame is that page's, and its ref resolves
/// there.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_cover_in_the_page_above_is_named_by_that_page() -> TestResult {
    let shape = Shape::open("/covered/above.html").await?;
    let result = async {
        let target = locator(
            &shape.page,
            &[css("#inner"), enter_frame(), css("#target")],
            "frameLocator('#inner').locator('#target')",
        );
        let (message, cover) = covered(shape.act("locator.click", &target, json!({})).await)?;
        assert_eq!(
            (text(&cover, "role"), text(&cover, "name")),
            ("dialog", "Your location"),
            "{message}"
        );
        let reference = text(&cover, "ref");
        assert!(
            !reference.starts_with('f'),
            "the top page's ref: {reference}"
        );
        assert!(shape.resolves(reference).await?);
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// Row 11: two dialogs in one layer; the answer names the one at the click point, and closing
/// the other does not clear it.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_answer_names_the_dialog_at_the_point() -> TestResult {
    let shape = Shape::shape("two").await?;
    let result = async {
        let (_, cover) = covered(shape.click("#target").await)?;
        assert_eq!(text(&cover, "name"), "Sign in");
        shape.click("#cookies").await?;
        let (_, cover) = covered(shape.click("#target").await)?;
        assert_eq!(text(&cover, "name"), "Sign in");
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// Row 12: a dialog with 3,000 characters before its button: the controls, Rust's own lines,
/// still list it.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_long_dialog_keeps_its_button_in_the_controls() -> TestResult {
    let shape = Shape::shape("long").await?;
    let result = async {
        let (message, _) = covered(shape.click("#target").await)?;
        assert!(message.contains("button \"I agree\" [ref="), "{message}");
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// Row 13: a dialog that holds a private box: no line and no field carries its value.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_covers_private_box_stays_masked() -> TestResult {
    let shape = Shape::shape("masked").await?;
    let result = async {
        let (message, cover) = covered(shape.click("#target").await)?;
        assert!(!message.contains("hunter2"), "{message}");
        assert!(!cover.to_string().contains("hunter2"), "{cover}");
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// Row 7: a cover that arrives with the press's own move: the press is stopped at the window,
/// the page saw no click, and the answer says when the cover came and that one press went out.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_cover_that_came_during_the_click_says_so() -> TestResult {
    let shape = Shape::shape("late").await?;
    let result = async {
        let (message, _) = covered(shape.click("#target").await)?;
        assert!(message.contains("\n  It appeared "), "{message}");
        assert!(message.contains(" s into this click."), "{message}");
        assert!(
            message.contains("\n  One press went out, and Playwright's interceptor stopped it"),
            "{message}"
        );
        assert!(shape.log().await?.is_empty(), "the target saw no click");
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// Row 14: hover, check and dblclick under a standing cover answer the same way; `force` makes
/// no hit test and is unchanged.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_every_pointer_action_answers_covered_but_force() -> TestResult {
    let shape = Shape::shape("boxed").await?;
    let result = async {
        let target = shape.at("#target");
        for (op, name) in [
            ("locator.hover", "hover"),
            // Upstream's check is a click of the box: its log, and the answer, say click.
            ("locator.check", "click"),
            ("locator.dblclick", "dblclick"),
        ] {
            let (message, cover) = covered(shape.act(op, &target, json!({})).await)?;
            assert_eq!(text(&cover, "kind"), "page", "{op}");
            assert!(
                message.contains(&format!("since this {name} began")),
                "{message}"
            );
        }
        let forced = shape
            .act("locator.click", &target, json!({ "force": true }))
            .await;
        assert!(forced.is_ok(), "{forced:?}");
        assert!(
            shape.log().await?.is_empty(),
            "the forced press went to the cover"
        );
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// The same layer again: a second covered answer naming the same root on this tab adds the
/// line, and never names `dispatchEvent`.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_same_layer_again_is_said() -> TestResult {
    let shape = Shape::shape("dialog").await?;
    let result = async {
        let (first, _) = covered(shape.click("#target").await)?;
        assert!(!first.contains("The same layer"), "{first}");
        let (second, _) = covered(shape.click("#target").await)?;
        assert!(
            second.contains("\n  The same layer covered your last click on this tab, "),
            "{second}"
        );
        assert!(!second.contains("dispatchEvent"));
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// Under the conformance switch nothing changes: `not_actionable`, no field, upstream's text.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_conformance_keeps_upstreams_answer() -> TestResult {
    let shape = Shape::shape("dialog").await?;
    let result = async {
        let input = json!({ "call": { "api": "locator.click", "timeoutMs": CHOSEN_MS,
                                      "conformance": true },
                            "target": shape.at("#target"), "strict": "strict" });
        let error = shape
            .run
            .invoke("locator.click", input)
            .await
            .err()
            .ok_or("the click succeeded")?;
        assert_eq!(error.error_code(), Some(ErrorCode::NotActionable));
        assert!(error.cover_details().is_none());
        assert!(!error.message().contains("On top"), "{error}");
        assert!(error.message().contains("intercepts pointer events"));
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// The prelude renders the cover's own snapshot under "On top", in a script's default snapshot
/// form, and leaves the script `error.cover` without it.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_script_sees_the_covers_snapshot_and_field() -> TestResult {
    let shape = Shape::shape("dialog").await?;
    let host: Arc<dyn OpHost> = Arc::clone(&shape.run) as Arc<dyn OpHost>;
    let result = js_runner::run_script(
        &host,
        &json!({ "page": brand(&shape.page) }),
        "try { await page.locator('#target').click({ timeout: 1500 }); return 'clicked' }
         catch (error) { return { name: error.name, code: error.code, message: error.message,
                                  cover: error.cover } }",
    )
    .await;
    shape.close().await;
    let seen = result?;
    assert_eq!(seen.get("name"), Some(&json!("TimeoutError")));
    assert_eq!(seen.get("code"), Some(&json!("covered")));
    let message = seen
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    in_order(
        message,
        &[
            "\n  On top: dialog \"What You Need To Know\"",
            "\n    - dialog \"What You Need To Know\":",
            "Resale prices often exceed face value.",
            "\n  It has been there since this click began.",
            "\n  Its controls:",
            "\nCall log:",
        ],
    )?;
    let cover = seen.get("cover").ok_or("error.cover")?;
    assert_eq!(cover.get("kind"), Some(&json!("page")));
    assert!(cover.get("snapshot").is_none(), "{cover}");
    Ok(())
}

/// Row 22: the answer's own snapshot of the cover moves nothing the agent's reads depend on. A tab
/// that met a covered answer reads its next snapshot, and its next delta, exactly as a tab that
/// did not.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_covered_answer_moves_nothing_the_agents_reads_depend_on() -> TestResult {
    let reads = |click: bool| async move {
        let shape = Shape::shape("dialog").await?;
        let host: Arc<dyn OpHost> = Arc::clone(&shape.run) as Arc<dyn OpHost>;
        let script = format!(
            "let code = null
             if ({click}) try {{ await page.locator('#target').click({{ timeout: 1500 }}) }}
               catch (error) {{ code = error.code }}
             const first = await page.ariaSnapshot()
             await page.locator('#dismiss').click()
             const delta = await page.ariaSnapshot({{ delta: true }})
             return {{ code, first, delta }}"
        );
        let read =
            js_runner::run_script(&host, &json!({ "page": brand(&shape.page) }), &script).await;
        shape.close().await;
        TestResult::Ok(read?)
    };
    let after = reads(true).await?;
    let clean = reads(false).await?;
    assert_eq!(after.get("code"), Some(&json!("covered")));
    assert_eq!(after.get("first"), clean.get("first"), "the next read");
    assert_eq!(after.get("delta"), clean.get("delta"), "the next delta");
    Ok(())
}

/// A cover whose content sits in a closed shadow root this world does not know is not blank: the
/// line says a new snapshot shows it, and never "a blank layer".
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_cover_in_a_closed_part_is_not_blank() -> TestResult {
    let shape = Shape::shape("closed").await?;
    let result = async {
        let (message, _) = covered(shape.click("#target").await)?;
        assert!(
            message.contains(
                "Do: its content is in a closed part of the page: a new snapshot \
                              shows it."
            ),
            "{message}"
        );
        assert!(!message.contains("blank layer"), "{message}");
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// A consent tool's nameless host whose open shadow root draws a backdrop and a dialog, the target
/// under the backdrop: the host holds that one dialog, so the answer shows the dialog, read
/// through the root, and the dialog's buttons are its controls, with refs that resolve and press.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_dialog_in_an_open_shadow_root_is_named_with_its_buttons() -> TestResult {
    let shape = Shape::shape("shadowopen").await?;
    let result = async {
        let (message, cover) = covered(shape.click("#target").await)?;
        assert_eq!(
            (text(&cover, "role"), text(&cover, "name")),
            ("dialog", "Privacy choices"),
            "{message}"
        );
        assert!(
            message.contains("\n  On top: dialog \"Privacy choices\" [ref="),
            "{message}"
        );
        let snapshot = cover
            .get("snapshot")
            .map(Value::to_string)
            .unwrap_or_default();
        assert!(snapshot.contains("Privacy choices"), "{snapshot}");
        assert!(!message.contains("blank layer"), "{message}");
        assert!(!message.contains("Do: a blank layer"), "{message}");
        let reject = cover
            .get("controls")
            .and_then(Value::as_array)
            .and_then(|controls| {
                controls
                    .iter()
                    .find(|control| control.get("name") == Some(&json!("Reject all")))
            })
            .and_then(|control| control.get("ref"))
            .and_then(Value::as_str)
            .ok_or("the controls list Reject all")?
            .to_owned();
        assert!(message.contains("button \"Accept all\" [ref="), "{message}");
        let by_ref = locator(&shape.page, &[aria_ref(&reject)], &reject);
        shape.act("locator.click", &by_ref, json!({})).await?;
        shape.click("#target").await?;
        assert_eq!(
            shape.log().await?,
            ["target"],
            "the way out pressed, the click lands"
        );
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// The same dialog in a closed shadow root, which Neo's world reads once a marked snapshot
/// opened it: named, with its buttons.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_dialog_in_an_opened_closed_root_is_named_with_its_buttons() -> TestResult {
    let shape = Shape::shape("shadowclosed").await?;
    let host: Arc<dyn OpHost> = Arc::clone(&shape.run) as Arc<dyn OpHost>;
    let seen = js_runner::run_script(
        &host,
        &json!({ "page": brand(&shape.page) }),
        "await page.ariaSnapshot()
         try { await page.locator('#target').click({ timeout: 1500 }); return 'clicked' }
         catch (error) { return { code: error.code, message: error.message, cover: error.cover } }",
    )
    .await;
    shape.close().await;
    let seen = seen?;
    let message = seen
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert_eq!(seen.get("code"), Some(&json!("covered")), "{seen}");
    let cover = seen.get("cover").ok_or("error.cover")?;
    assert_eq!(
        (text(cover, "role"), text(cover, "name")),
        ("dialog", "Privacy choices"),
        "{message}"
    );
    assert!(message.contains("button \"Reject all\" [ref="), "{message}");
    assert!(!message.contains("blank layer"), "{message}");
    Ok(())
}

/// A blank loading layer beside a dialog that does not stand on it: the answer is the layer's,
/// and the dialog is not named.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_dialog_beside_a_blank_layer_is_not_named_for_it() -> TestResult {
    let shape = Shape::shape("beside").await?;
    let result = async {
        let (message, cover) = covered(shape.click("#target").await)?;
        assert_ne!(text(&cover, "name"), "Cookies", "{message}");
        assert!(!message.contains("Open over it"), "{message}");
        assert!(message.contains("Do: a blank layer. Look for an open dialog in a new snapshot. If there is none, reload."), "{message}");
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// Row 24: a fixed banner inside the page's `main`, over a control outside `main`. The cover's
/// root is `main` itself; the answer names the banner and never `main`. A second control under the
/// same banner, inside `main`, has the banner for its root, and its answer carries the same-layer
/// line though the two roots differ. A plain div placed inside `main` over a third control is
/// named itself, never `main`, and is not the banner's layer.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_banner_inside_main_is_named_and_never_main() -> TestResult {
    let shape = Shape::open("/covered/banner.html").await?;
    let result = async {
        let (first, cover) = covered(shape.click("#outside").await)?;
        assert_eq!(text(&cover, "name"), "Support us", "{first}");
        assert_ne!(text(&cover, "role"), "main", "{first}");
        assert!(!first.contains("The same layer"), "{first}");
        let (second, cover) = covered(shape.click("#inside").await)?;
        assert_eq!(text(&cover, "name"), "Support us", "{second}");
        assert!(
            second.contains("\n  The same layer covered your last click on this tab, "),
            "{second}"
        );
        let (third, cover) = covered(shape.click("#outside2").await)?;
        assert_eq!(text(&cover, "role"), "generic", "{third}");
        assert!(third.contains("<div id=\"promo\""), "{third}");
        assert!(!third.contains("The same layer"), "{third}");
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// The ref of the cover's control named `name`.
fn control_ref(cover: &Value, name: &str) -> TestResult<String> {
    let found = cover
        .get("controls")
        .and_then(Value::as_array)
        .and_then(|controls| {
            controls
                .iter()
                .find(|control| control.get("name") == Some(&json!(name)))
        })
        .and_then(|control| control.get("ref"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("the controls list {name}: {cover}"))?;
    Ok(found.to_owned())
}

/// A blank backdrop in the document and the dialog it dims drawn from a host's open shadow root,
/// standing on it: the answer names that dialog, whose button presses by its ref.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_backdrop_is_named_after_a_dialog_in_a_shadow_root() -> TestResult {
    let shape = Shape::shape("shadowbehind").await?;
    let result = async {
        let (message, cover) = covered(shape.click("#target").await)?;
        assert!(
            message.contains(". Open over it: dialog \"Newsletter\" [ref="),
            "{message}"
        );
        let later = control_ref(&cover, "Not now")?;
        let by_ref = locator(&shape.page, &[aria_ref(&later)], &later);
        shape.act("locator.click", &by_ref, json!({})).await?;
        shape.click("#target").await?;
        assert_eq!(
            shape.log().await?,
            ["target"],
            "the way out pressed, the click lands"
        );
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// The roles and names of a cover's controls, in the answer's order.
fn control_names(cover: &Value) -> TestResult<Vec<(String, String)>> {
    Ok(cover
        .get("controls")
        .and_then(Value::as_array)
        .ok_or("the cover lists its controls")?
        .iter()
        .map(|control| {
            (
                text(control, "role").to_owned(),
                text(control, "name").to_owned(),
            )
        })
        .collect())
}

/// A real sign-in sheet's first shape: a blank fixed layer on top at the click point, beside a
/// dialog named by a span with `tabindex="-1"` in its h2, holding a focus target with
/// `tabindex="-1"` off screen, its Close button and its frame. The answer names the dialog behind
/// the layer, its controls are exactly the button and the frame (role `iframe`), and the button's
/// ref closes it.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_sign_in_sheet_lists_only_its_ways_out() -> TestResult {
    let shape = Shape::shape("signinsheet").await?;
    let result = async {
        let (message, cover) = covered(shape.click("#target").await)?;
        assert!(
            message.contains(". Open over it: dialog \"Sign in or Sign up\" [ref="),
            "{message}"
        );
        assert_eq!(
            control_names(&cover)?,
            [
                ("button".to_owned(), "Close Sign in or Sign up".to_owned()),
                ("iframe".to_owned(), "Login/Signup".to_owned()),
            ],
            "{message}"
        );
        let close = control_ref(&cover, "Close Sign in or Sign up")?;
        let by_ref = locator(&shape.page, &[aria_ref(&close)], &close);
        shape.act("locator.click", &by_ref, json!({})).await?;
        shape.click("#target").await?;
        assert_eq!(
            shape.log().await?,
            ["target"],
            "the sheet closed, the click lands"
        );
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// A real sign-in sheet's second shape: a nameless portal in a carrier that takes no pointer,
/// holding a dark backdrop on top at the click point, two focus guards and a dialog labelled
/// "Log in or sign up" that holds only its frame. The answer shows that dialog: its role, name
/// and ref, and exactly one control, the frame; since that is its only control, the kind is
/// `frame` and the line points into the frame.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_nameless_portal_is_shown_as_its_dialog() -> TestResult {
    let shape = Shape::shape("portalsheet").await?;
    let result = async {
        let (message, cover) = covered(shape.click("#target").await)?;
        assert_eq!(
            (text(&cover, "role"), text(&cover, "name")),
            ("dialog", "Log in or sign up"),
            "{message}"
        );
        assert!(
            message.contains("\n  On top: dialog \"Log in or sign up\" [ref="),
            "{message}"
        );
        assert_eq!(
            control_names(&cover)?,
            [("iframe".to_owned(), "Log in or sign up".to_owned())],
            "{message}"
        );
        // Its one control is its frame, so its way out is in there: the frame's kind, with a
        // line that points into the frame, though the backdrop lies at the point.
        assert_eq!(text(&cover, "kind"), "frame", "{message}");
        assert!(
            message.contains(
                "\n  Do: its content is in a frame: it is under the frame's ref in the snapshot. If \
                 it is a bot check, use the solver."
            ),
            "{message}"
        );
        assert!(shape.resolves(text(&cover, "ref")).await?, "{message}");
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// The prelude's header under Neo's built-in timeout: Playwright's 30 s, stamped as nobody's
/// choice (covered-click B). The rows that call an op with it stand in for the prelude.
fn built_in() -> Value {
    json!({ "timeoutMs": 30_000, "timeoutBuiltIn": true })
}

impl Shape {
    /// A pointer op on `#target` whose header is `call`, as the prelude or a host sends it.
    async fn act_called(&self, op: &str, call: Value) -> Result<Value, PlaywrightError> {
        let mut call = call;
        if let Value::Object(fields) = &mut call {
            fields.insert("api".to_owned(), json!(op));
        }
        self.run
            .invoke(
                op,
                json!({ "call": call, "target": self.at("#target"), "strict": "strict" }),
            )
            .await
    }

    /// A script through the prelude on this shape's page.
    async fn script(&self, source: &str) -> TestResult<Value> {
        let host: Arc<dyn OpHost> = Arc::clone(&self.run) as Arc<dyn OpHost>;
        Ok(js_runner::run_script(&host, &json!({ "page": brand(&self.page) }), source).await?)
    }
}

/// The early stop's first line in its one fixed form: the api, then `Timeout cut short: the same
/// cover stayed on top for`, the seconds with one decimal, `s (`, the checks a whole number,
/// `checks).`. Answers the seconds and the checks.
fn stopped_early_form(message: &str, api: &str) -> TestResult<(f64, u64)> {
    let first = message.lines().next().unwrap_or_default();
    let rest = first
        .strip_prefix(&format!(
            "{api}: Timeout cut short: the same cover stayed on top for "
        ))
        .ok_or_else(|| format!("not the early stop's first line: {first}"))?;
    let (seconds, rest) = rest.split_once(" s (").ok_or("seconds, then ` s (`")?;
    let (whole, tenth) = seconds.split_once('.').ok_or("seconds with one decimal")?;
    let digits = |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit());
    if !digits(whole) || tenth.len() != 1 || !digits(tenth) {
        return Err(format!("seconds with one decimal: {first}").into());
    }
    let checks = rest
        .strip_suffix(" checks).")
        .filter(|checks| digits(checks))
        .ok_or_else(|| format!("a whole number of checks and a full stop: {first}"))?;
    Ok((seconds.parse()?, checks.parse()?))
}

/// Covered-click B, row 1: under Neo's built-in timeout (the prelude's stamp) a click under a
/// standing cover stops once the same cover stood 5 s and at least 4 checks: a `TimeoutError`
/// with the code `covered`, its first line in one fixed form, A's facts after it, nothing sent and
/// nothing pressed, and the cover field's `stoodMs` and `checks`. The same click from a script
/// that set no timeout does the same, so the prelude stamps it. With the rule taken out, or the
/// stamp never set, it waits 30 s.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_standing_cover_stops_a_click_after_five_seconds() -> TestResult {
    let shape = Shape::shape("dialog").await?;
    let result = async {
        // The run's calls before the click: the page's own `goto` marked the run sent.
        let before = shape.run.calls().len();
        let began = Instant::now();
        let error = shape
            .act_called("locator.click", built_in())
            .await
            .err()
            .ok_or("the click under the cover succeeded")?;
        let took = began.elapsed();
        // Early, not at 30 s: the bound only tells the two apart, never a tight one.
        assert!(
            took >= Duration::from_secs(5) && took < Duration::from_secs(15),
            "{took:?}"
        );
        assert_eq!(error.error_code(), Some(ErrorCode::Covered), "{error}");
        assert_eq!(error.timed_out(), TimedOut::Yes, "still a TimeoutError");
        let message = error.message();
        let (seconds, checks) = stopped_early_form(message, "locator.click")?;
        assert!(seconds >= 5.0 && checks >= 4, "{message}");
        in_order(
            message,
            &[
                "\n  On top: dialog \"What You Need To Know\"",
                "\n  Nothing was pressed.",
            ],
        )?;
        assert!(
            !shape.run.calls()[before..].contains(&EnvCall::MarkSent),
            "an early stop where no attempt sent input is not_sent"
        );
        let cover = error.cover_details().ok_or("error.cover")?;
        assert!(
            cover
                .get("stoodMs")
                .and_then(Value::as_u64)
                .is_some_and(|millis| millis >= 5_000),
            "{cover}"
        );
        assert_eq!(cover.get("checks").and_then(Value::as_u64), Some(checks));
        assert!(shape.log().await?.is_empty(), "the target saw no click");
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result?;
    let shape = Shape::shape("dialog").await?;
    let began = Instant::now();
    let seen = shape
        .script(
            "try { await page.locator('#target').click(); return 'clicked' }
             catch (error) { return { name: error.name, code: error.code, message: error.message } }",
        )
        .await;
    let took = began.elapsed();
    shape.close().await;
    let seen = seen?;
    assert_eq!(seen.get("name"), Some(&json!("TimeoutError")), "{seen}");
    assert_eq!(seen.get("code"), Some(&json!("covered")), "{seen}");
    stopped_early_form(
        seen.get("message")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "locator.click",
    )?;
    assert!(
        took < Duration::from_secs(15),
        "the prelude stamped it: it stopped early, not at 30 s: {took:?}"
    );
    Ok(())
}

/// Covered-click B, row 2: a timeout somebody chose runs to its end under a standing cover, with
/// the facts: an op sent with 8 s and with 2 s and no stamp, as a host's own call or a script's
/// chosen timeout arrives, and one sent with no timeout at all, still waiting at 8 s.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_chosen_timeout_keeps_its_whole_wait() -> TestResult {
    let shape = Shape::shape("dialog").await?;
    let result = async {
        for millis in [8_000_u64, 2_000] {
            let began = Instant::now();
            let (message, _) = covered(
                shape
                    .act_called("locator.click", json!({ "timeoutMs": millis }))
                    .await,
            )?;
            let took = began.elapsed();
            assert!(took >= Duration::from_millis(millis), "{millis}: {took:?}");
            assert!(
                message.starts_with(&format!("locator.click: Timeout {millis}ms exceeded.")),
                "{message}"
            );
        }
        let unlimited = time::timeout(
            Duration::from_secs(8),
            shape.act_called("locator.click", json!({})),
        )
        .await;
        assert!(
            unlimited.is_err(),
            "a call with no timeout is still waiting at 8 s: {unlimited:?}"
        );
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// Covered-click B: under the spec harness's conformance switch a covered wait never stops early,
/// though the prelude stamps the call there too (`headerTimeout` does not know the switch, and the
/// bridge adds it to the same header). Only `Waiting::of`'s conformance test keeps the wait whole,
/// so this row sends the stamp with the switch and finds the click still waiting at 12 s. The
/// window is a lower bound; its width only gives a wrong early stop at 5 s room to show.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_conformance_never_stops_early() -> TestResult {
    let shape = Shape::shape("dialog").await?;
    let mut call = built_in();
    call["conformance"] = json!(true);
    let waiting = time::timeout(
        Duration::from_secs(12),
        shape.act_called("locator.click", call),
    )
    .await;
    shape.close().await;
    assert!(
        waiting.is_err(),
        "stamped under the conformance switch, the click is still waiting at 12 s: {waiting:?}"
    );
    Ok(())
}

/// Covered-click B: a default anyone but Neo's own prelude set is a choice. A script that sets
/// `page.setDefaultTimeout(30_000)`, `context.setDefaultTimeout(30_000)` or `timeout` on the call
/// keeps the whole wait under a cover that stands until 8 s: the click lands after it, with one
/// trusted click. With the stamp always set, each stops at 5 s.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_timeout_set_by_hand_keeps_the_whole_wait() -> TestResult {
    for (setup, click) in [
        ("page.setDefaultTimeout(30_000);", "click()"),
        ("page.context().setDefaultTimeout(30_000);", "click()"),
        ("", "click({ timeout: 30_000 })"),
    ] {
        let shape = Shape::shape("leaves8").await?;
        let began = Instant::now();
        let seen = shape
            .script(&format!(
                "{setup} await page.locator('#target').{click}; return 'clicked'"
            ))
            .await;
        let took = began.elapsed();
        let log = shape.log().await;
        shape.close().await;
        assert_eq!(seen?, json!("clicked"), "{setup} {click}");
        assert!(
            took >= Duration::from_millis(6_500),
            "{setup} {click}: {took:?}"
        );
        assert_eq!(log?, ["target"], "{setup} {click}");
    }
    Ok(())
}

/// Covered-click B, rows 3 and 4: a cover that leaves by itself. Gone at 3 s, the click lands
/// under the built-in timeout. Gone at 12 s, the built-in timeout stops first (its first line
/// says so), and a chosen 20 s lands when it leaves. The cover leaves well after the stop, so a
/// loaded machine's slow first check cannot let the click land first.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_cover_that_leaves_lets_the_click_land_in_time() -> TestResult {
    let shape = Shape::shape("leaves3").await?;
    let landed = shape.act_called("locator.click", built_in()).await;
    let log = shape.log().await;
    shape.close().await;
    landed?;
    assert_eq!(log?, ["target"], "gone at 3 s: one trusted click");
    let shape = Shape::shape("leaves12").await?;
    let early = shape.act_called("locator.click", built_in()).await;
    shape.close().await;
    let (message, _) = covered(early)?;
    stopped_early_form(&message, "locator.click")?;
    let shape = Shape::shape("leaves12").await?;
    let landed = shape
        .act_called("locator.click", json!({ "timeoutMs": 20_000 }))
        .await;
    let log = shape.log().await;
    shape.close().await;
    landed?;
    assert_eq!(log?, ["target"], "a chosen 20 s lands at 12 s");
    Ok(())
}

/// Covered-click B, row 5: a cover that moves, fades, says it is loading or holds a progress bar
/// keeps the whole wait under the built-in timeout: the click lands when it leaves, at 7 or 8 s.
/// The moving cover's place never repeats while it stands: a move with a period can coincide with
/// the checks' phase (a check every 500 ms plus its own cost), see one place at every check, and
/// stop the click early for a cover that never stood still.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_cover_that_moves_fades_or_loads_keeps_the_wait() -> TestResult {
    each_shape(
        &["moving", "fading", "busy", "progress"],
        |shape, name| async move {
            let result = async {
                let began = Instant::now();
                shape.act_called("locator.click", built_in()).await?;
                let took = began.elapsed();
                let log = shape.log().await?;
                if log != ["target"] || took < Duration::from_millis(6_000) {
                    return Err(format!("{name}: {log:?} after {took:?}").into());
                }
                TestResult::Ok(())
            }
            .await;
            (shape, result)
        },
    )
    .await
}

/// Covered-click B, row 6: one cover replaced by another at 3 s starts the count again: the stop
/// comes 5 s after the second arrived, and names it.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_replaced_cover_starts_the_count_again() -> TestResult {
    let shape = Shape::shape("replaced").await?;
    let result = async {
        let began = Instant::now();
        let (message, cover) = covered(shape.act_called("locator.click", built_in()).await)?;
        let took = began.elapsed();
        stopped_early_form(&message, "locator.click")?;
        assert!(
            took >= Duration::from_millis(7_500) && took < Duration::from_secs(25),
            "{took:?}"
        );
        assert_eq!(text(&cover, "name"), "Second notice", "{message}");
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// Covered-click B, row 7: a cover that arrives with the press's own move: the press is stopped
/// at the window, the page saw no click, and the stop comes 5 s later, saying when it came.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_cover_that_came_with_the_press_stops_later() -> TestResult {
    let shape = Shape::shape("late").await?;
    let result = async {
        let (message, _) = covered(shape.act_called("locator.click", built_in()).await)?;
        stopped_early_form(&message, "locator.click")?;
        assert!(message.contains("\n  It appeared "), "{message}");
        assert!(shape.log().await?.is_empty(), "the target saw no click");
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}

/// Covered-click B: an early stop leaves through the timeout's own exit. A drag whose drop target
/// stands under a cover stops after 5 s under the built-in timeout, and the button the drag still
/// held is released, as on a timeout: the window saw the trusted press on the source, then the
/// release.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_drag_stopped_early_releases_its_button() -> TestResult {
    let shape = Shape::shape("dragcovered").await?;
    let result = async {
        let began = Instant::now();
        let input = json!({
            "call": { "api": "locator.dragTo", "timeoutMs": 30_000, "timeoutBuiltIn": true },
            "target": shape.at("#src"), "strict": "strict", "to": shape.at("#dst"), "steps": 5
        });
        let (message, _) = covered(shape.run.invoke("locator.dragTo", input).await)?;
        let took = began.elapsed();
        stopped_early_form(&message, "locator.dragTo")?;
        assert!(
            took < Duration::from_secs(15),
            "early, not at 30 s: {took:?}"
        );
        // The owed release goes out once the op has let the tab's input go. It comes within
        // milliseconds; the poll allows 10 s so a loaded machine cannot fail the row.
        let mut log = Vec::new();
        for _ in 0..100 {
            log = shape.log().await?;
            if log.contains(&"mouseup".to_owned()) {
                break;
            }
            time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(log, ["mousedown", "mouseup"], "the press, then its release");
        TestResult::Ok(())
    }
    .await;
    shape.close().await;
    result
}
