//! Input from many calls and runs on one tab of a fresh headless BrowserOS neo: every input
//! call takes the tab's input turn (`page.rs` `TabInputs`, shared by the runs of one `Runtime`),
//! so fills, key presses, checks, selections and clicks never split one another's input, turns
//! go in arrival order, a call that stops waiting for its turn sends nothing, a press cut short
//! still releases its keys, a fill cut short mid-insertText still inserts its whole value
//! before the next input, and a run's end releases its keys while a frame of the tab is busy.
//! Behaviour only: page-seen events, values and delivery records.

#[expect(
    dead_code,
    reason = "shared fixture includes navigation helpers other suites use"
)]
#[path = "support/browser.rs"]
mod browser;

use std::ffi::OsString;
use std::time::Duration;
use std::time::Instant;

use browser::{Fixture, Tab};
use neo_runtime::{CallResult, Delivery, PageId, RunOutcome, RunRequest, RunStatus, Runtime};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio::time;
use tokio_util::sync::CancellationToken;

/// Every control the tests type into, and a recorder of the events input produces, by target.
/// No `change`: a text field fires it when it loses focus, during the next call's input.
/// `#hold` marks `#flag` when its press lands, so a script can see that click holding the turn.
const PAGE: &str = r#"<!doctype html><html><body>
<button id='hold' onmousedown="document.getElementById('flag').value = 'held'">Hold</button>
<button id='c'>Click</button>
<input id='a'><input id='b'><input id='f'><input id='p'><input id='late'><input id='flag'>
<input id='cb' type='checkbox'>
<select id='s'><option value='one'>One</option><option value='two'>Two</option></select>
<script>
window.__events = [];
for (const type of ['mousedown', 'mouseup', 'click', 'keydown', 'keypress', 'keyup', 'beforeinput', 'input'])
  addEventListener(type, e => __events.push({ type: e.type, target: e.target.id, key: e.key ?? null, data: e.data ?? null, shift: e.shiftKey ?? null }), true);
</script>
</body></html>"#;

async fn open() -> Option<Tab> {
    let binary = browser::e2e_browser()?;
    Some(
        Tab::open(
            binary,
            Fixture {
                pages: vec![("/inputs.html".into(), PAGE.into())],
                ..Fixture::default()
            },
            "/inputs.html",
        )
        .await,
    )
}

/// One `Runtime` for the tab, whose runs share its input turns as one server's runs do.
async fn runtime(tab: &Tab) -> Runtime {
    Runtime::connect(tab.cdp.clone())
        .await
        .unwrap_or_else(|error| panic!("Runtime::connect: {error}"))
}

fn request(tab: &Tab, code: &str, timeout: Duration, cancel: CancellationToken) -> RunRequest {
    RunRequest {
        code: code.to_string(),
        page: PageId::new(tab.target_id.clone()),
        timeout,
        cancel,
        artifacts: None,
        hooks: None,
    }
}

fn assert_ok(outcome: &RunOutcome) {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
}

/// The recorded events, as `(target, type, data)`.
fn events(recorded: &Value) -> Vec<(String, String, Value)> {
    recorded
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|event| {
            (
                event["target"].as_str().unwrap_or_default().to_string(),
                event["type"].as_str().unwrap_or_default().to_string(),
                event["data"].clone(),
            )
        })
        .collect()
}

/// Two runs started together on one tab, one filling `#a` and one pressing `Shift+B` into `#b`,
/// ten times each: every fill lands whole in `#a`, every `B` in `#b`, and neither field ever
/// sees the other's input.
#[tokio::test(flavor = "multi_thread")]
async fn two_runs_typing_into_two_fields_never_mix() {
    let Some(tab) = open().await else { return };
    let runtime = runtime(&tab).await;
    let filling = "for (let i = 0; i < 10; i++) await page.locator('#a').fill('alpha' + i)";
    let pressing = "for (let i = 0; i < 10; i++) await page.locator('#b').press('Shift+B')";
    let timeout = Duration::from_secs(30);
    let (filled, pressed) = tokio::join!(
        runtime.run(request(&tab, filling, timeout, CancellationToken::new())),
        runtime.run(request(&tab, pressing, timeout, CancellationToken::new())),
    );
    let values = tab
        .evaluate("[document.querySelector('#a').value, document.querySelector('#b').value]")
        .await;
    let recorded = events(&tab.evaluate("window.__events").await);
    tab.close().await;

    assert_ok(&filled);
    assert_ok(&pressed);
    assert_eq!(values, json!(["alpha9", "BBBBBBBBBB"]));
    for (target, kind, data) in &recorded {
        match target.as_str() {
            "a" => assert!(
                !kind.starts_with("key")
                    && (data.is_null()
                        || data.as_str().is_some_and(|data| data.starts_with("alpha"))),
                "{kind} {data} reached #a: {recorded:?}"
            ),
            "b" => assert!(
                data.is_null() || *data == json!("B"),
                "{kind} {data} reached #b: {recorded:?}"
            ),
            other => panic!("input reached #{other}: {recorded:?}"),
        }
    }
    let typed: Vec<_> = recorded
        .iter()
        .filter(|(target, kind, _)| target == "b" && kind == "input")
        .collect();
    assert_eq!(typed.len(), 10, "{recorded:?}");
    for outcome in [&filled, &pressed] {
        assert!(
            outcome
                .calls
                .iter()
                .all(|call| call.delivery == Some(Delivery::Done)),
            "{:#?}",
            outcome.calls
        );
    }
}

