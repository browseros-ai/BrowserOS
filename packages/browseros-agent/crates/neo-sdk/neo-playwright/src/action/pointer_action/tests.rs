use std::time::Duration;

use super::{MAX_TIMER_MS, arm_expiry, own_line, unchanged};
use crate::input::{ClickOptions, InputDevices, Modifiers, Platform, Point};

#[test]
fn tests_that_an_interceptor_outlives_every_event_its_op_can_still_send() {
    let mut devices = InputDevices::new(Platform::Linux);
    let slow = devices.mouse.click(
        Point { x: 5.0, y: 5.0 },
        ClickOptions {
            click_count: 2,
            delay: Duration::from_millis(3500),
            ..ClickOptions::default()
        },
        Modifiers::default(),
    );
    assert_eq!(
        arm_expiry(Some(Duration::from_secs(5)), &slow),
        8000,
        "3 s past the op's timeout, after which the op sends nothing"
    );
    assert_eq!(
        arm_expiry(None, &slow),
        13_500,
        "an op without a timeout: 3 s past its burst's three 3.5 s delays"
    );
    assert_eq!(
        arm_expiry(Some(Duration::from_secs(u64::MAX)), &slow),
        MAX_TIMER_MS,
        "a timeout too long for setTimeout, which would fire at once, is the longest it keeps"
    );
}

/// Covered-click C: the call log's one line per case, in the fixed form a host counts with one
/// match, and a checking action's error that names which of the target's own took the press.
#[test]
fn tests_that_a_press_of_the_targets_own_says_which_in_one_form() {
    use crate::probe::{OwnCase, OwnPress, Point as FramePoint};

    let press = |case, markup: Option<&str>, position| OwnPress {
        case,
        markup: markup.map(str::to_owned),
        position,
        checked: None,
    };
    assert_eq!(
        own_line(&press(
            OwnCase::Label,
            Some("<label for=\"terms\">…</label>"),
            None
        )),
        "  its own label takes the press: <label for=\"terms\">…</label>"
    );
    assert_eq!(
        own_line(&press(
            OwnCase::Wrapper,
            Some("<div class=\"option\">…</div>"),
            None
        )),
        "  its own wrapper takes the press: <div class=\"option\">…</div>"
    );
    assert_eq!(
        own_line(&press(
            OwnCase::Point,
            None,
            Some(FramePoint { x: 12.4, y: 7.6 })
        )),
        "  a clear point of it takes the press: { x: 12, y: 8 }"
    );
    assert_eq!(
        unchanged(None),
        "Clicking the checkbox did not change its state"
    );
    assert_eq!(
        unchanged(Some(OwnCase::Label)),
        "Clicking the checkbox did not change its state: its own label took the press"
    );
    assert_eq!(
        unchanged(Some(OwnCase::Wrapper)),
        "Clicking the checkbox did not change its state: its own wrapper took the press"
    );
}

/// The rule that sends a burst into an out-of-process frame on that frame's own session
/// (`FrameInput`): only where the browser would otherwise pick the frame, at a pointer scale of 1,
/// with a point the placement gave in that frame.
#[test]
fn tests_that_only_a_browser_routed_unscaled_point_in_a_frame_takes_the_frames_session() {
    use super::frame_shift;

    let page = Point { x: 28.5, y: 130.25 };
    let own = Some(Point { x: 20.5, y: 10.25 });
    assert_eq!(
        frame_shift(true, 1.0, own, page),
        Some(Point { x: -8.0, y: -120.0 }),
        "an element the browser would route to: its frame's own pixels"
    );
    assert_eq!(
        frame_shift(false, 1.0, own, page),
        None,
        "the main frame, a same-process frame of it, a drop target or the touch preview"
    );
    assert_eq!(
        frame_shift(true, 1.5, own, page),
        None,
        "a scaled tab, whose frames' pixels its sessions are not known to share"
    );
    assert_eq!(
        frame_shift(true, 1.0, None, page),
        None,
        "a point only the browser could map"
    );
}

mod sessions {
    //! Which session carries a pointer burst, scripted end to end over [`ScriptedTab`]: a click
    //! in the main frame and in a same-process frame of it goes on the tab's root session at the
    //! page point, one in an out-of-process frame on that frame's own session at its own point,
    //! and `force`, a drag and a burst with moves on the way stay on the root with the route
    //! check (`moves` asked).

