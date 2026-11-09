//! Neo: tests of the element checkpoint (hooks feature 4). Loop A's checkpoint runs under paused
//! time against click's scripted tab and a scripted fake of the hook sandbox; then real calls go
//! through the run's dispatcher to the fake browser, which pins, fills and clicks.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::FutureExt;
use neo_runtime_spec::injected::{FormFillOutcome, FormFillOutcomeStatus, InjectedFillResult};
use serde_json::json;
use tokio::sync::Notify;
use tokio::time;
use tokio_util::sync::CancellationToken;

use super::element::{ElementOptions, Rerouted};
use super::tests::{
    FakeHooks, Harness, Timeline, decided, frame_guid, hooks_of, kept, message, mouse_click, never,
    record, run_scope,
};
use super::*;
use crate::fake_browser::{GO_NODE, REFUSED_FILL};
use crate::hooks::ElementRef;
use crate::method_router::Dispatcher;
use crate::playwright_methods::check::{self, CheckOptions};
use crate::playwright_methods::click::tests::{ArmAnswer, FakeTab, Step, pin_guid};
use crate::playwright_methods::click::{self, PageError, PointerOptions, StatesCheck};
use crate::playwright_methods::evaluate::Handles;
use crate::playwright_methods::fill::{self, FillOptions};
use crate::playwright_methods::forms::CheckedState;
use crate::progress::{CallError, CallStart, Progress};
use crate::run::{CallHookRecord, CallOrigin, CallRecord, CallResult, Delivery, RunEnd};

/// The secret a hook swaps in for the script's placeholder.
const SECRET: &str = "s3cret-hunter2";
const PLACEHOLDER: &str = "{{password}}";
const SELECTOR: &str = "#pw";
/// What click's fake tab says it resolved.
const PREVIEW: &str = "<button>Submit</button>";

/// The paused tests' route: a hook's args decode as a fill's when they carry a `value`, else as
/// a click's, and only for [`SELECTOR`], as the dispatcher refuses another element.
struct TestElementRoute;

impl ElementRoute for TestElementRoute {
    fn reroute_element(&self, args: Value) -> Result<Rerouted, String> {
        if args["selector"] != SELECTOR {
            return Err("they change the element the call acts on".to_string());
        }
        let options: ElementOptions = if args.get("value").is_some() {
            Arc::new(FillOptions::from_params(
                serde_json::from_value(args).map_err(|error| error.to_string())?,
            ))
        } else {
            Arc::new(
                PointerOptions::from_click(
                    serde_json::from_value(args).map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.message)?,
            )
        };
        Ok(Rerouted {
            options,
            charge: None,
        })
    }
}

/// One paused-time element call: its entry, and its progress with the element gate of `hooks`.
struct Paused {
    entry: Arc<CallEntry>,
    progress: Progress,
}

fn paused(hooks: &Arc<FakeHooks>, method: &str, args: Value, timeout_ms: f64) -> Paused {
    let entry = Arc::new(CallEntry::new(CallStart {
        call_id: 1,
        guid: "frame@T".to_string(),
        method: method.to_string(),
        title: None,
        origin: CallOrigin::Agent,
        action: None,
        params: args.clone(),
        location: None,
        input: true,
    }));
    let event = ActionEvent {
        kind: Some(if method == "fill" { "fill" } else { "click" }),
        group: Some(if method == "fill" { "type" } else { "pointer" }),
        method: method.to_string(),
        guid: "frame@T".to_string(),
        args,
        location: None,
        page: Some("page@T".to_string()),
        frame: Some("frame@T".to_string()),
        context: "context@default".to_string(),
        locator: Some(LocatorRef {
            frame: "frame@T".to_string(),
            selector: SELECTOR.to_string(),
        }),
        element: None,
    };
    let gate = Arc::new(ElementGate::new(
        hooks_of(hooks),
        event,
        1,
        Box::new(TestElementRoute),
    ));
    let run = run_scope(Duration::from_hours(1));
    let progress = Progress::new(
        Arc::clone(&entry),
        timeout_ms,
        run,
        CancellationToken::new(),
    )
    .with_element_gate(Some(gate));
    Paused { entry, progress }
}

fn click_args() -> Value {
    json!({ "selector": SELECTOR, "strict": true })
}

fn click_options() -> PointerOptions {
    PointerOptions::from_click(serde_json::from_value(click_args()).unwrap()).unwrap()
}

fn fill_args(value: &str) -> Value {
    json!({ "selector": SELECTOR, "strict": true, "value": value })
}

fn fill_options(value: &str) -> FillOptions {
    FillOptions::from_params(serde_json::from_value(fill_args(value)).unwrap())
}

/// The record of a call only beforeAction saw.
fn before_record(before: BeforeHookResult) -> CallHookRecord {
    CallHookRecord {
        before: Some(before),
        after: None,
    }
}

/// The pinned element each beforeAction invocation received, in order.
fn pins(fake: &FakeHooks) -> Vec<Option<ElementRef>> {
    fake.events(ActionHook::Before)
        .into_iter()
        .map(|(event, _)| event.element)
        .collect()
}

fn pinned(node: u32) -> ElementRef {
    ElementRef {
        guid: pin_guid(node),
        preview: PREVIEW.to_string(),
    }
}