/// A press cancelled in the middle of its 500 ms `delay` still sends both keyUps: the page sees
/// no key left down, the next press types an unshifted `a`, and the record's delivery is the
/// writer's (every key answered).
#[tokio::test(flavor = "multi_thread")]
async fn a_press_cancelled_mid_delay_still_releases_its_keys() {
    let Some(tab) = open().await else { return };
    let runtime = runtime(&tab).await;
    let cancel = CancellationToken::new();
    // Cancel once the page has seen both keyDowns: the press is then inside its `delay`, however
    // long the run took to reach its first page call. A fixed sleep raced the run's setup.
    let stopper = async {
        let started = Instant::now();
        while tab
            .evaluate("window.__events.filter(event => event.type === 'keydown').length >= 2")
            .await
            != json!(true)
        {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "the press never reached its delay"
            );
            time::sleep(Duration::from_millis(10)).await;
        }
        cancel.cancel();
    };
    let (cut, ()) = tokio::join!(
        runtime.run(request(
            &tab,
            "await page.locator('#p').press('Shift+a', { delay: 500 })",
            Duration::from_secs(30),
            cancel.clone(),
        )),
        stopper,
    );
    let next = runtime
        .run(request(
            &tab,
            "await page.locator('#p').press('a')",
            Duration::from_secs(30),
            CancellationToken::new(),
        ))
        .await;
    let recorded = tab.evaluate("window.__events").await;
    let value = tab.evaluate("document.querySelector('#p').value").await;
    tab.close().await;

    assert_eq!(cut.status, RunStatus::Cancelled, "{cut:#?}");
    let [press] = cut.calls.as_slice() else {
        panic!("one press: {cut:#?}");
    };
    assert_eq!(
        press.delivery,
        Some(Delivery::Done),
        "both keyDowns and both keyUps were answered: {press:?}"
    );
    assert_ok(&next);
    let keys: Vec<(String, bool)> = recorded
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|event| event["type"] == "keydown" || event["type"] == "keyup")
        .map(|event| {
            (
                format!(
                    "{} {}",
                    event["type"].as_str().unwrap_or_default(),
                    event["key"].as_str().unwrap_or_default()
                ),
                event["shift"].as_bool().unwrap_or_default(),
            )
        })
        .collect();
    let downs = keys
        .iter()
        .filter(|(key, _)| key.starts_with("keydown"))
        .count();
    let ups = keys
        .iter()
        .filter(|(key, _)| key.starts_with("keyup"))
        .count();
    assert_eq!(downs, ups, "no key is left down: {keys:?}");
    assert_eq!(
        keys.last().cloned(),
        Some(("keyup a".to_string(), false)),
        "{keys:?}"
    );
    assert_eq!(
        value,
        json!("aa"),
        "a typed while Shift was held, then a without it"
    );
}