    use std::mem;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Mutex, PoisonError};

    use browseros_core::SessionId;
    use serde_json::{Value, json};

    use crate::Op;
    use crate::test_support::{
        ProbeAnswer, SCRIPTED_CHILD, SCRIPTED_MAIN, SCRIPTED_OOPIF, ScriptedRun, ScriptedTab,
        TabScript, TestResult,
    };

    /// The button's point in its own frame, and its frame's content-box origin in the page.
    const POINT: (f64, f64) = (20.5, 10.25);
    const ORIGIN: (f64, f64) = (8.0, 120.0);

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Lives {
        Main,
        Child,
        Oopif,
    }

    /// One mouse event as the tab saw it: its type, position and session.
    #[derive(Debug, PartialEq)]
    struct Seen {
        kind: String,
        x: f64,
        y: f64,
        on_oopif: bool,
    }

    /// A tab whose button lives in `lives`, answering every probe step of a click on it.
    struct FrameButton {
        lives: Lives,
        moves: AtomicU64,
        asked_moves: AtomicUsize,
        mouse: Mutex<Vec<Seen>>,
    }

    impl FrameButton {
        fn new(lives: Lives) -> Self {
            Self {
                lives,
                moves: AtomicU64::default(),
                asked_moves: AtomicUsize::default(),
                mouse: Mutex::default(),
            }
        }

        fn frame(&self) -> &'static str {
            match self.lives {
                Lives::Main => SCRIPTED_MAIN,
                Lives::Child => SCRIPTED_CHILD,
                Lives::Oopif => SCRIPTED_OOPIF,
            }
        }

        fn mouse(&self) -> Vec<Seen> {
            mem::take(&mut *self.mouse.lock().unwrap_or_else(PoisonError::into_inner))
        }
    }

    impl TabScript for FrameButton {
        fn child(&self) -> bool {
            self.lives == Lives::Child
        }

        fn oopif(&self) -> bool {
            self.lives == Lives::Oopif
        }

        fn probe(&self, frame: &str, request: &Value) -> ProbeAnswer {
            let rect =
                json!({ "x": POINT.0 - 10.0, "y": POINT.1 - 5.0, "width": 20, "height": 10 });
            let point = json!({ "x": POINT.0, "y": POINT.1 });
            match request["op"].as_str() {
                // The main frame's iframe, on the way down to the button's frame.
                Some("resolve") if frame == SCRIPTED_MAIN && self.lives != Lives::Main => {
                    ProbeAnswer::reply(json!({ "kind": "frame", "pin": 1, "key": "e1",
                        "visible": true, "src": "https://pay.a.test/" }))
                }
                Some("act") if frame == self.frame() && request["arm"].is_object() => {
                    ProbeAnswer::reply(json!({ "kind": "armed", "rect": rect, "point": point,
                        "token": request["arm"]["token"] }))
                }
                Some("act") if frame == self.frame() => ProbeAnswer::reply(json!({
                    "kind": "measured", "pin": 3, "rect": rect, "point": point,
                    "inView": true, "transformed": false })),
                Some("frameBox") if frame == SCRIPTED_MAIN => {
                    let hit = if request["point"].is_object() {
                        json!("done")
                    } else {
                        Value::Null
                    };
                    ProbeAnswer::reply(json!({ "kind": "frameBox",
                        "origin": { "x": ORIGIN.0, "y": ORIGIN.1 }, "hit": hit, "unseen": false }))
                }
                Some("verdict" | "disarm") => {
                    ProbeAnswer::reply(json!({ "kind": "verdict", "hit": "done" }))
                }
                // The route check: every read counts one move more than the one before.
                Some("moves") => {
                    self.asked_moves.fetch_add(1, Ordering::SeqCst);
                    let moves = self.moves.fetch_add(1, Ordering::SeqCst);
                    ProbeAnswer::reply(json!({ "kind": "moves", "moves": moves }))
                }
                _ => ProbeAnswer::unexpected(),
            }
        }

        fn seen(&self, method: &str, params: &Value, session: Option<&SessionId>) {
            if method != "Input.dispatchMouseEvent" {
                return;
            }
            let on_oopif =
                session.is_some_and(|session| session.as_str().ends_with(SCRIPTED_OOPIF));
            self.mouse
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(Seen {
                    kind: params["type"].as_str().unwrap_or_default().to_owned(),
                    x: params["x"].as_f64().unwrap_or(f64::NAN),
                    y: params["y"].as_f64().unwrap_or(f64::NAN),
                    on_oopif,
                });
        }

        fn cdp_reply(&self, method: &str, params: &Value) -> Option<Value> {
            if method == "Runtime.callFunctionOn"
                && params["arguments"][0]["value"]["request"]["op"] == "pinned"
            {
                return Some(json!({ "result": { "type": "object", "subtype": "node",
                    "objectId": "iframe-owner" } }));
            }
            if method == "DOM.describeNode" && params["objectId"] == "iframe-owner" {
                let mut node = json!({ "backendNodeId": 1, "nodeName": "IFRAME",
                    "frameId": self.frame() });
                if self.lives == Lives::Child {
                    node["contentDocument"] = json!({ "backendNodeId": 2 });
                }
                return Some(json!({ "node": node }));
            }
            None
        }
    }

    /// `locator('iframe').contentFrame().getByRole('button')`, or the button itself in the main
    /// frame.
    fn button(run: &ScriptedRun, lives: Lives) -> Value {
        let part = |selector: &str| json!({ "name": "css", "body": selector, "source": selector });
        let chunks = if lives == Lives::Main {
            json!([{ "parts": [part("button")] }])
        } else {
            json!([{ "parts": [part("iframe")] }, { "parts": [part("button")] }])
        };
        json!({ "$neo": "locator", "page": run.page, "frame": null, "chunks": chunks,
            "display": "the button" })
    }

    /// Runs `api` (`locator.click`, `locator.dragTo`) on the button with `options`, and returns the
    /// mouse events the tab saw and whether the route check asked for moves.
    async fn act(lives: Lives, api: &str, options: Value) -> TestResult<(Vec<Seen>, bool)> {
        let tab = ScriptedTab::new(FrameButton::new(lives));
        let run = ScriptedRun::over(&tab).await?;
        let target = button(&run, lives);
        let mut input = json!({ "call": { "api": api, "timeoutMs": 2000 }, "target": target,
            "strict": "strict" });
        if api == "locator.dragTo" {
            input["to"] = button(&run, lives);
        }
        if let (Some(fields), Value::Object(options)) = (input.as_object_mut(), options) {
            fields.extend(options);
        }
        let op = Op::from_name(api).ok_or("a pointer op")?;
        run.playwright.invoke(&run.env, op, input).await?;
        let asked = tab.script().asked_moves.load(Ordering::SeqCst) > 0;
        Ok((tab.script().mouse(), asked))
    }

    fn at(kind: &str, x: f64, y: f64, on_oopif: bool) -> Seen {
        Seen {
            kind: kind.to_owned(),
            x,
            y,
            on_oopif,
        }
    }

    /// A one-point click's three events at one place on one session.
    fn click_at(x: f64, y: f64, on_oopif: bool) -> Vec<Seen> {
        ["mouseMoved", "mousePressed", "mouseReleased"]
            .into_iter()
            .map(|kind| at(kind, x, y, on_oopif))
            .collect()
    }

    const PAGE: (f64, f64) = (POINT.0 + ORIGIN.0, POINT.1 + ORIGIN.1);

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_click_in_the_main_frame_goes_on_the_root_session() -> TestResult {
        let (mouse, asked) = act(Lives::Main, "locator.click", json!({})).await?;
        assert_eq!(
            mouse,
            click_at(POINT.0, POINT.1, false),
            "the page point, on the root"
        );
        assert!(!asked, "the page picks no frame there: no route check");
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_click_in_a_same_process_frame_goes_on_the_root_session() -> TestResult {
        let (mouse, asked) = act(Lives::Child, "locator.click", json!({})).await?;
        assert_eq!(
            mouse,
            click_at(PAGE.0, PAGE.1, false),
            "the page point, on the root"
        );
        assert!(!asked, "one renderer hit-tests it: no route check");
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_click_in_an_out_of_process_frame_goes_on_its_own_session() -> TestResult {
        let (mouse, asked) = act(Lives::Oopif, "locator.click", json!({})).await?;
        assert_eq!(
            mouse,
            click_at(POINT.0, POINT.1, true),
            "the frame's own point, on the frame's own session"
        );
        assert!(!asked, "nothing above it can take it: no route check");
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_forced_click_in_an_out_of_process_frame_stays_on_the_root() -> TestResult
    {
        let (mouse, asked) = act(Lives::Oopif, "locator.click", json!({ "force": true })).await?;
        assert_eq!(
            mouse,
            click_at(PAGE.0, PAGE.1, false),
            "it skipped the parents' hit tests, so a page above must still take its input"
        );
        assert!(asked, "the route check before its press");
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_click_with_moves_on_the_way_stays_on_the_root() -> TestResult {
        let (mouse, asked) = act(Lives::Oopif, "locator.click", json!({ "steps": 3 })).await?;
        assert!(
            mouse.iter().all(|seen| !seen.on_oopif),
            "its moves cross the frames between: all on the root, {mouse:?}"
        );
        assert_eq!(
            mouse
                .iter()
                .filter(|seen| seen.kind == "mouseMoved")
                .count(),
            3,
            "three moves on the way"
        );
        assert!(asked, "the route check before its press");
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_drag_in_an_out_of_process_frame_stays_on_the_root() -> TestResult {
        let (mouse, asked) = act(Lives::Oopif, "locator.dragTo", json!({})).await?;
        assert!(
            !mouse.is_empty() && mouse.iter().all(|seen| !seen.on_oopif),
            "its button is held across two bursts and let go on the root: {mouse:?}"
        );
        assert!(asked, "the route check before its press");
        Ok(())
    }
}
