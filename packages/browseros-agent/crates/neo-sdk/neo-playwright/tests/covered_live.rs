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
use live_run::{LiveRun, TestPage, TestResult, brand};
use locator::{aria_ref, css, enter_frame, locator};
use neo_playwright::{ErrorCode, PlaywrightError, TimedOut};
use serde_json::{Value, json};

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
    for (name, kind, line) in [
        ("frame", "frame", "Do: a frame covers it."),
        ("partial", "partial", "Do: only part of it is covered."),
        (
            "label",
            "label",
            "Do: it sits under its own label. Press Space on it.",
        ),
        (
            "wrapper",
            "wrapper",
            "Do: its own wrapper takes the pointer.",
        ),
        (
            "labels",
            "labels",
            "Do: it labels the control on top. Act on that control:",
        ),
        (
            "nothing",
            "nothing",
            "Do: it takes no pointer at its centre.",
        ),
        ("folded", "folded", "Do: it is not shown where it lies"),
    ] {
        let shape = Shape::shape(name).await?;
        let result = async {
            let (message, cover) = covered(shape.click("#target").await)
                .map_err(|error| format!("{name}: {error}"))?;
            assert_eq!(text(&cover, "kind"), kind, "{name}: {message}");
            assert!(message.contains(line), "{name}: {message}");
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

/// The ways out the lines give land: a click at the clear point, Space on a control under its
/// own label, and a click on a label with links at the point given, which follows none.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_the_ways_out_land() -> TestResult {
    let position = |message: &str| -> TestResult<Value> {
        let after = message
            .split("{ position: ")
            .nth(1)
            .ok_or("a position in the line")?;
        let inner = after.split(" }").next().unwrap_or_default();
        let number = |name: &str| -> TestResult<f64> {
            Ok(inner
                .split(&format!("{name}: "))
                .nth(1)
                .and_then(|rest| rest.split(',').next())
                .ok_or("a coordinate")?
                .trim()
                .parse()?)
        };
        Ok(json!({ "x": number("x")?, "y": number("y")? }))
    };
    let partial = Shape::shape("partial").await?;
    let result = async {
        let (message, _) =
            covered(partial.click("#target").await).map_err(|error| format!("partial: {error}"))?;
        let at = position(&message)?;
        partial
            .act(
                "locator.click",
                &partial.at("#target"),
                json!({ "position": at }),
            )
            .await?;
        assert_eq!(partial.log().await?, ["target"]);
        TestResult::Ok(())
    }
    .await;
    partial.close().await;
    result?;

    let label = Shape::shape("label").await?;
    let result = async {
        covered(label.click("#target").await)?;
        label
            .act(
                "locator.press",
                &label.at("#target"),
                json!({ "key": "Space" }),
            )
            .await?;
        assert_eq!(
            label
                .run
                .eval(&label.page, "document.getElementById('target').checked")
                .await?,
            json!(true)
        );
        TestResult::Ok(())
    }
    .await;
    label.close().await;
    result?;

    let links = Shape::shape("links").await?;
    let result = async {
        let (message, cover) = covered(links.click("#target").await)?;
        assert!(message.contains("The label holds 2 links."), "{message}");
        let at = position(&message)?;
        let label = locator(&links.page, &[aria_ref(text(&cover, "ref"))], "the label");
        links
            .act("locator.click", &label, json!({ "position": at }))
            .await?;
        assert_eq!(
            links
                .run
                .eval(&links.page, "document.getElementById('target').checked")
                .await?,
            json!(true)
        );
        assert_eq!(
            links.run.eval(&links.page, "location.pathname").await?,
            json!("/covered/covered.html"),
            "the page stayed"
        );
        TestResult::Ok(())
    }
    .await;
    links.close().await;
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

/// A consent tool's host whose open shadow root draws a backdrop and a dialog, the target under
/// the backdrop: the host is not blank, its snapshot shows the dialog, and the dialog's buttons are
/// its controls, with refs that resolve and press.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tests_that_a_dialog_in_an_open_shadow_root_is_named_with_its_buttons() -> TestResult {
    let shape = Shape::shape("shadowopen").await?;
    let result = async {
        let (message, cover) = covered(shape.click("#target").await)?;
        assert!(message.contains("<div id=\"consent\""), "{message}");
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