/// While a click holds the tab's input through its 2.5 s press, a click, a fill, a check, a
/// selection and a press queue in that order. Each one's input reaches the page whole and after
/// the one before it, and a press that gives up waiting for its turn sends nothing.
///
/// Arrival order is the order the calls ask for the turn, after their own element waits. The
/// queue starts only once the holding click is pressing (its `mousedown` marks `#flag`, read
/// with `inputValue`, which takes no turn), and each call starts 300 ms after the one before,
/// far longer than any of them takes to find its element.
#[tokio::test(flavor = "multi_thread")]
async fn queued_inputs_take_turns_in_arrival_order() {
    let Some(tab) = open().await else { return };
    let outcome = tab
        .start_run(
            r"const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const hold = page.locator('#hold').click({ delay: 2500 });
while (await page.locator('#flag').inputValue() !== 'held') await pause(10);
const click = page.locator('#c').click();
await pause(300);
const fill = page.locator('#f').fill('typed');
await pause(300);
const check = page.locator('#cb').check();
await pause(300);
const select = page.locator('#s').selectOption('two');
await pause(300);
const press = page.locator('#p').press('x');
await pause(300);
const late = page.locator('#late').press('y', { timeout: 100 }).then(() => 'resolved', error => error.name);
const results = await Promise.all([hold, click, fill, check, select, press, late]);
return results.map(result => result === undefined ? 'undefined' : result);"
                .to_string(),
        )
        .await
        .await
        .unwrap_or_else(|error| panic!("the run's task: {error}"));
    let recorded = events(&tab.evaluate("window.__events").await);
    let values = tab
        .evaluate("['#f', '#p', '#cb', '#s', '#late'].map(id => document.querySelector(id)).map(e => e.type === 'checkbox' ? e.checked : e.value)")
        .await;
    tab.close().await;

    assert_ok(&outcome);
    assert_eq!(
        outcome.value,
        Some(json!([
            "undefined",
            "undefined",
            "undefined",
            "undefined",
            ["two"],
            "undefined",
            "TimeoutError"
        ]))
    );
    assert_eq!(values, json!(["typed", "x", true, "two", ""]));
    let mut groups: Vec<&str> = Vec::new();
    for (target, _, _) in &recorded {
        if groups.last() != Some(&target.as_str()) {
            groups.push(target);
        }
    }
    assert_eq!(
        groups,
        ["hold", "c", "f", "cb", "s", "p"],
        "each call's input whole, in arrival order: {recorded:?}"
    );
    let late = outcome
        .calls
        .iter()
        .find(|call| call.params["selector"] == "#late")
        .unwrap_or_else(|| panic!("the late press: {:#?}", outcome.calls));
    assert_eq!(late.delivery, Some(Delivery::NotSent), "{late:?}");
    assert!(
        matches!(&late.result, CallResult::Error { name, .. } if name == "TimeoutError"),
        "{late:?}"
    );
    let inputs: Vec<_> = outcome
        .calls
        .iter()
        .filter(|call| call.method != "inputValue" && call.params["selector"] != "#late")
        .collect();
    assert_eq!(inputs.len(), 6, "{:#?}", outcome.calls);
    assert!(
        inputs
            .iter()
            .all(|call| call.delivery == Some(Delivery::Done)),
        "{inputs:#?}"
    );
}

/// A slow page: `#a`'s handler keeps the renderer busy for 3 s inside `Input.insertText`, so a
/// fill of `#a` with a 1.5 s timeout is still waiting for its insertText's answer when the
/// timeout passes, however long (under 1.5 s) the fill took to reach its insertText.
const SLOW_FILL_PAGE: &str = r"<!doctype html><html><body>
<input id='a'><input id='b'>
<script>
window.__events = [];
for (const type of ['beforeinput', 'input'])
  addEventListener(type, e => __events.push([e.type, e.target.id, (e.data ?? '').length]), true);
document.getElementById('a').addEventListener('beforeinput', () => {
  const end = performance.now() + 3000;
  while (performance.now() < end);
});
</script>
</body></html>";