/// Where `name` first appears in `calls`.
fn at(calls: &[String], name: &str) -> usize {
    calls
        .iter()
        .position(|call| call == name)
        .unwrap_or_else(|| panic!("{name} in {calls:?}"))
}

// ---------------------------------------------------------------------------------------------
// Loop A's checkpoint, under paused time, on click's scripted tab.
// ---------------------------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn tests_that_the_hook_sees_the_found_element_before_the_action_touches_it() {
    let tab = FakeTab::ready();
    let calls = Arc::clone(&tab.calls);
    let timeline = Timeline::default();
    let fake = Arc::new(FakeHooks::new(&timeline).before(move |_, event| {
        let guid = event
            .element
            .map(|element| element.guid)
            .unwrap_or_default();
        calls
            .lock()
            .unwrap()
            .push(format!("hook asked about {guid}"));
        decided(Ok(ActionDecision::Allow))
    }));
    let call = paused(&fake, "click", click_args(), 30_000.0);

    let result = click::pointer_action(&tab, &click_options(), &call.progress).await;

    assert_eq!(result, Ok(()));
    let seen = tab.calls();
    let asked = format!("hook asked about {}", pin_guid(1));
    assert!(at(&seen, "resolve") < at(&seen, "pin 1"), "{seen:?}");
    assert!(at(&seen, "pin 1") < at(&seen, &asked), "{seen:?}");
    assert!(at(&seen, &asked) < at(&seen, "input"), "{seen:?}");
    assert!(
        seen.iter().any(|call| call.starts_with("press")),
        "{seen:?}"
    );
    let [(event, None)] = &fake.events(ActionHook::Before)[..] else {
        panic!("one question: {:?}", fake.events(ActionHook::Before));
    };
    assert_eq!(event.element, Some(pinned(1)));
    assert_eq!(
        event.locator,
        Some(LocatorRef {
            frame: "frame@T".to_string(),
            selector: SELECTOR.to_string(),
        })
    );
    assert_eq!(event.args, click_args());
    assert_eq!(
        record(&call.entry).hook,
        Some(before_record(BeforeHookResult::Allowed))
    );
    assert!(
        !tab.calls().contains(&"unpin 1".to_string()),
        "the pin lives as long as its call"
    );
    drop(call);
    assert!(tab.calls().contains(&"unpin 1".to_string()));
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_retry_that_finds_another_element_asks_again_and_one_that_finds_the_same_does_not()
 {
    // The first attempt finds its element detached: loop A resolves again and finds node 2.
    let mut tab = FakeTab::ready();
    tab.check_states = Step::new(
        Duration::from_millis(33),
        [Ok(StatesCheck::NotConnected), Ok(StatesCheck::Ready)],
    );
    tab.nodes = Step::new(Duration::ZERO, [1, 2]);
    let timeline = Timeline::default();
    let fake = Arc::new(FakeHooks::new(&timeline));
    let call = paused(&fake, "click", click_args(), 30_000.0);
    let result = click::pointer_action(&tab, &click_options(), &call.progress).await;
    assert_eq!(result, Ok(()));
    assert_eq!(pins(&fake), [Some(pinned(1)), Some(pinned(2))]);
    let seen = tab.calls();
    assert!(
        at(&seen, "unpin 1") < at(&seen, "pin 2"),
        "the first pin goes before the second is minted: {seen:?}"
    );
    assert_eq!(tab.count("resolve"), 2);
    drop(call);

    // The same node again: its decision stands.
    let mut tab = FakeTab::ready();
    tab.check_states = Step::new(
        Duration::from_millis(33),
        [Ok(StatesCheck::NotConnected), Ok(StatesCheck::Ready)],
    );
    let fake = Arc::new(FakeHooks::new(&timeline));
    let call = paused(&fake, "click", click_args(), 30_000.0);
    let result = click::pointer_action(&tab, &click_options(), &call.progress).await;
    assert_eq!(result, Ok(()));
    assert_eq!(pins(&fake), [Some(pinned(1))], "asked once");
    assert_eq!(tab.count("resolve"), 2);
    assert_eq!(tab.count("pin 1"), 1);
    assert_eq!(
        tab.count("node_key 1"),
        1,
        "the second element was identified"
    );
    assert_eq!(
        record(&call.entry).hook,
        Some(before_record(BeforeHookResult::Allowed))
    );
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_new_element_gets_a_new_decision_and_a_replacement_stays_redacted() {
    // The first element's hook replaces the value; the second element is allowed as written.
    let mut tab = FakeTab::ready();
    tab.nodes = Step::new(Duration::ZERO, [1, 2]);
    tab.forms.fill = Step::new(
        Duration::from_millis(1),
        [
            Ok(InjectedFillResult {
                outcome: FormFillOutcome::Status(FormFillOutcomeStatus::ErrorNotconnected),
            }),
            Ok(InjectedFillResult {
                outcome: FormFillOutcome::Status(FormFillOutcomeStatus::Done),
            }),
        ],
    );
    let timeline = Timeline::default();
    let fake = Arc::new(FakeHooks::new(&timeline).before(|invocation, event| {
        let mut args = event.args;
        args["value"] = json!(SECRET);
        decided(Ok(if invocation == 0 {
            ActionDecision::ReplaceArgs(args)
        } else {
            ActionDecision::Allow
        }))
    }));
    let call = paused(&fake, "fill", fill_args(PLACEHOLDER), 30_000.0);
    let result = fill::fill(&tab, &fill_options(PLACEHOLDER), &call.progress).await;
    assert_eq!(result, Ok(()));
    assert_eq!(pins(&fake), [Some(pinned(1)), Some(pinned(2))]);
    let typed: Vec<String> = tab
        .calls()
        .into_iter()
        .filter(|call| call.starts_with("fill \""))
        .collect();
    assert_eq!(
        typed,
        [
            format!("fill {SECRET:?} force=false"),
            format!("fill {PLACEHOLDER:?} force=false"),
        ]
    );
    let record = record(&call.entry);
    assert_eq!(record.hook, Some(before_record(BeforeHookResult::Allowed)));
    assert!(record.log.is_empty(), "the replaced attempt hid the log");
    assert!(call.progress.element_gate().unwrap().replaced());
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_node_number_another_document_reuses_is_another_element() {
    // The retry finds node 1 again, but in another document (a cross-site navigation keeps the
    // frame, and the new renderer may number its nodes from the same ids).
    let mut tab = FakeTab::ready();
    tab.check_states = Step::new(
        Duration::from_millis(33),
        [Ok(StatesCheck::NotConnected), Ok(StatesCheck::Ready)],
    );
    tab.documents = Step::new(Duration::ZERO, [1, 2]);
    let fake = Arc::new(FakeHooks::new(&Timeline::default()));
    let call = paused(&fake, "click", click_args(), 30_000.0);
    let result = click::pointer_action(&tab, &click_options(), &call.progress).await;
    assert_eq!(result, Ok(()));
    assert_eq!(
        pins(&fake),
        [Some(pinned(1)), Some(pinned(1))],
        "asked again"
    );
    assert_eq!(tab.count("pin 1"), 2);
    let seen = tab.calls();
    let second_pin = seen
        .iter()
        .rposition(|call| call == "pin 1")
        .expect("pinned again");
    assert!(at(&seen, "unpin 1") < second_pin, "{seen:?}");
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_node_found_after_the_input_went_out_is_only_verified() {
    // A check whose click landed, then whose box re-renders: the read-back finds a new node.
    let mut tab = FakeTab::ready();
    let state = |matches: bool| CheckedState {
        matches,
        received: Some(if matches { "checked" } else { "unchecked" }.into()),
        is_radio: Some(false),
    };
    tab.forms.checked = Step::new(
        Duration::from_millis(1),
        [
            Ok(state(false)),
            Err(PageError::NotConnected),
            Ok(state(true)),
        ],
    );
    tab.nodes = Step::new(Duration::ZERO, [1, 2]);
    let fake = Arc::new(FakeHooks::new(&Timeline::default()));
    let call = paused(&fake, "check", click_args(), 30_000.0);
    let options = CheckOptions::from_check(serde_json::from_value(click_args()).unwrap());
    let result = check::set_checked(&tab, &options, &call.progress).await;
    assert_eq!(result, Ok(()));
    assert_eq!(pins(&fake), [Some(pinned(1))], "no second question");
    assert_eq!(tab.count("pin 2"), 0);
    assert_eq!(call.progress.delivery().unwrap().delivery(), Delivery::Done);
    assert_eq!(
        record(&call.entry).hook,
        Some(before_record(BeforeHookResult::Allowed)),
        "the record keeps the decision that let the click through"
    );
    let gate = call.progress.element_gate().unwrap();
    assert!(!gate.refused(), "it acted: its afterAction runs");
    assert_eq!(
        gate.acted().0,
        Some(pinned(1)),
        "afterAction sees the element clicked"
    );
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_forced_click_on_a_hooked_element_keeps_its_hit_check() {
    let forced = |position: bool| {
        let mut args = json!({ "selector": SELECTOR, "strict": true, "force": true });
        if position {
            args["position"] = json!({ "x": 230, "y": 20 });
        }
        PointerOptions::from_click(serde_json::from_value(args).unwrap()).unwrap()
    };
    for position in [true, false] {
        // Another element is at the point: a forced click would land on it.
        let mut tab = FakeTab::ready();
        tab.arm = Step::always(
            Duration::from_millis(1),
            Ok(ArmAnswer::Intercepted(
                "<button id=\"pay\">Pay</button>".into(),
            )),
        );
        let fake = Arc::new(FakeHooks::new(&Timeline::default()));
        let call = paused(&fake, "click", click_args(), 1000.0);
        let result = click::pointer_action(&tab, &forced(position), &call.progress).await;
        assert_eq!(result, Err(PlaywrightError::timeout(1000.0).into()));
        assert!(tab.count_prefix("arm ") > 0, "{:?}", tab.calls());
        assert_eq!(
            tab.count_prefix("press"),
            0,
            "nothing pressed: {:?}",
            tab.calls()
        );
        assert_eq!(
            call.progress.delivery().unwrap().delivery(),
            Delivery::NotSent
        );
    }
    // Without hooks, force is Playwright's: no hit check, the press goes out.
    let tab = FakeTab::ready();
    let progress = Progress::for_test(1000.0);
    let result = click::pointer_action(&tab, &forced(true), &progress).await;
    assert_eq!(result, Ok(()));
    assert_eq!(tab.count_prefix("arm "), 0, "{:?}", tab.calls());
    assert_eq!(tab.count_prefix("press"), 1, "{:?}", tab.calls());
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_swapped_value_never_reaches_an_agent_calls_log_or_error() {
    let run = run_scope(Duration::from_hours(1));
    run.swapped
        .remember(&fill_args(PLACEHOLDER), &fill_args(SECRET));
    let agent = Arc::new(CallEntry::new(CallStart {
        call_id: 2,
        guid: "frame@T".to_string(),
        method: "press".to_string(),
        title: None,
        origin: CallOrigin::Agent,
        action: None,
        params: json!({ "selector": SELECTOR, "key": "Enter" }),
        location: None,
        input: true,
    }));
    agent.scrub_with(Arc::clone(&run.swapped));
    let progress = Progress::new(Arc::clone(&agent), 30_000.0, run, CancellationToken::new());
    // The page mirrors the box's value into its attribute: the next call's preview names it.
    progress.log(format!(
        "  locator resolved to <input id=\"pw\" type=\"password\" value=\"{SECRET}\"/>"
    ));
    let error = agent.scrubbed(format!(
        "strict mode violation: <input value=\"{SECRET}\"/>"
    ));
    let log = agent.finish(CallResult::Error {
        name: "Error".to_string(),
        message: error.clone(),
    });
    assert!(!error.contains(SECRET), "{error}");
    assert!(!log.join("\n").contains(SECRET), "{log:?}");
    assert!(
        log.join("\n").contains("locator resolved to"),
        "the log is kept: {log:?}"
    );
    let record = record(&agent);
    assert!(
        !serde_json::to_string(&record).unwrap().contains(SECRET),
        "{record:?}"
    );
    assert!(
        record.params.to_string().contains("Enter"),
        "params are the script's"
    );
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_blocked_element_action_sends_nothing() {
    let tab = FakeTab::ready();
    let timeline = Timeline::default();
    let fake = Arc::new(
        FakeHooks::new(&timeline)
            .before(|_, _| decided(Ok(ActionDecision::Block("not this box".to_string())))),
    );
    let call = paused(&fake, "click", click_args(), 30_000.0);
    let result = click::pointer_action(&tab, &click_options(), &call.progress).await;
    assert_eq!(
        result,
        Err(PlaywrightError::error(format!("{BLOCKED_BY_HOOK}: not this box")).into())
    );
    assert_eq!(tab.count("input"), 0, "{:?}", tab.calls());
    assert!(!tab.calls().iter().any(|call| call.starts_with("press")));
    assert_eq!(
        call.progress.delivery().unwrap().delivery(),
        Delivery::NotSent
    );
    assert_eq!(
        record(&call.entry).hook,
        Some(before_record(BeforeHookResult::Blocked))
    );
    assert!(call.progress.element_gate().unwrap().refused());
    assert!(
        tab.calls().contains(&"unpin 1".to_string()),
        "a refused pin goes at once"
    );
}

#[tokio::test(start_paused = true)]
async fn tests_that_replaced_arguments_act_on_the_pinned_element_and_hide_the_log() {
    let tab = FakeTab::ready();
    let timeline = Timeline::default();
    let fake = Arc::new(FakeHooks::new(&timeline).before(|_, event| {
        let mut args = event.args;
        args["value"] = json!(SECRET);
        decided(Ok(ActionDecision::ReplaceArgs(args)))
    }));
    let call = paused(&fake, "fill", fill_args(PLACEHOLDER), 30_000.0);
    let result = fill::fill(&tab, &fill_options(PLACEHOLDER), &call.progress).await;
    assert_eq!(result, Ok(()));
    assert!(
        tab.calls()
            .contains(&format!("fill {SECRET:?} force=false")),
        "the page got the hook's value: {:?}",
        tab.calls()
    );
    assert!(
        !tab.calls()
            .contains(&format!("fill {PLACEHOLDER:?} force=false"))
    );
    assert_eq!(
        tab.count("resolve"),
        1,
        "on the element the hook was asked about"
    );
    let record = record(&call.entry);
    assert_eq!(record.params, fill_args(PLACEHOLDER));
    assert_eq!(
        record.hook,
        Some(before_record(BeforeHookResult::ArgsReplaced))
    );
    assert!(record.log.is_empty(), "{:?}", record.log);
    assert!(!serde_json::to_string(&record).unwrap().contains(SECRET));
    assert_eq!(record.delivery, Some(Delivery::Done));
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_replacement_aimed_at_another_element_or_a_failed_hook_fails_closed() {
    let other = Arc::new(FakeHooks::new(&Timeline::default()).before(|_, event| {
        let mut args = event.args;
        args["selector"] = json!("#user");
        decided(Ok(ActionDecision::ReplaceArgs(args)))
    }));
    let threw = Arc::new(
        FakeHooks::new(&Timeline::default())
            .before(|_, _| decided(Err(HookFailure::Threw(format!("leaked {SECRET}"))))),
    );
    for fake in [other, threw] {
        let tab = FakeTab::ready();
        let call = paused(&fake, "fill", fill_args(PLACEHOLDER), 30_000.0);
        let result = fill::fill(&tab, &fill_options(PLACEHOLDER), &call.progress).await;
        assert_eq!(result, Err(before_failed().into()));
        assert_eq!(tab.count("input"), 0, "{:?}", tab.calls());
        assert_eq!(tab.count("fill"), 0, "{:?}", tab.calls());
        assert_eq!(
            call.progress.delivery().unwrap().delivery(),
            Delivery::NotSent
        );
        assert_eq!(
            record(&call.entry).hook,
            Some(before_record(BeforeHookResult::Failed))
        );
        let failures = fake.take_failures();
        assert_eq!(
            failures.len(),
            1,
            "the reason goes to the host: {failures:?}"
        );
        assert!(
            !serde_json::to_string(&record(&call.entry))
                .unwrap()
                .contains(SECRET)
        );
    }
}

#[tokio::test(start_paused = true)]
async fn tests_that_the_hooks_wait_counts_against_the_actions_own_timeout() {
    let tab = FakeTab::ready();
    let dropped = Arc::new(AtomicBool::new(false));
    let abandoned = Arc::clone(&dropped);
    let fake = Arc::new(FakeHooks::new(&Timeline::default()).before(move |_, _| never(&abandoned)));
    let call = paused(&fake, "click", click_args(), 500.0);
    let started = time::Instant::now();
    let result = click::pointer_action(&tab, &click_options(), &call.progress).await;
    assert_eq!(result, Err(PlaywrightError::timeout(500.0).into()));
    assert_eq!(started.elapsed(), Duration::from_millis(500));
    assert!(
        dropped.load(Ordering::SeqCst),
        "the invocation is abandoned"
    );
    assert_eq!(tab.count("input"), 0);
    assert_eq!(
        call.progress.delivery().unwrap().delivery(),
        Delivery::NotSent
    );
    assert!(
        call.progress.element_gate().unwrap().refused(),
        "stopped while its hook decided: it never acted, so no afterAction sees it"
    );
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_call_meeting_its_runs_closed_hooks_stops_without_a_failure() {
    let tab = FakeTab::ready();
    let fake = Arc::new(FakeHooks::new(&Timeline::default()).before(|_, _| {
        decided(Err(HookFailure::Realm(
            "the hook realm is gone".to_string(),
        )))
    }));
    fake.closed.store(true, Ordering::SeqCst);
    let call = paused(&fake, "click", click_args(), 30_000.0);
    let result = click::pointer_action(&tab, &click_options(), &call.progress).await;
    assert_eq!(result, Err(CallError::RunEnded(RunEnd::Settled)));
    assert!(fake.take_failures().is_empty());
    assert_eq!(record(&call.entry).hook, None);
    assert_eq!(tab.count("input"), 0);
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_call_without_an_element_gate_mints_no_pin() {
    let tab = FakeTab::ready();
    let progress = Progress::for_test(30_000.0);
    let result = click::pointer_action(&tab, &click_options(), &progress).await;
    assert_eq!(result, Ok(()));
    assert!(
        !tab.calls()
            .iter()
            .any(|call| call.starts_with("pin") || call.starts_with("node_key")),
        "{:?}",
        tab.calls()
    );
}

// ---------------------------------------------------------------------------------------------
// Real calls through the run's dispatcher to the fake browser.
// ---------------------------------------------------------------------------------------------

fn click(id: u64) -> Value {
    message(
        id,
        &frame_guid(),
        "click",
        json!({ "selector": "#go", "strict": true }),
        json!({ "timeout": 5000 }),
    )
}

fn fill_go(id: u64, value: &str) -> Value {
    message(
        id,
        &frame_guid(),
        "fill",
        json!({ "selector": "#go", "strict": true, "value": value }),
        json!({ "timeout": 5000 }),
    )
}

/// A hook that swaps the placeholder for `value` in every fill it is asked about.
fn swapping(value: &'static str) -> Arc<FakeHooks> {
    Arc::new(
        FakeHooks::new(&Timeline::default())
            .wanting(|_, kind| kind == "fill")
            .before(move |_, event| {
                let mut args = event.args;
                args["value"] = json!(value);
                decided(Ok(ActionDecision::ReplaceArgs(args)))
            }),
    )
}

impl Harness {
    /// The text of every `Input.insertText` the page received, in order.
    fn inserted(&self) -> Vec<Value> {
        self.browser
            .exchanges()
            .into_iter()
            .filter(|exchange| exchange.method() == "Input.insertText")
            .map(|exchange| exchange.command["params"]["text"].clone())
            .collect()
    }

    /// How many commands named `method` the page received.
    fn sent(&self, method: &str) -> usize {
        self.browser
            .exchanges()
            .into_iter()
            .filter(|exchange| exchange.method() == method)
            .count()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_hooked_click_pins_its_element_for_the_hook_alone() {
    let timeline = Timeline::default();
    let seen = timeline.clone();
    let views: Arc<OnceLock<(Arc<Handles>, Arc<Handles>)>> = Arc::default();
    let tables = Arc::clone(&views);
    let fake = Arc::new(FakeHooks::new(&timeline).before(move |_, event| {
        let guid = event
            .element
            .map(|element| element.guid)
            .unwrap_or_default();
        if let Some((agent, hooks)) = tables.get() {
            seen.push(format!(
                "agent sees the pin: {}, the hook sees it: {}",
                agent.frame_guid(&guid).is_some(),
                hooks.frame_guid(&guid).is_some()
            ));
        }
        seen.push(format!("pin {guid}"));
        decided(Ok(ActionDecision::Allow))
    }));
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    let scope = harness.dispatcher.run_scope();
    let _set = views.set((Arc::clone(&scope.handles), Arc::clone(&scope.hook_handles)));

    let reply = harness.call(&click(1)).await.unwrap();
    assert_eq!(reply, json!({ "id": 1 }));
    assert!(
        !harness.mouse_events().is_empty(),
        "the click reached the page"
    );
    let [(event, None)] = &fake.events(ActionHook::Before)[..] else {
        panic!("one question: {:?}", fake.events(ActionHook::Before));
    };
    let element = event.element.clone().expect("a pinned element");
    assert!(element.guid.starts_with("handle@"), "{element:?}");
    assert_eq!(
        event.locator,
        Some(LocatorRef {
            frame: frame_guid(),
            selector: "#go".to_string(),
        })
    );
    let lines: Vec<String> = timeline
        .lines()
        .into_iter()
        .filter(|line| !line.starts_with("before ") && !line.starts_with("after "))
        .collect();
    assert_eq!(
        lines,
        [
            "agent sees the pin: false, the hook sees it: true".to_string(),
            format!("pin {}", element.guid),
        ]
    );
    // The call is over: its pin is gone from the hook realm's view, its object released.
    assert!(
        harness
            .dispatcher
            .run_scope()
            .hook_handles
            .frame_guid(&element.guid)
            .is_none()
    );
    assert_eq!(harness.sent("Runtime.releaseObject"), 1);
    let pinned_node = harness.browser.exchanges().into_iter().any(|exchange| {
        exchange.method() == "DOM.resolveNode"
            && exchange.command["params"]["backendNodeId"] == GO_NODE
    });
    assert!(pinned_node, "the pin holds the button's node");
    let records = harness.records(RunEnd::Settled).await;
    assert_eq!(
        records[0].hook,
        Some(CallHookRecord {
            before: Some(BeforeHookResult::Allowed),
            after: Some(AfterHookResult::Kept),
        })
    );
    assert_eq!(records[0].delivery, Some(Delivery::Done));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_swapped_fill_types_the_secret_and_nothing_the_model_reads_holds_it() {
    let fake = swapping(SECRET);
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    let reply = harness.call(&fill_go(1, PLACEHOLDER)).await.unwrap();
    assert!(reply.get("error").is_none(), "{reply}");
    assert!(!reply.to_string().contains(SECRET), "{reply}");
    assert_eq!(harness.inserted(), [json!(SECRET)]);
    // afterAction sees the element the fill typed into, and the arguments it typed with.
    let before = fake.events(ActionHook::Before);
    let after = fake.events(ActionHook::After);
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].0.element, before[0].0.element);
    assert!(after[0].0.element.is_some());
    assert_eq!(after[0].0.args["value"], SECRET);
    let records = harness.records(RunEnd::Settled).await;
    let record = &records[0];
    assert_eq!(record.params["value"], PLACEHOLDER);
    assert_eq!(
        record.hook,
        Some(CallHookRecord {
            before: Some(BeforeHookResult::ArgsReplaced),
            after: Some(AfterHookResult::Kept),
        })
    );
    assert!(record.log.is_empty(), "{:?}", record.log);
    assert_eq!(record.result, CallResult::Ok);
    assert!(!serde_json::to_string(&records).unwrap().contains(SECRET));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_failing_swapped_fill_names_the_secret_only_to_the_host() {
    let refused: &'static str = Box::leak(format!("{REFUSED_FILL}{SECRET}").into_boxed_str());
    let fake = swapping(refused);
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    let reply = harness.call(&fill_go(1, PLACEHOLDER)).await.unwrap();
    assert_eq!(reply["error"]["error"]["name"], "Error", "{reply}");
    assert!(!reply.to_string().contains(SECRET), "{reply}");
    assert_eq!(reply["log"], json!([]), "{reply}");
    let records = harness.records(RunEnd::Settled).await;
    let record = &records[0];
    assert!(matches!(&record.result, CallResult::Error { name, .. } if name == "Error"));
    assert!(record.log.is_empty());
    assert!(!serde_json::to_string(&records).unwrap().contains(SECRET));
    let failures = fake.take_failures();
    assert!(
        failures.iter().any(|line| line.contains(SECRET)),
        "the host-only trace has the page's own text: {failures:?}"
    );

    // Without a hook, the same page error reaches the script: the redaction is the hook's.
    let harness = Harness::start(None, Duration::from_secs(30)).await;
    let reply = harness.call(&fill_go(1, refused)).await.unwrap();
    assert!(reply.to_string().contains(SECRET), "{reply}");
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_replacement_that_retargets_or_a_blocking_hook_sends_no_input() {
    let retarget = Arc::new(
        FakeHooks::new(&Timeline::default())
            .wanting(|_, kind| kind == "fill")
            .before(|_, event| {
                let mut args = event.args;
                args["selector"] = json!("#user");
                args["value"] = json!(SECRET);
                decided(Ok(ActionDecision::ReplaceArgs(args)))
            }),
    );
    let blocking = Arc::new(
        FakeHooks::new(&Timeline::default())
            .wanting(|_, kind| kind == "fill")
            .before(|_, _| decided(Ok(ActionDecision::Block("a text box".to_string())))),
    );
    for (fake, decision) in [
        (retarget, BeforeHookResult::Failed),
        (blocking, BeforeHookResult::Blocked),
    ] {
        let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
        let reply = harness.call(&fill_go(1, PLACEHOLDER)).await.unwrap();
        assert_eq!(reply["error"]["error"]["name"], "Error", "{reply}");
        assert!(!reply.to_string().contains(SECRET), "{reply}");
        assert!(harness.inserted().is_empty());
        let records = harness.records(RunEnd::Settled).await;
        assert_eq!(records[0].hook, Some(before_record(decision)));
        assert_eq!(records[0].delivery, Some(Delivery::NotSent));
        assert!(!serde_json::to_string(&records).unwrap().contains(SECRET));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_waiting_element_hook_holds_no_input_lock() {
    let timeline = Timeline::default();
    let asked = Arc::new(Notify::new());
    let notified = Arc::clone(&asked);
    let decisions = timeline.clone();
    let fake = Arc::new(
        FakeHooks::new(&timeline)
            .wanting(|hook, kind| hook == ActionHook::Before && kind == "click")
            .before(move |_, _| {
                notified.notify_one();
                let decisions = decisions.clone();
                async move {
                    time::sleep(Duration::from_millis(500)).await;
                    decisions.push("click decided");
                    Ok(ActionDecision::Allow)
                }
                .boxed()
            }),
    );
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    let other = async {
        asked.notified().await;
        // A call that needs the tab's input, while the click's hook decides.
        let reply = harness.call(&mouse_click(2, 5.0, 5.0)).await.unwrap();
        timeline.push("mouse click done");
        reply
    };
    let hooked = click(1);
    let (clicked, moused) = tokio::join!(harness.call(&hooked), other);
    assert_eq!(clicked.unwrap(), json!({ "id": 1 }));
    assert_eq!(moused, json!({ "id": 2 }));
    let lines = timeline.lines();
    assert!(
        at(&lines, "mouse click done") < at(&lines, "click decided"),
        "{lines:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_hooks_own_input_goes_while_the_element_action_waits_for_it() {
    let realm: Arc<OnceLock<Dispatcher>> = Arc::default();
    let hook_side = Arc::clone(&realm);
    let fake = Arc::new(
        FakeHooks::new(&Timeline::default())
            .wanting(|hook, kind| hook == ActionHook::Before && kind == "click")
            .before(move |_, _| {
                let realm = hook_side.get().cloned();
                async move {
                    let realm = realm.ok_or_else(|| HookFailure::Realm("no realm".into()))?;
                    // The hook's own input, while the agent's click waits for this decision.
                    let reply = realm.call(&mouse_click(90, 5.0, 5.0).to_string()).await;
                    reply
                        .map(|_| ActionDecision::Allow)
                        .map_err(|error| HookFailure::Threw(format!("{error:?}")))
                }
                .boxed()
            }),
    );
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    let _set = realm.set(harness.hook_realm.clone());
    let hooked = click(1);
    let reply = time::timeout(Duration::from_secs(10), harness.call(&hooked))
        .await
        .expect("the hook's input never waits for the call it decides")
        .unwrap();
    assert_eq!(reply, json!({ "id": 1 }));
    let records = harness.records(RunEnd::Settled).await;
    let hooks_own = records
        .iter()
        .find(|record| record.origin == CallOrigin::Hook)
        .expect("the hook's mouse click");
    assert_eq!(hooks_own.result, CallResult::Ok);
    assert_eq!(hooks_own.delivery, Some(Delivery::Done));
    let agents = records
        .iter()
        .find(|record| record.method == "click")
        .expect("the agent's click");
    assert_eq!(agents.result, CallResult::Ok);
    assert_eq!(agents.delivery, Some(Delivery::Done));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_replies_after_a_swap_name_it_to_hooks_only() {
    // Swaps the placeholder only; any other fill goes as the script wrote it.
    let fake = Arc::new(
        FakeHooks::new(&Timeline::default())
            .wanting(|hook, kind| hook == ActionHook::Before && kind == "fill")
            .before(|_, event| {
                if event.args["value"] != PLACEHOLDER {
                    return decided(Ok(ActionDecision::Allow));
                }
                let mut args = event.args;
                args["value"] = json!(SECRET);
                decided(Ok(ActionDecision::ReplaceArgs(args)))
            }),
    );
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    let reply = harness.call(&fill_go(1, PLACEHOLDER)).await.unwrap();
    assert!(reply.get("error").is_none(), "{reply}");
    // A later call whose page error names the secret (here it is in the call's own text; on a
    // real page, a mirrored attribute in a preview).
    let refused = format!("{REFUSED_FILL}{SECRET}");
    let agents = harness.call(&fill_go(2, &refused)).await.unwrap();
    assert_eq!(agents["error"]["error"]["name"], "Error", "{agents}");
    assert!(!agents.to_string().contains(SECRET), "{agents}");
    let hooks = harness.hook_call(&fill_go(3, &refused)).await.unwrap();
    assert!(
        hooks["error"]["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains(SECRET)),
        "a hook's own reply keeps the real text: {hooks}"
    );
    // Records keep what the script sent (`params`); no log or error text names the secret.
    let records = harness.records(RunEnd::Settled).await;
    for record in &records {
        let text = serde_json::to_string(&(&record.log, &record.result)).unwrap();
        assert!(!text.contains(SECRET), "{record:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_an_element_action_no_hook_wants_mints_no_pin() {
    let fake = Arc::new(FakeHooks::new(&Timeline::default()).wanting(|_, kind| kind == "goto"));
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    assert_eq!(harness.call(&click(1)).await.unwrap(), json!({ "id": 1 }));
    let reply = harness.call(&fill_go(2, PLACEHOLDER)).await.unwrap();
    assert!(reply.get("error").is_none(), "{reply}");
    assert_eq!(harness.inserted(), [json!(PLACEHOLDER)]);
    assert_eq!(fake.invocations(), 0);
    assert_eq!(harness.sent("DOM.describeNode"), 0, "no pin minted");
    assert_eq!(harness.sent("DOM.resolveNode"), 0, "no pin minted");
    let records = harness.records(RunEnd::Settled).await;
    assert!(records.iter().all(|record| record.hook.is_none()));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_after_action_sees_the_element_acted_on_and_a_blocked_one_gets_none() {
    let timeline = Timeline::default();
    let fake = Arc::new(
        FakeHooks::new(&timeline)
            .wanting(|_, kind| kind == "click")
            .after(|_, _| decided(Ok(kept(vec!["clicked".to_string()])))),
    );
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    assert_eq!(harness.call(&click(1)).await.unwrap(), json!({ "id": 1 }));
    let before = fake.events(ActionHook::Before);
    let after = fake.events(ActionHook::After);
    assert_eq!(after.len(), 1);
    assert_eq!(
        after[0].0.element, before[0].0.element,
        "the element acted on"
    );
    assert_eq!(after[0].1, Some(ActionEnd::Result(Value::Null)));
    assert_eq!(fake.take_notes(), ["clicked"]);
    let records = harness.records(RunEnd::Settled).await;
    assert_eq!(
        records[0].hook,
        Some(CallHookRecord {
            before: Some(BeforeHookResult::Allowed),
            after: Some(AfterHookResult::Kept),
        })
    );

    let blocking = Arc::new(
        FakeHooks::new(&timeline)
            .wanting(|_, kind| kind == "click")
            .before(|_, _| decided(Ok(ActionDecision::Block("no".to_string())))),
    );
    let harness = Harness::start(Some(hooks_of(&blocking)), Duration::from_secs(30)).await;
    let reply = harness.call(&click(1)).await.unwrap();
    assert_eq!(reply["error"]["error"]["name"], "Error");
    assert!(
        blocking.events(ActionHook::After).is_empty(),
        "it never acted"
    );
    assert!(harness.mouse_events().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_drag_asks_once_at_its_source_and_keeps_its_target() {
    let drag = |id: u64| {
        message(
            id,
            &frame_guid(),
            "dragAndDrop",
            json!({ "source": "#go", "target": "#go", "strict": true }),
            json!({ "timeout": 5000 }),
        )
    };
    let fake =
        Arc::new(FakeHooks::new(&Timeline::default()).wanting(|_, kind| kind == "dragAndDrop"));
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    let reply = harness.call(&drag(1)).await.unwrap();
    assert!(reply.get("error").is_none(), "{reply}");
    let asked = fake.events(ActionHook::Before);
    assert_eq!(asked.len(), 1, "asked once, at the source");
    assert_eq!(
        asked[0].0.locator,
        Some(LocatorRef {
            frame: frame_guid(),
            selector: "#go".to_string(),
        }),
        "its locator is the source's"
    );
    assert!(asked[0].0.element.is_some());
    assert!(!harness.mouse_events().is_empty());

    let retarget = Arc::new(
        FakeHooks::new(&Timeline::default())
            .wanting(|_, kind| kind == "dragAndDrop")
            .before(|_, event| {
                let mut args = event.args;
                args["target"] = json!("#elsewhere");
                decided(Ok(ActionDecision::ReplaceArgs(args)))
            }),
    );
    let harness = Harness::start(Some(hooks_of(&retarget)), Duration::from_secs(30)).await;
    let reply = harness.call(&drag(1)).await.unwrap();
    assert_eq!(reply["error"]["error"]["name"], "Error", "{reply}");
    assert!(harness.mouse_events().is_empty(), "nothing pressed");
    let records = harness.records(RunEnd::Settled).await;
    assert_eq!(
        records[0].hook,
        Some(before_record(BeforeHookResult::Failed))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_hooks_own_element_action_is_never_hooked() {
    let fake = Arc::new(FakeHooks::new(&Timeline::default()));
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    let reply = harness.hook_call(&click(1)).await.unwrap();
    assert_eq!(reply, json!({ "id": 1 }));
    assert_eq!(fake.invocations(), 0);
    assert_eq!(harness.sent("DOM.resolveNode"), 0, "no pin minted");
    let records: Vec<CallRecord> = harness.records(RunEnd::Settled).await;
    assert_eq!(records[0].origin, CallOrigin::Hook);
    assert_eq!(records[0].hook, None);
}