/// A fill of a large value whose call times out in the middle of its `Input.insertText` still
/// inserts the whole value, and the next fill, issued once that call has stopped and while its
/// writer still waits, lands whole on another field after it. Each record's delivery is what
/// its writer saw: the stopped fill's text was answered, so `Done`.
#[tokio::test(flavor = "multi_thread")]
async fn a_fill_stopped_mid_insert_text_lands_whole_before_the_next() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![("/slow-fill.html".into(), SLOW_FILL_PAGE.into())],
            ..Fixture::default()
        },
        "/slow-fill.html",
    )
    .await;
    let outcome = tab
        .start_run(
            r"const large = 'x'.repeat(50000);
const first = await page.locator('#a').fill(large, { timeout: 1500 }).then(() => 'resolved', error => error.name);
const next = await page.locator('#b').fill('second');
return [first, next === undefined];"
                .to_string(),
        )
        .await
        .await
        .unwrap_or_else(|error| panic!("the run's task: {error}"));
    let values = tab
        .evaluate("[document.querySelector('#a').value.length, document.querySelector('#b').value]")
        .await;
    let recorded = tab.evaluate("window.__events").await;
    tab.close().await;

    assert_ok(&outcome);
    assert_eq!(outcome.value, Some(json!(["TimeoutError", true])));
    assert_eq!(values, json!([50000, "second"]));
    assert_eq!(
        recorded,
        json!([
            ["beforeinput", "a", 50000],
            ["input", "a", 50000],
            ["beforeinput", "b", 6],
            ["input", "b", 6],
        ]),
        "the next fill lands after the whole first value"
    );
    let [stopped, next] = outcome.calls.as_slice() else {
        panic!("two fills: {:#?}", outcome.calls);
    };
    assert!(
        matches!(&stopped.result, CallResult::Error { name, .. } if name == "TimeoutError"),
        "{stopped:?}"
    );
    assert_eq!(stopped.delivery, Some(Delivery::Done), "{stopped:?}");
    assert_eq!(
        (&next.result, next.delivery),
        (&CallResult::Ok, Some(Delivery::Done)),
        "{next:?}"
    );
}

async fn review_wait_for(tab: &Tab, condition: &str) {
    let started = Instant::now();
    while tab.evaluate(condition).await != json!(true) {
        assert!(started.elapsed() < Duration::from_secs(30), "{condition}");
        time::sleep(Duration::from_millis(10)).await;
    }
}

async fn review_record(tab: &Tab) {
    tab.evaluate(
        r"(() => {
      window.__review = [];
      for (const type of ['keydown', 'keyup', 'mousedown', 'mouseup', 'mousemove'])
        addEventListener(type, e => __review.push({type, key: e.key, shift: e.shiftKey,
          buttons: e.buttons, target: e.target.id}), true);
      return true;
    })()",
    )
    .await;
}

/// Three targets in a row at y 180 (`start` at x 100, `middle` at 200, `end` at 300), checked
/// by hit-testing.
async fn review_targets(tab: &Tab) {
    tab.evaluate(r"(() => {
      for (const [id, left] of [['start',80],['middle',180],['end',280]]) {
        const e = document.createElement('div'); e.id = id;
        e.style = `position:absolute;left:${left}px;top:160px;width:40px;height:40px;background:red`;
        document.body.append(e);
      }
      return true;
    })()").await;
    let scene = tab.evaluate("({size:[innerWidth,innerHeight],targets:[100,200,300].map(x => document.elementFromPoint(x,180)?.id)})").await;
    assert_eq!(
        scene["targets"],
        json!(["start", "middle", "end"]),
        "{scene}"
    );
}

/// One live run keeps Shift and the shared pointer state alive while other runs finish.
/// A worker's mouse button is released at its end; temporary modifiers restore the first
/// run's ownership; a later move starts at the previous run's position, crossing the middle
/// target. No exact mouse coordinates or elapsed timings are compared.
#[tokio::test(flavor = "multi_thread")]
async fn review_run_end_keeps_other_owners_and_cross_run_pointer_state() {
    let Some(tab) = open().await else { return };
    let runtime = runtime(&tab).await;
    review_record(&tab).await;
    review_targets(&tab).await;
    let cancel = CancellationToken::new();
    let timeout = Duration::from_mins(1);
    let exercise = async {
        review_wait_for(
            &tab,
            "__review.some(e => e.type === 'keydown' && e.key === 'Shift')",
        )
        .await;
        let first = runtime
            .run(request(
                &tab,
                "await page.mouse.move(100,180); await page.mouse.down()",
                timeout,
                CancellationToken::new(),
            ))
            .await;
        assert_ok(&first);
        assert_eq!(
            tab.evaluate("__review.filter(e => e.key === 'Shift').map(e => e.type)")
                .await,
            json!(["keydown"]),
            "another run's end must not release Shift"
        );
        assert_eq!(
            tab.evaluate("__review.filter(e => e.type === 'mouseup').map(e => e.buttons)")
                .await,
            json!([0]),
            "the finishing run released its mouse button"
        );
        tab.evaluate("window.__review = []").await;
        let second = runtime.run(request(&tab,
            "await page.mouse.move(300,180,{steps:2}); await page.locator('#c').click({modifiers:[]}); await page.locator('#p').press('KeyA')",
            timeout, CancellationToken::new())).await;
        assert_ok(&second);
        assert_eq!(
            tab.evaluate(
                "__review.filter(e => e.type === 'mousemove').slice(0,2).map(e => e.target)"
            )
            .await,
            json!(["middle", "end"]),
            "the next run starts at the retained pointer"
        );
        assert_eq!(
            tab.evaluate("__review.filter(e => e.key === 'Shift').map(e => e.type)")
                .await,
            json!(["keyup", "keydown"]),
            "restored Shift still belongs to the live run"
        );
        assert_eq!(
            tab.evaluate("document.querySelector('#p').value").await,
            json!("A")
        );
        cancel.cancel();
    };
    let (keeper, ()) = tokio::join!(
        runtime.run(request(
            &tab,
            "await page.keyboard.down('Shift'); await new Promise(() => {})",
            timeout,
            cancel.clone()
        )),
        exercise,
    );
    assert_eq!(keeper.status, RunStatus::Cancelled, "{keeper:#?}");
    let next = runtime
        .run(request(
            &tab,
            "await page.locator('#p').press('KeyA'); await page.locator('#c').click()",
            timeout,
            CancellationToken::new(),
        ))
        .await;
    let value = tab.evaluate("document.querySelector('#p').value").await;
    let shift = tab
        .evaluate("__review.filter(e => e.type === 'mousedown').at(-1).shift")
        .await;
    tab.close().await;
    assert_ok(&next);
    assert_eq!(
        value,
        json!("Aa"),
        "run end released the owning run's Shift"
    );
    assert_eq!(shift, json!(false), "the later click is unmodified");
}

/// Cut a multi-event device call only after its first input reaches the page. Its owed
/// release still lands, the partial call is Unknown, and the next run sees no held input.
#[tokio::test(flavor = "multi_thread")]
async fn review_partial_device_calls_release_and_remain_unknown() {
    for (method, code, started) in [
        (
            "keyboardType",
            "await page.keyboard.type('abc',{delay:30000})",
            "__review.some(e => e.type === 'keydown' && e.key === 'a')",
        ),
        (
            "mouseClick",
            "await page.mouse.click(30,20,{delay:30000,clickCount:2})",
            "__review.some(e => e.type === 'mousedown')",
        ),
    ] {
        let Some(tab) = open().await else { return };
        let runtime = runtime(&tab).await;
        review_record(&tab).await;
        tab.evaluate("document.querySelector('#p').focus(); true")
            .await;
        let cancel = CancellationToken::new();
        let stopper = async {
            review_wait_for(&tab, started).await;
            cancel.cancel();
        };
        let (cut, ()) = tokio::join!(
            runtime.run(request(&tab, code, Duration::from_mins(1), cancel.clone())),
            stopper
        );
        assert_eq!(cut.status, RunStatus::Cancelled, "{cut:#?}");
        let call = cut
            .calls
            .iter()
            .find(|call| call.method == method)
            .expect("device call");
        assert_eq!(call.delivery, Some(Delivery::Unknown), "{call:#?}");
        let released = if method == "keyboardType" {
            "__review.filter(e => e.key === 'a').map(e => e.type)"
        } else {
            "__review.filter(e => e.type === 'mousedown' || e.type === 'mouseup').map(e => e.type)"
        };
        assert_eq!(
            tab.evaluate(released).await,
            if method == "keyboardType" {
                json!(["keydown", "keyup"])
            } else {
                json!(["mousedown", "mouseup"])
            }
        );
        let next = runtime
            .run(request(
                &tab,
                "await page.locator('#p').press('KeyA'); await page.locator('#c').click()",
                Duration::from_secs(30),
                CancellationToken::new(),
            ))
            .await;
        let normal = tab
            .evaluate("__review.filter(e => e.type === 'mousedown').at(-1)")
            .await;
        tab.close().await;
        assert_ok(&next);
        assert_eq!(normal["buttons"], json!(1));
        assert_eq!(normal["shift"], json!(false));
    }
}

/// A cancelled pointer action restores the previous modifiers before run-end cleanup.
#[tokio::test(flavor = "multi_thread")]
async fn review_cancelled_pointer_restores_modifiers_before_run_end() {
    let Some(tab) = open().await else { return };
    let runtime = runtime(&tab).await;
    review_record(&tab).await;
    let cancel = CancellationToken::new();
    let stopper = async {
        review_wait_for(&tab, "__review.some(e => e.type === 'mousedown')").await;
        cancel.cancel();
    };
    let (cut, ()) = tokio::join!(runtime.run(request(&tab,
        "await page.keyboard.down('Shift'); await page.locator('#c').click({modifiers:[],delay:30000,clickCount:2})",
        Duration::from_mins(1), cancel.clone())), stopper);
    let keys = tab
        .evaluate("__review.filter(e => e.key === 'Shift').map(e => e.type)")
        .await;
    let clicks = tab
        .evaluate("__review.filter(e => e.type === 'mousedown').map(e => e.shift)")
        .await;
    tab.close().await;
    assert_eq!(cut.status, RunStatus::Cancelled, "{cut:#?}");
    assert_eq!(keys, json!(["keydown", "keyup", "keydown", "keyup"]));
    assert_eq!(clicks, json!([false]));
    let click = cut
        .calls
        .iter()
        .find(|call| call.method == "click")
        .expect("click");
    assert_eq!(click.delivery, Some(Delivery::Unknown), "{click:#?}");
}

/// Known review issue: stop after Alt lands, before the planned Shift keyDown. The Alt keyUp
/// must not claim Shift is held merely because the original plan included it.
#[tokio::test(flavor = "multi_thread")]
async fn review_stop_during_modifier_setup_uses_the_actual_mask() {
    let Some(tab) = open().await else { return };
    let runtime = runtime(&tab).await;
    review_record(&tab).await;
    tab.cdp
        .send("Runtime.enable", json!({}), Some(&tab.driver))
        .await
        .expect("enable");
    tab.cdp
        .send(
            "Runtime.addBinding",
            json!({"name":"reviewAltLanded"}),
            Some(&tab.driver),
        )
        .await
        .expect("binding");
    tab.evaluate(
        r"(() => {
      addEventListener('keydown', e => {
        if (e.key !== 'Alt') return;
        reviewAltLanded('down');
        const end = performance.now() + 1000;
        while (performance.now() < end) {}
      }, {once:true});
      return true;
    })()",
    )
    .await;
    let mut binding = tab.cdp.on_event("Runtime.bindingCalled");
    let cancel = CancellationToken::new();
    let stopper = async {
        time::timeout(Duration::from_secs(30), async {
            loop {
                let event = binding.recv().await.expect("binding event");
                if event.params["name"] == "reviewAltLanded" {
                    cancel.cancel();
                    break;
                }
            }
        })
        .await
        .expect("Alt reached the page");
    };
    let (cut, ()) = tokio::join!(
        runtime.run(request(
            &tab,
            "await page.locator('#c').click({modifiers:['Alt','Shift']})",
            Duration::from_mins(1),
            cancel.clone()
        )),
        stopper
    );
    let keys = tab
        .evaluate("__review.filter(e => e.type === 'keydown' || e.type === 'keyup')")
        .await;
    tab.close().await;
    assert_eq!(cut.status, RunStatus::Cancelled, "{cut:#?}");
    let keys = keys.as_array().expect("events");
    assert!(
        !keys
            .iter()
            .any(|event| event["key"] == "Shift" && event["type"] == "keydown"),
        "cancellation must occur before Shift is pressed: {keys:?}"
    );
    assert!(
        keys.iter().all(|event| event["shift"] == false),
        "no event may claim Shift when it was never pressed: {keys:?}"
    );
}

/// An explicit down by a second live run also needs the modifier to stay held when the
/// first run finishes. Its subsequent `KeyA` must still carry Shift.
#[tokio::test(flavor = "multi_thread")]
async fn review_finishing_first_holder_preserves_the_second_holder() {
    let Some(tab) = open().await else { return };
    let runtime = runtime(&tab).await;
    review_record(&tab).await;
    let (finished, first_finished) = oneshot::channel();
    let timeout = Duration::from_mins(1);
    let first = async {
        let outcome = runtime.run(request(&tab,
            "await page.keyboard.down('Shift'); await page.locator('#finishA').waitFor({state:'attached'})",
            timeout, CancellationToken::new())).await;
        finished.send(()).expect("first finished");
        outcome
    };
    let second = async {
        review_wait_for(
            &tab,
            "__review.filter(e => e.type === 'keydown' && e.key === 'Shift').length === 1",
        )
        .await;
        let advance = async {
            review_wait_for(
                &tab,
                "__review.filter(e => e.type === 'keydown' && e.key === 'Shift').length === 2",
            )
            .await;
            tab.evaluate(
                "document.body.insertAdjacentHTML('beforeend','<span id=finishA></span>'); true",
            )
            .await;
            first_finished.await.expect("first run drained");
            tab.evaluate(
                "document.body.insertAdjacentHTML('beforeend','<span id=continueB></span>'); true",
            )
            .await;
        };
        let (outcome, ()) = tokio::join!(runtime.run(request(&tab,
            "await page.keyboard.down('Shift'); await page.locator('#continueB').waitFor({state:'attached'}); await page.locator('#p').press('KeyA')",
            timeout, CancellationToken::new())), advance);
        outcome
    };
    let (first, second) = tokio::join!(first, second);
    let value = tab.evaluate("document.querySelector('#p').value").await;
    let events = tab
        .evaluate("__review.filter(e => e.type === 'keydown' || e.type === 'keyup')")
        .await;
    tab.close().await;
    assert_ok(&first);
    assert_ok(&second);
    assert_eq!(
        value,
        json!("A"),
        "the second run is still holding Shift: {events}"
    );
}

/// Cancelling the new Frame.type route after its first key still releases that key.
#[tokio::test(flavor = "multi_thread")]
async fn review_a3a_cancelled_type_releases_its_key_and_stays_unknown() {
    let Some(tab) = open().await else { return };
    let runtime = runtime(&tab).await;
    review_record(&tab).await;
    tab.evaluate(
        "window.__a3Held = new Set(); addEventListener('keydown', e => __a3Held.add(e.code)); addEventListener('keyup', e => __a3Held.delete(e.code)); true",
    )
    .await;
    let cancel = CancellationToken::new();
    let stopper = async {
        review_wait_for(
            &tab,
            "__review.some(e => e.type === 'keydown' && e.key === 'a')",
        )
        .await;
        cancel.cancel();
    };
    let (cut, ()) = tokio::join!(
        runtime.run(request(
            &tab,
            "await page.locator('#p').pressSequentially('abc', {delay:30000})",
            Duration::from_mins(1),
            cancel.clone(),
        )),
        stopper
    );
    let released = tab
        .evaluate("__review.filter(e => e.key === 'a').map(e => e.type)")
        .await;
    let held = tab.evaluate("Array.from(__a3Held)").await;
    let partial = tab.evaluate("document.querySelector('#p').value").await;
    tab.close().await;
    assert_eq!(cut.status, RunStatus::Cancelled, "{cut:#?}");
    let call = cut
        .calls
        .iter()
        .find(|call| call.method == "type")
        .expect("Frame.type call");
    assert_eq!(call.delivery, Some(Delivery::Unknown), "{call:#?}");
    assert_eq!(released, json!(["keydown", "keyup"]));
    assert_eq!(held, json!([]));
    assert_eq!(partial, json!("a"));
}

/// A field, the key events the page sees (`__keys`) and the keys it saw go down and not come up
/// yet (`__held`), as its own handlers would track them, and a cross-site, so out-of-process,
/// child frame. `__recovered` is set once a document's busy work is over.
const BUSY_TOP: &str = r"<!doctype html><html><body>
<input id='field'><iframe name='child'></iframe>
<script>
window.__keys = []; window.__held = new Set();
for (const type of ['keydown', 'keyup'])
  addEventListener(type, e => {
    __keys.push({ type, key: e.key, shift: e.shiftKey });
    if (type === 'keydown') __held.add(e.key); else __held.delete(e.key);
  }, true);
addEventListener('message', e => { if (e.data === 'recovered') window.__recovered = true });
document.querySelector('iframe').src = 'http://localhost:' + location.port + '/busy-child.html';
</script>
</body></html>";

const BUSY_CHILD: &str =
    "<!doctype html><html><body><button id='child' type='button'>Child</button></body></html>";

/// Holds `{keys}` in the top field and returns while `{busy}`, a document of the tab, runs three
/// seconds of synchronous work, from just after the call that starts it answers.
const HOLD_THEN_BUSY: &str = r"const frame = page.frame({ name: 'child' })
await frame.waitForURL('**/busy-child.html')
{setup}
await page.locator('#field').focus()
for (const key of [{keys}]) await page.keyboard.down(key)
await {busy}.evaluate(ms => {
  setTimeout(() => {
    const end = Date.now() + ms
    while (Date.now() < end) {}
    top.postMessage('recovered', '*')
  }, 10)
}, 3000)
await new Promise(resolve => setTimeout(resolve, 100))
return 'busy'";

/// Says it is running, waits for the test's `#go`, types `a`, then reads what the top document
/// saw. The press takes the tab's input turn, so it lands after any release an earlier run is
/// still writing.
const TYPE_THEN_READ: &str = r"await page.evaluate(() => { window.__waiting = true })
await page.locator('#go').waitFor({ state: 'attached', timeout: 20000 })
await page.locator('#field').press('a')
return await page.evaluate(() => ({
  held: Array.from(window.__held),
  value: document.querySelector('#field').value,
  typed: window.__keys.filter(key => key.type === 'keydown' && key.key.toLowerCase() === 'a'),
}))";

/// A run holding keys ends while a frame of its tab is busy: a child frame with the run's
/// highlight, whose removal waits for it while the releases must not, or the focused page,
/// whose releases wait for it past the run's outcome. Either way the page sees every key come
/// up, and the next run on the tab through the same `Runtime` types an unshifted `a`, also when
/// it was already running (so the tab's input state outlives the first run).
#[tokio::test(flavor = "multi_thread")]
async fn a_busy_frame_never_leaves_an_ended_runs_keys_held() {
    let Some(binary) = browser::e2e_browser() else {
        return;
    };
    let highlight = "await frame.locator('#child').highlight()";
    let both = "'Shift', 'Control'";
    let mut seen = Vec::new();
    for (case, setup, busy, keys, waiting) in [
        ("highlighted child", highlight, "frame", both, false),
        ("focused page", "", "page", both, false),
        (
            "highlighted child, next run waiting",
            highlight,
            "frame",
            "'Shift'",
            true,
        ),
    ] {
        let hold = HOLD_THEN_BUSY
            .replace("{setup}", setup)
            .replace("{busy}", busy)
            .replace("{keys}", keys);
        let (first, next) = busy_frame_case(binary.clone(), case, &hold, waiting).await;
        seen.push((case, first, next));
    }
    let unshifted = Some(json!({
        "held": [],
        "value": "a",
        "typed": [{ "type": "keydown", "key": "a", "shift": false }],
    }));
    let busy = Some(json!("busy"));
    assert_eq!(
        seen,
        [
            ("highlighted child", busy.clone(), unshifted.clone()),
            ("focused page", busy.clone(), unshifted.clone()),
            ("highlighted child, next run waiting", busy, unshifted),
        ],
        "every key came up at the first run's end, and the next run's typing is unshifted"
    );
}

/// One case of [`a_busy_frame_never_leaves_an_ended_runs_keys_held`] on a fresh browser: runs
/// `hold`, waits for the busy document to recover, and answers both runs' values. The next run
/// starts after that, or, when `waiting`, before the first run and waits for the test's `#go`.
async fn busy_frame_case(
    binary: OsString,
    case: &str,
    hold: &str,
    waiting: bool,
) -> (Option<Value>, Option<Value>) {
    let tab = Tab::open(
        binary,
        Fixture {
            pages: vec![
                ("/busy-top.html".into(), BUSY_TOP.into()),
                ("/busy-child.html".into(), BUSY_CHILD.into()),
            ],
            ..Fixture::default()
        },
        "/busy-top.html",
    )
    .await;
    let runtime = runtime(&tab).await;
    let timeout = Duration::from_secs(30);
    let next_run = || {
        runtime.run(request(
            &tab,
            TYPE_THEN_READ,
            timeout,
            CancellationToken::new(),
        ))
    };
    // The first run, the busy document's recovery, then the next run's go.
    let first_run = async {
        if waiting {
            review_wait_for(&tab, "window.__waiting === true").await;
        }
        let first = runtime
            .run(request(&tab, hold, timeout, CancellationToken::new()))
            .await;
        let targets = tab
            .cdp
            .send("Target.getTargets", json!({}), None)
            .await
            .unwrap_or_else(|error| panic!("Target.getTargets: {error}"));
        review_wait_for(&tab, "window.__recovered === true").await;
        tab.evaluate(
            "document.body.append(Object.assign(document.createElement('p'), { id: 'go' })); true",
        )
        .await;
        (first, targets)
    };
    let ((first, targets), next) = if waiting {
        tokio::join!(first_run, next_run())
    } else {
        let first = first_run.await;
        (first, next_run().await)
    };
    tab.close().await;

    assert_ok(&first);
    assert_ok(&next);
    let out_of_process = targets["targetInfos"].as_array().is_some_and(|targets| {
        targets.iter().any(|target| {
            target["type"] == "iframe"
                && target["url"]
                    .as_str()
                    .is_some_and(|url| url.starts_with("http://localhost:"))
        })
    });
    assert!(
        out_of_process,
        "{case}: the child runs out of process: {targets}"
    );
    (first.value, next.value)
}
