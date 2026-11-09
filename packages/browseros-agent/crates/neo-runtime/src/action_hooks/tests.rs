//! Neo: tests of the action hooks of actions that target no element (hooks feature 3). The
//! gate's order, decisions and stops run under paused time against a scripted fake of the hook
//! sandbox; then real calls go through the run's dispatcher to the fake browser.

use std::future;
use std::mem;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use futures_util::FutureExt;
use neo_runtime_spec::serve::{FrameClickParams, FrameTitleParams, PageMouseClickParams};
use serde_json::json;
use tokio::runtime::Handle;
use tokio::sync::oneshot;
use tokio::task;
use tokio::time;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::*;
use crate::actions::SpecCommand;
use crate::browser_representations::cookies_and_storage::Permissions;
use crate::browser_representations::page::TabInputs;
use crate::browser_representations::tabs::Tabs;
use crate::chromium_bridge::browser_connection_settings::Switches;
use crate::deadline::Deadline;
use crate::fake_browser::{FakeBrowser, PAGE, Replies};
use crate::method_router::{BridgeError, Dispatcher};
use crate::playwright_methods::screenshot::TabScreenshots;
use crate::progress::{CallStart, ErrorName, Progress, Trace};
use crate::run::{
    CallHookRecord, CallLocation, CallOrigin, CallRecord, CallResult, Delivery, PageId, RunBinary,
    RunEnd,
};

type BeforeScript = Box<
    dyn Fn(usize, ActionEvent) -> BoxFuture<'static, Result<ActionDecision, HookFailure>>
        + Send
        + Sync,
>;
type AfterScript = Box<
    dyn Fn(ActionEvent, ActionEnd) -> BoxFuture<'static, Result<AfterDecision, HookFailure>>
        + Send
        + Sync,
>;
type WantsScript = Box<dyn Fn(ActionHook, &str) -> bool + Send + Sync>;

/// What happened, in order: the fake's invocations (`before <tag>`, `after <tag>`), what the
/// scripted hooks pushed (`decided <tag>`) and what the test did (`start <tag>`).
#[derive(Clone, Default)]
struct Timeline(Arc<Mutex<Vec<String>>>);

impl Timeline {
    fn push(&self, line: impl Into<String>) {
        self.0.lock().unwrap().push(line.into());
    }

    fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// A scripted stand-in for the hook sandbox: what it wants, what each invocation decides, and
/// what it was asked.
struct FakeHooks {
    wants: WantsScript,
    before: BeforeScript,
    after: AfterScript,
    timeline: Timeline,
    /// How many times the call path asked `wants`.
    asked: AtomicUsize,
    befores: AtomicUsize,
    /// Every event an invocation received, with how the action ended for `afterAction`.
    events: Mutex<Vec<(ActionHook, ActionEvent, Option<ActionEnd>)>>,
    /// What the call path left with the hooks, as the run's sandbox keeps it.
    notes: Mutex<Vec<String>>,
    failures: Mutex<Vec<String>>,
    /// Whether the run is done with its hooks ([`ActionHooks::ended`]).
    closed: AtomicBool,
}

impl FakeHooks {
    /// Wants every action, allows it and keeps its result.
    fn new(timeline: &Timeline) -> Self {
        Self {
            wants: Box::new(|_, _| true),
            before: Box::new(|_, _| decided(Ok(ActionDecision::Allow))),
            after: Box::new(|_, _| decided(Ok(kept(Vec::new())))),
            timeline: timeline.clone(),
            asked: AtomicUsize::new(0),
            befores: AtomicUsize::new(0),
            events: Mutex::default(),
            notes: Mutex::default(),
            failures: Mutex::default(),
            closed: AtomicBool::new(false),
        }
    }

    fn wanting(self, wants: impl Fn(ActionHook, &str) -> bool + Send + Sync + 'static) -> Self {
        Self {
            wants: Box::new(wants),
            ..self
        }
    }

    fn before(
        self,
        before: impl Fn(usize, ActionEvent) -> BoxFuture<'static, Result<ActionDecision, HookFailure>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            before: Box::new(before),
            ..self
        }
    }

    fn after(
        self,
        after: impl Fn(ActionEvent, ActionEnd) -> BoxFuture<'static, Result<AfterDecision, HookFailure>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            after: Box::new(after),
            ..self
        }
    }

    fn events(&self, hook: ActionHook) -> Vec<(ActionEvent, Option<ActionEnd>)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(seen, ..)| *seen == hook)
            .map(|(_, event, ended)| (event.clone(), ended.clone()))
            .collect()
    }

    fn invocations(&self) -> usize {
        self.events.lock().unwrap().len()
    }

    /// The notes the call path left so far, taken.
    fn take_notes(&self) -> Vec<String> {
        mem::take(&mut *self.notes.lock().unwrap())
    }

    /// The failure lines the call path left so far, taken.
    fn take_failures(&self) -> Vec<String> {
        mem::take(&mut *self.failures.lock().unwrap())
    }
}

impl ActionHooks for FakeHooks {
    fn wants(&self, event: ActionHook, kind: Option<&str>, _group: Option<&str>) -> bool {
        self.asked.fetch_add(1, Ordering::SeqCst);
        (self.wants)(event, kind.unwrap_or_default())
    }

    fn before_action(
        &self,
        event: ActionEvent,
    ) -> BoxFuture<'static, Result<ActionDecision, HookFailure>> {
        let invocation = self.befores.fetch_add(1, Ordering::SeqCst);
        self.timeline.push(format!("before {}", tag(&event)));
        self.events
            .lock()
            .unwrap()
            .push((ActionHook::Before, event.clone(), None));
        (self.before)(invocation, event)
    }

    fn after_action(
        &self,
        event: ActionEvent,
        ended: ActionEnd,
    ) -> BoxFuture<'static, Result<AfterDecision, HookFailure>> {
        self.timeline.push(format!("after {}", tag(&event)));
        self.events
            .lock()
            .unwrap()
            .push((ActionHook::After, event.clone(), Some(ended.clone())));
        (self.after)(event, ended)
    }

    fn add_notes(&self, notes: Vec<String>) {
        self.notes.lock().unwrap().extend(notes);
    }

    fn add_failure(&self, line: String) {
        self.failures.lock().unwrap().push(line);
    }

    fn ended(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

/// A call's name in the timeline: its args' `name`, else the line it was made on, else its
/// method.
fn tag(event: &ActionEvent) -> String {
    if let Some(name) = event.args.get("name").and_then(Value::as_str) {
        return name.to_string();
    }
    event.location.map_or_else(
        || event.method.clone(),
        |location| format!("line {}", location.line),
    )
}

fn decided<T: Send + 'static>(decision: T) -> BoxFuture<'static, T> {
    future::ready(decision).boxed()
}

fn later<T: Send + 'static>(delay: Duration, decision: T) -> BoxFuture<'static, T> {
    async move {
        time::sleep(delay).await;
        decision
    }
    .boxed()
}

/// Never decides; sets `dropped` once the call path abandons it.
fn never<T: Send + 'static>(dropped: &Arc<AtomicBool>) -> BoxFuture<'static, T> {
    let flag = SetOnDrop(Arc::clone(dropped));
    async move {
        let _flag = flag;
        future::pending().await
    }
    .boxed()
}

struct SetOnDrop(Arc<AtomicBool>);

impl Drop for SetOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn kept(notes: Vec<String>) -> AfterDecision {
    AfterDecision {
        replace: None,
        notes,
    }
}

fn replaced(value: Value) -> AfterDecision {
    AfterDecision {
        replace: Some(Replaced {
            value: Some(value),
            binaries: Vec::new(),
        }),
        notes: Vec::new(),
    }
}

fn title() -> ServedMethod {
    <FrameTitleParams as SpecCommand>::METHOD
}

fn entry() -> Arc<CallEntry> {
    Arc::new(CallEntry::new(CallStart {
        call_id: 1,
        guid: "frame@T".to_string(),
        method: "title".to_string(),
        title: Some("Get page title"),
        origin: CallOrigin::Agent,
        action: None,
        params: json!({}),
        location: None,
        input: false,
    }))
}

fn run_scope(timeout: Duration) -> RunScope {
    RunScope::new(
        Deadline::new(timeout, CancellationToken::new()),
        TaskTracker::new(),
    )
}

fn event(name: &str) -> ActionEvent {
    ActionEvent {
        kind: Some("title"),
        group: Some("read"),
        method: "title".to_string(),
        guid: "frame@T".to_string(),
        args: json!({ "name": name }),
        location: None,
        page: Some("page@T".to_string()),
        frame: Some("frame@T".to_string()),
        context: "context@default".to_string(),
        locator: None,
        element: None,
    }
}

/// The test's call path: a call is a string; a hook's args route as `rerouted <name>`, unless
/// `refuse` says why not; an allowed call settles as `settled <call>` and sets `settled`.
#[derive(Default)]
struct TestRoute {
    refuse: Option<String>,
    settled: Arc<AtomicBool>,
}

impl CallRoute for TestRoute {
    type Call = String;

    fn reroute(&self, args: Value) -> Result<String, String> {
        match &self.refuse {
            Some(why) => Err(format!("{why} {}", args["name"])),
            None => Ok(format!(
                "rerouted {}",
                args["name"].as_str().unwrap_or_default()
            )),
        }
    }

    fn settle(&self, call: String) -> String {
        self.settled.store(true, Ordering::SeqCst);
        format!("settled {call}")
    }
}

/// The hooks of a `title` call named `name`, admitted now: placed in `order` unless `held`.
fn admitted(
    fake: &Arc<FakeHooks>,
    name: &str,
    order: &Arc<CallOrder>,
    held: bool,
) -> CallHooks<TestRoute> {
    routed(fake, name, order, held, TestRoute::default())
}

/// [`admitted`], with `route` as its call path.
fn routed(
    fake: &Arc<FakeHooks>,
    name: &str,
    order: &Arc<CallOrder>,
    held: bool,
    route: TestRoute,
) -> CallHooks<TestRoute> {
    let hooks: Arc<dyn ActionHooks> = Arc::<FakeHooks>::clone(fake);
    let plan = HookPlan::new(&hooks, title()).unwrap();
    plan.admitted(event(name), 1, false, order, held, || route)
}

/// The gate of a call made as `"script"`.
async fn gate(
    hooks: &mut CallHooks<TestRoute>,
    run: &RunScope,
    entry: &CallEntry,
) -> Gated<String> {
    hooks.before(run, entry, "script".to_string()).await
}

/// `entry`'s record as the run reads it.
fn record(entry: &Arc<CallEntry>) -> CallRecord {
    let trace = Trace::default();
    trace.push(Arc::clone(entry));
    trace.take(RunEnd::Settled).0.remove(0)
}

fn before_record(before: BeforeHookResult) -> CallHookRecord {
    CallHookRecord {
        before: Some(before),
        after: None,
    }
}

fn after_record(after: AfterHookResult) -> CallHookRecord {
    CallHookRecord {
        before: None,
        after: Some(after),
    }
}

#[test]
fn tests_that_only_wanted_actions_without_an_element_are_planned() {
    let timeline = Timeline::default();
    let all: Arc<dyn ActionHooks> = Arc::new(FakeHooks::new(&timeline));
    let click = <FrameClickParams as SpecCommand>::METHOD;
    assert!(
        HookPlan::new(&all, click).is_none(),
        "an element action is feature 4's"
    );
    let plan = HookPlan::new(&all, title()).unwrap();
    assert!(plan.gates());
    let fake = Arc::new(FakeHooks::new(&timeline).wanting(|hook, _| hook == ActionHook::After));
    let after_only: Arc<dyn ActionHooks> = Arc::<FakeHooks>::clone(&fake);
    let plan = HookPlan::new(&after_only, title()).unwrap();
    assert!(!plan.gates(), "afterAction alone never holds the call");
    let nothing: Arc<dyn ActionHooks> = Arc::new(FakeHooks::new(&timeline).wanting(|_, _| false));
    assert!(HookPlan::new(&nothing, title()).is_none());
    assert_eq!(fake.asked.load(Ordering::SeqCst), 2);
    let mouse = <PageMouseClickParams as SpecCommand>::METHOD;
    assert!(
        HookPlan::new(&all, mouse).is_some(),
        "page.mouse has no element: hooked before it starts"
    );
}

#[tokio::test(start_paused = true)]
async fn tests_that_hooks_decide_and_calls_start_in_arrival_order() {
    let timeline = Timeline::default();
    let decisions = timeline.clone();
    // The first call's hook answers last.
    let fake = Arc::new(FakeHooks::new(&timeline).before(move |invocation, event| {
        let delay = Duration::from_millis(if invocation == 0 { 100 } else { 10 });
        let decisions = decisions.clone();
        async move {
            time::sleep(delay).await;
            decisions.push(format!("decided {}", tag(&event)));
            Ok(ActionDecision::Allow)
        }
        .boxed()
    }));
    let run = run_scope(Duration::from_mins(1));
    let order = Arc::new(CallOrder::default());
    let first = admitted(&fake, "first", &order, false);
    let second = admitted(&fake, "second", &order, false);
    // The second call's task runs first; its place still comes after the first's.
    let tasks = [("second", second), ("first", first)].map(|(name, mut hooks)| {
        let run = run.clone();
        let timeline = timeline.clone();
        tokio::spawn(async move {
            let gated = gate(&mut hooks, &run, &entry()).await;
            timeline.push(format!("start {name}"));
            gated
        })
    });
    for task in tasks {
        assert_eq!(
            task.await.unwrap(),
            Gated::Run("settled script".to_string())
        );
    }
    assert_eq!(
        timeline.lines(),
        [
            "before first",
            "decided first",
            "start first",
            "before second",
            "decided second",
            "start second"
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_call_the_debugger_held_takes_its_place_once_released() {
    let timeline = Timeline::default();
    let fake = Arc::new(FakeHooks::new(&timeline));
    let run = run_scope(Duration::from_mins(1));
    let order = Arc::new(CallOrder::default());
    let mut held = admitted(&fake, "held", &order, true);
    // The debugger's resume arrives after the held call and is decided while it is held.
    let mut resume = admitted(&fake, "resume", &order, false);
    let decided = time::timeout(Duration::from_secs(1), gate(&mut resume, &run, &entry())).await;
    assert_eq!(decided, Ok(Gated::Run("settled script".to_string())));
    assert_eq!(
        gate(&mut held, &run, &entry()).await,
        Gated::Run("settled script".to_string())
    );
    assert_eq!(timeline.lines(), ["before resume", "before held"]);
}

#[tokio::test(start_paused = true)]
async fn tests_that_an_abort_at_the_gate_replies_abort_error_and_abandons_the_hook() {
    let timeline = Timeline::default();
    let dropped = Arc::new(AtomicBool::new(false));
    let abandoned = Arc::clone(&dropped);
    let fake = Arc::new(FakeHooks::new(&timeline).before(move |_, _| never(&abandoned)));
    let run = run_scope(Duration::from_mins(1));
    let entry = entry();
    let mut hooks = admitted(&fake, "aborted", &Arc::default(), false);
    let abort = async {
        time::sleep(Duration::from_secs(5)).await;
        entry.abort("stop waiting".to_string());
    };
    let (gated, ()) = tokio::join!(gate(&mut hooks, &run, &entry), abort);
    assert_eq!(
        gated,
        Gated::Refused(PlaywrightError::aborted("stop waiting"))
    );
    assert!(
        dropped.load(Ordering::SeqCst),
        "the invocation is abandoned"
    );
    assert_eq!(record(&entry).hook, None, "the hook decided nothing");
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_call_aborted_before_its_turn_leaves_the_order() {
    let timeline = Timeline::default();
    let (release, released) = oneshot::channel::<()>();
    let released = Mutex::new(Some(released));
    let fake = Arc::new(FakeHooks::new(&timeline).before(move |invocation, _| {
        let released = released.lock().unwrap().take();
        match (invocation, released) {
            (0, Some(released)) => async move {
                let _released = released.await;
                Ok(ActionDecision::Allow)
            }
            .boxed(),
            _ => decided(Ok(ActionDecision::Allow)),
        }
    }));
    let run = run_scope(Duration::from_mins(1));
    let order = Arc::new(CallOrder::default());
    let (first_entry, second_entry, third_entry) = (entry(), entry(), entry());
    let mut first = admitted(&fake, "first", &order, false);
    let mut second = admitted(&fake, "second", &order, false);
    let mut third = admitted(&fake, "third", &order, false);
    let script = async {
        time::sleep(Duration::from_secs(1)).await;
        second_entry.abort("gave up".to_string());
        time::sleep(Duration::from_secs(1)).await;
        release.send(()).unwrap();
    };
    let (first, second, third, ()) = tokio::join!(
        gate(&mut first, &run, &first_entry),
        gate(&mut second, &run, &second_entry),
        gate(&mut third, &run, &third_entry),
        script
    );
    assert_eq!(first, Gated::Run("settled script".to_string()));
    assert_eq!(second, Gated::Refused(PlaywrightError::aborted("gave up")));
    assert_eq!(third, Gated::Run("settled script".to_string()));
    assert_eq!(
        timeline.lines(),
        ["before first", "before third"],
        "the aborted call never reached the hook, and left its place"
    );
}

#[tokio::test(start_paused = true)]
async fn tests_that_the_run_stopping_at_the_gate_ends_the_call_without_a_reply() {
    let timeline = Timeline::default();
    let dropped = Arc::new(AtomicBool::new(false));
    let abandoned = Arc::clone(&dropped);
    let fake = Arc::new(FakeHooks::new(&timeline).before(move |_, _| never(&abandoned)));

    let run = run_scope(Duration::from_secs(10));
    let started = Instant::now();
    let mut hooks = admitted(&fake, "deadline", &Arc::default(), false);
    assert_eq!(gate(&mut hooks, &run, &entry()).await, Gated::Stopped);
    assert_eq!(
        started.elapsed(),
        Duration::from_secs(10),
        "at the run's deadline"
    );
    assert!(
        dropped.load(Ordering::SeqCst),
        "the invocation is abandoned"
    );

    let run = run_scope(Duration::from_mins(1));
    let mut hooks = admitted(&fake, "ended", &Arc::default(), false);
    let end = async {
        time::sleep(Duration::from_secs(1)).await;
        run.end(RunEnd::Settled);
    };
    let ended = entry();
    let (gated, ()) = tokio::join!(gate(&mut hooks, &run, &ended), end);
    assert_eq!(gated, Gated::Stopped);

    let run = run_scope(Duration::from_mins(1));
    let mut hooks = admitted(&fake, "closed", &Arc::default(), false);
    let close = async {
        time::sleep(Duration::from_secs(1)).await;
        run.context_closed.cancel();
    };
    let closed = entry();
    let (gated, ()) = tokio::join!(gate(&mut hooks, &run, &closed), close);
    assert_eq!(gated, Gated::Refused(PlaywrightError::target_closed()));
}

#[tokio::test(start_paused = true)]
async fn tests_that_the_gate_waits_for_a_slow_hook_on_no_clock_but_the_runs() {
    let timeline = Timeline::default();
    let fake = Arc::new(
        FakeHooks::new(&timeline)
            .before(|_, _| later(Duration::from_secs(50), Ok(ActionDecision::Allow))),
    );
    let run = run_scope(Duration::from_mins(1));
    let mut hooks = admitted(&fake, "slow", &Arc::default(), false);
    assert_eq!(
        gate(&mut hooks, &run, &entry()).await,
        Gated::Run("settled script".to_string())
    );
}

#[tokio::test(start_paused = true)]
async fn tests_that_each_before_decision_runs_blocks_or_reroutes_the_call() {
    let timeline = Timeline::default();
    let run = run_scope(Duration::from_mins(1));
    let order = Arc::new(CallOrder::default());

    let allow = Arc::new(FakeHooks::new(&timeline));
    let entry_allowed = entry();
    let mut hooks = admitted(&allow, "allowed", &order, false);
    assert_eq!(
        gate(&mut hooks, &run, &entry_allowed).await,
        Gated::Run("settled script".to_string())
    );
    assert_eq!(
        record(&entry_allowed).hook,
        Some(CallHookRecord {
            before: Some(BeforeHookResult::Allowed),
            after: None
        })
    );

    let block = Arc::new(
        FakeHooks::new(&timeline)
            .before(|_, _| decided(Ok(ActionDecision::Block("no reads here".to_string())))),
    );
    let entry_blocked = entry();
    let mut hooks = admitted(&block, "blocked", &order, false);
    let Gated::Refused(error) = gate(&mut hooks, &run, &entry_blocked).await else {
        panic!("a blocked call must not run");
    };
    assert_eq!(error.name, ErrorName::Error);
    assert!(error.message.contains("no reads here"), "{error:?}");
    assert_eq!(
        record(&entry_blocked).hook,
        Some(before_record(BeforeHookResult::Blocked))
    );

    let replace = Arc::new(FakeHooks::new(&timeline).before(|_, _| {
        decided(Ok(ActionDecision::ReplaceArgs(
            json!({ "name": "swapped" }),
        )))
    }));
    let entry_replaced = entry();
    let mut hooks = admitted(&replace, "replaced", &order, false);
    assert_eq!(
        gate(&mut hooks, &run, &entry_replaced).await,
        Gated::Run("settled rerouted swapped".to_string())
    );
    let progress = Progress::new(
        Arc::clone(&entry_replaced),
        0.0,
        run.clone(),
        CancellationToken::new(),
    );
    progress.log("typed s3cret");
    progress.name_locator("locator('#s3cret')");
    assert!(
        progress.folded_log().is_empty(),
        "a call run with a hook's arguments keeps no log"
    );
    assert_eq!(
        record(&entry_replaced).hook,
        Some(before_record(BeforeHookResult::ArgsReplaced))
    );
    assert!(record(&entry_replaced).log.is_empty());
    // afterAction sees the arguments the call ran with.
    let after = hooks.ran(&run).unwrap();
    after.finish(&entry_replaced, Ok(None)).await;
    let (seen, _) = replace.events(ActionHook::After).remove(0);
    assert_eq!(seen.args, json!({ "name": "swapped" }));
}

#[tokio::test(start_paused = true)]
async fn tests_that_replaced_arguments_the_call_refuses_block_it() {
    let timeline = Timeline::default();
    let run = run_scope(Duration::from_mins(1));
    let fake = Arc::new(
        FakeHooks::new(&timeline)
            .before(|_, _| decided(Ok(ActionDecision::ReplaceArgs(json!({ "name": "s3cret" }))))),
    );
    let entry = entry();
    let settled = Arc::new(AtomicBool::new(false));
    let route = TestRoute {
        refuse: Some("bad".to_string()),
        settled: Arc::clone(&settled),
    };
    let mut hooks = routed(&fake, "refused", &Arc::default(), false, route);
    let gated = gate(&mut hooks, &run, &entry).await;
    let Gated::Refused(error) = gated else {
        panic!("refused arguments must block the call");
    };
    assert_eq!(error.name, ErrorName::Error);
    assert!(!error.message.contains("s3cret"), "{error:?}");
    assert!(
        !settled.load(Ordering::SeqCst),
        "nothing settles for a blocked call"
    );
    assert_eq!(
        record(&entry).hook,
        Some(before_record(BeforeHookResult::Failed))
    );
    let trace = fake.take_failures();
    assert!(
        trace.iter().any(|line| line.contains("s3cret")),
        "{trace:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_failed_before_hook_blocks_the_call_and_tells_only_the_host() {
    let failures = [
        HookFailure::Threw("Error: s3cret".to_string()),
        HookFailure::Timeout,
        HookFailure::BadDecision("{\"allow\":\"s3cret\"}".to_string()),
        HookFailure::Realm("the hook realm is gone".to_string()),
    ];
    for failure in failures {
        let timeline = Timeline::default();
        let run = run_scope(Duration::from_mins(1));
        let failed = failure.clone();
        let fake =
            Arc::new(FakeHooks::new(&timeline).before(move |_, _| decided(Err(failed.clone()))));
        let entry = entry();
        let mut hooks = admitted(&fake, "failing", &Arc::default(), false);
        let Gated::Refused(error) = gate(&mut hooks, &run, &entry).await else {
            panic!("{failure:?} must block the call");
        };
        assert_eq!(error.name, ErrorName::Error, "{failure:?}");
        assert!(!error.message.contains("s3cret"), "{error:?}");
        assert_eq!(
            record(&entry).hook,
            Some(before_record(BeforeHookResult::Failed))
        );
        let trace = fake.take_failures();
        assert_eq!(trace.len(), 1, "{failure:?}: {trace:?}");
        if let HookFailure::Threw(detail) | HookFailure::BadDecision(detail) = &failure {
            assert!(trace[0].contains(detail.as_str()), "{trace:?}");
        }
    }
}

#[tokio::test(start_paused = true)]
async fn tests_that_after_action_keeps_or_replaces_the_result_and_takes_notes() {
    let timeline = Timeline::default();
    let run = run_scope(Duration::from_mins(1));
    let actual = json!({ "value": "Fake page" });

    let keep = Arc::new(FakeHooks::new(&timeline).after(|_, _| {
        decided(Ok(kept(vec![
            "saw a title".to_string(),
            "twice".to_string(),
        ])))
    }));
    let entry_kept = entry();
    let after = admitted(&keep, "kept", &Arc::default(), false)
        .ran(&run)
        .unwrap();
    let outcome = after.finish(&entry_kept, Ok(Some(actual.clone()))).await;
    assert_eq!(outcome, Some(Ok(Some(actual.clone()))));
    assert_eq!(
        record(&entry_kept).hook,
        Some(after_record(AfterHookResult::Kept))
    );
    assert_eq!(keep.take_notes(), ["saw a title", "twice"]);
    let (_, ended) = keep.events(ActionHook::After).remove(0);
    assert_eq!(ended, Some(ActionEnd::Result(actual.clone())));

    let replace = Arc::new(
        FakeHooks::new(&timeline).after(|_, _| decided(Ok(replaced(json!({ "value": "Hooked" }))))),
    );
    let entry_replaced = entry();
    let after = admitted(&replace, "replaced", &Arc::default(), false)
        .ran(&run)
        .unwrap();
    let outcome = after.finish(&entry_replaced, Ok(Some(actual))).await;
    assert_eq!(outcome, Some(Ok(Some(json!({ "value": "Hooked" })))));
    assert_eq!(
        record(&entry_replaced).hook,
        Some(after_record(AfterHookResult::Replaced))
    );

    // A failed call's error reaches the hook, which may keep it.
    let entry_failed = entry();
    let after = admitted(&keep, "failed", &Arc::default(), false)
        .ran(&run)
        .unwrap();
    let error = PlaywrightError::timeout(5.0);
    let outcome = after.finish(&entry_failed, Err(error.clone())).await;
    assert_eq!(outcome, Some(Err(error.clone())));
    let (_, ended) = keep.events(ActionHook::After).remove(1);
    assert_eq!(
        ended,
        Some(ActionEnd::Error(RunError::new(
            "TimeoutError",
            error.message
        )))
    );
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_failed_after_hook_fails_the_call_and_tells_only_the_host() {
    let failures = [
        HookFailure::Threw("Error: s3cret".to_string()),
        HookFailure::Timeout,
        HookFailure::BadDecision("{\"keep\":\"s3cret\"}".to_string()),
    ];
    let actual = json!({ "value": "Fake page" });
    for failure in failures {
        let timeline = Timeline::default();
        let run = run_scope(Duration::from_mins(1));
        let failed = failure.clone();
        let fake =
            Arc::new(FakeHooks::new(&timeline).after(move |_, _| decided(Err(failed.clone()))));
        let entry = entry();
        let after = admitted(&fake, "failing", &Arc::default(), false)
            .ran(&run)
            .unwrap();
        let Some(Err(error)) = after.finish(&entry, Ok(Some(actual.clone()))).await else {
            panic!("{failure:?} must fail the call");
        };
        assert_eq!(error.name, ErrorName::Error, "{failure:?}");
        assert!(!error.message.contains("s3cret"), "{error:?}");
        assert_eq!(
            record(&entry).hook,
            Some(after_record(AfterHookResult::Failed))
        );
        assert_eq!(fake.take_failures().len(), 1, "{failure:?}");
        assert!(fake.take_notes().is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_replacement_without_the_commands_shape_fails_the_call() {
    let timeline = Timeline::default();
    let actual = json!({ "value": "Fake page" });
    let binary = RunBinary {
        pointer: "/value".to_string(),
        bytes: vec![1, 2, 3],
    };
    let cases = [
        // A replacement after an error.
        (
            Some(json!({ "value": "x" })),
            Vec::new(),
            Err(PlaywrightError::timeout(5.0)),
        ),
        // Binaries, which no action's reply carries.
        (
            Some(json!({ "value": "x" })),
            vec![binary],
            Ok(Some(actual.clone())),
        ),
        // Not `title`'s result shape.
        (Some(json!(42)), Vec::new(), Ok(Some(actual.clone()))),
        (None, Vec::new(), Ok(Some(actual.clone()))),
        // A command that answers no result has none to replace.
        (Some(json!({})), Vec::new(), Ok(None)),
    ];
    for (value, binaries, outcome) in cases {
        let run = run_scope(Duration::from_mins(1));
        let decision = AfterDecision {
            replace: Some(Replaced { value, binaries }),
            notes: vec!["lost with the decision".to_string()],
        };
        let shown = format!("{decision:?} on {outcome:?}");
        let fake =
            Arc::new(FakeHooks::new(&timeline).after(move |_, _| decided(Ok(decision.clone()))));
        let entry = entry();
        let after = admitted(&fake, "shape", &Arc::default(), false)
            .ran(&run)
            .unwrap();
        let Some(Err(error)) = after.finish(&entry, outcome).await else {
            panic!("{shown} must fail the call");
        };
        assert_eq!(error.name, ErrorName::Error, "{shown}");
        assert_eq!(
            record(&entry).hook,
            Some(after_record(AfterHookResult::Failed)),
            "{shown}"
        );
        assert!(fake.take_notes().is_empty(), "{shown}");
        assert_eq!(fake.take_failures().len(), 1, "{shown}");
    }
}

#[tokio::test(start_paused = true)]
async fn tests_that_after_action_races_the_run_but_not_an_abort() {
    let timeline = Timeline::default();
    let dropped = Arc::new(AtomicBool::new(false));
    let abandoned = Arc::clone(&dropped);
    let stalls = Arc::new(FakeHooks::new(&timeline).after(move |_, _| never(&abandoned)));
    let run = run_scope(Duration::from_secs(10));
    let entry_stopped = entry();
    let after = admitted(&stalls, "stopped", &Arc::default(), false)
        .ran(&run)
        .unwrap();
    assert_eq!(after.finish(&entry_stopped, Ok(None)).await, None);
    assert!(
        dropped.load(Ordering::SeqCst),
        "the invocation is abandoned"
    );
    assert_eq!(record(&entry_stopped).hook, None);

    let slow = Arc::new(
        FakeHooks::new(&timeline).after(|_, _| later(Duration::from_secs(5), Ok(kept(Vec::new())))),
    );
    let run = run_scope(Duration::from_mins(1));
    let entry_aborted = entry();
    let after = admitted(&slow, "aborted", &Arc::default(), false)
        .ran(&run)
        .unwrap();
    let abort = async {
        time::sleep(Duration::from_secs(1)).await;
        entry_aborted.abort("too late".to_string());
    };
    let (outcome, ()) = tokio::join!(after.finish(&entry_aborted, Ok(None)), abort);
    assert_eq!(
        outcome,
        Some(Ok(None)),
        "the outcome was decided before the abort"
    );
    assert_eq!(
        record(&entry_aborted).hook,
        Some(after_record(AfterHookResult::Kept))
    );
}

/// A run's dispatcher on the fake browser's tab, with `hooks` as its action hooks.
struct Harness {
    browser: FakeBrowser,
    dispatcher: Dispatcher,
    /// The run's hook realm's dispatcher (`Dispatcher::for_hooks`).
    hook_realm: Dispatcher,
}

impl Harness {
    async fn start(hooks: Option<Arc<dyn ActionHooks>>, run_timeout: Duration) -> Self {
        Self::start_with(&Replies::default(), hooks, run_timeout).await
    }

    async fn start_with(
        replies: &Replies,
        hooks: Option<Arc<dyn ActionHooks>>,
        run_timeout: Duration,
    ) -> Self {
        let browser = FakeBrowser::start(*replies).await;
        let tasks = TaskTracker::new();
        let run = RunScope::new(
            Deadline::new(run_timeout, CancellationToken::new()),
            tasks.clone(),
        );
        let (tabs, _) = Tabs::open(
            browser.cdp().await,
            Handle::current(),
            TabInputs::default(),
            TabScreenshots::default(),
            Switches::default(),
            Permissions::default(),
            tasks,
            &PageId::new(PAGE.to_string()),
        )
        .await
        .unwrap_or_else(|error| panic!("opening the fake tab: {error:?}"));
        let dispatcher = Dispatcher::new(tabs, run);
        if let Some(hooks) = hooks {
            assert!(dispatcher.set_action_hooks(hooks));
        }
        let hook_realm = dispatcher.for_hooks();
        Self {
            browser,
            dispatcher,
            hook_realm,
        }
    }

    /// The reply to a message the hook realm sends: through the run's hook dispatcher, which
    /// has no action hooks.
    async fn hook_call(&self, message: &Value) -> Result<Value, BridgeError> {
        let (text, _) = self.hook_realm.call(&message.to_string()).await?;
        Ok(serde_json::from_str(&text).unwrap())
    }

    /// The reply to a message the script sends straight to its bridge, as JSON.
    async fn call(&self, message: &Value) -> Result<Value, BridgeError> {
        let (text, _) = self.dispatcher.call(&message.to_string()).await?;
        Ok(serde_json::from_str(&text).unwrap())
    }

    /// Where each mouse event the page received happened.
    fn mouse_events(&self) -> Vec<(Value, Value)> {
        self.browser
            .exchanges()
            .into_iter()
            .filter(|exchange| exchange.method() == "Input.dispatchMouseEvent")
            .map(|exchange| {
                let params = &exchange.command["params"];
                (params["x"].clone(), params["y"].clone())
            })
            .collect()
    }

    /// The HTML of every `setContent` write the page received, in order.
    fn writes(&self) -> Vec<Value> {
        self.glue_calls("writeContent", "html")
    }

    /// The `field` of every call of the glue function `name` the page received, in order.
    fn glue_calls(&self, name: &str, field: &str) -> Vec<Value> {
        self.browser
            .exchanges()
            .into_iter()
            .filter(|exchange| {
                exchange.method() == "Runtime.callFunctionOn"
                    && exchange.command["params"]["arguments"][0]["value"] == name
            })
            .map(|exchange| exchange.command["params"]["arguments"][1]["value"][field].clone())
            .collect()
    }

    async fn records(&self, reason: RunEnd) -> Vec<CallRecord> {
        self.dispatcher.finish(reason).await.0
    }
}

fn frame_guid() -> String {
    format!("frame@{PAGE}")
}

fn page_guid() -> String {
    format!("page@{PAGE}")
}

fn message(id: u64, guid: &str, method: &str, params: Value, metadata: Value) -> Value {
    let mut message = json!({ "id": id, "guid": guid, "method": method });
    message["params"] = params;
    message["metadata"] = metadata;
    message
}

fn mouse_click(id: u64, x: f64, y: f64) -> Value {
    message(
        id,
        &page_guid(),
        "mouseClick",
        json!({ "x": x, "y": y }),
        json!({}),
    )
}

fn hooks_of(fake: &Arc<FakeHooks>) -> Arc<dyn ActionHooks> {
    Arc::<FakeHooks>::clone(fake)
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_hooked_call_sees_its_event_and_runs_when_allowed() {
    let timeline = Timeline::default();
    let fake = Arc::new(FakeHooks::new(&timeline));
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    // A message straight to the bridge, flagged internal as the SDK's own calls are: the flag
    // decides nothing about hooks.
    let metadata = json!({
        "location": { "file": "run.js", "line": 3, "column": 7 },
        "internal": true,
    });
    let reply = harness
        .call(&message(1, &frame_guid(), "title", json!({}), metadata))
        .await
        .unwrap();
    assert_eq!(
        reply,
        json!({ "id": 1, "result": { "value": "Fake page" } })
    );
    let expected = ActionEvent {
        kind: Some("title"),
        group: Some("read"),
        method: "title".to_string(),
        guid: frame_guid(),
        args: json!({}),
        location: Some(CallLocation {
            line: 3,
            column: Some(7),
        }),
        page: Some(page_guid()),
        frame: Some(frame_guid()),
        context: "context@default".to_string(),
        locator: None,
        element: None,
    };
    assert_eq!(fake.events(ActionHook::Before), [(expected.clone(), None)]);
    assert_eq!(
        fake.events(ActionHook::After),
        [(
            expected,
            Some(ActionEnd::Result(json!({ "value": "Fake page" })))
        )]
    );
    let records = harness.records(RunEnd::Settled).await;
    assert_eq!(
        records[0].hook,
        Some(CallHookRecord {
            before: Some(BeforeHookResult::Allowed),
            after: Some(AfterHookResult::Kept),
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_frame_calls_selector_reaches_the_hook_as_its_locator() {
    let timeline = Timeline::default();
    let fake = Arc::new(FakeHooks::new(&timeline).wanting(|hook, _| hook == ActionHook::Before));
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    let params = json!({ "selector": "#go", "strict": true });
    let _reply = harness
        .call(&message(
            1,
            &frame_guid(),
            "textContent",
            params.clone(),
            json!({ "timeout": 100 }),
        ))
        .await
        .unwrap();
    let (seen, _) = fake.events(ActionHook::Before).remove(0);
    assert_eq!((seen.kind, seen.group), (Some("textContent"), Some("read")));
    assert_eq!(seen.args, params);
    assert_eq!(
        seen.locator,
        Some(LocatorRef {
            frame: frame_guid(),
            selector: "#go".to_string(),
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_blocked_input_call_sends_no_input() {
    let timeline = Timeline::default();
    let allow = Arc::new(FakeHooks::new(&timeline));
    let allowed = Harness::start(Some(hooks_of(&allow)), Duration::from_secs(30)).await;
    assert_eq!(
        allowed.call(&mouse_click(1, 10.0, 20.0)).await.unwrap(),
        json!({ "id": 1 })
    );
    assert!(
        !allowed.mouse_events().is_empty(),
        "an allowed click is sent"
    );
    assert_eq!(
        allowed.records(RunEnd::Settled).await[0].delivery,
        Some(Delivery::Done)
    );

    let block = Arc::new(
        FakeHooks::new(&timeline)
            .before(|_, _| decided(Ok(ActionDecision::Block("no clicking".to_string())))),
    );
    let blocked = Harness::start(Some(hooks_of(&block)), Duration::from_secs(30)).await;
    let reply = blocked.call(&mouse_click(1, 10.0, 20.0)).await.unwrap();
    assert_eq!(reply["error"]["error"]["name"], "Error");
    let message = reply["error"]["error"]["message"].as_str().unwrap();
    assert!(message.contains("no clicking"), "{reply}");
    assert!(
        blocked.mouse_events().is_empty(),
        "no input reached the page"
    );
    let record = blocked.records(RunEnd::Settled).await.remove(0);
    assert_eq!(record.delivery, Some(Delivery::NotSent));
    assert!(matches!(record.result, CallResult::Error { .. }));
    assert_eq!(record.hook, Some(before_record(BeforeHookResult::Blocked)));
    assert_eq!(
        block.invocations(),
        1,
        "afterAction never sees a blocked call"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_replaced_arguments_run_and_the_record_keeps_the_scripts() {
    let timeline = Timeline::default();
    let fake = Arc::new(FakeHooks::new(&timeline).before(|_, _| {
        decided(Ok(ActionDecision::ReplaceArgs(
            json!({ "x": 30.0, "y": 40.0 }),
        )))
    }));
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    assert_eq!(
        harness.call(&mouse_click(1, 10.0, 20.0)).await.unwrap(),
        json!({ "id": 1 })
    );
    let events = harness.mouse_events();
    assert!(!events.is_empty());
    assert!(
        events
            .iter()
            .all(|position| *position == (json!(30.0), json!(40.0))),
        "the click ran with the hook's arguments: {events:?}"
    );
    let (seen, _) = fake.events(ActionHook::After).remove(0);
    assert_eq!(seen.args, json!({ "x": 30.0, "y": 40.0 }));
    let record = harness.records(RunEnd::Settled).await.remove(0);
    assert_eq!(record.params, json!({ "x": 10.0, "y": 20.0 }));
    assert_eq!(
        record.hook,
        Some(CallHookRecord {
            before: Some(BeforeHookResult::ArgsReplaced),
            after: Some(AfterHookResult::Kept),
        })
    );
    assert!(record.log.is_empty());
    assert_eq!(record.delivery, Some(Delivery::Done));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_call_run_with_replaced_arguments_keeps_no_call_log() {
    let timeline = Timeline::default();
    let read = |selector: &str| {
        message(
            1,
            &frame_guid(),
            "textContent",
            json!({ "selector": selector, "strict": true }),
            json!({ "timeout": 300 }),
        )
    };
    // The same read without a swap: it waits for its element, and its log says so.
    let allow = Arc::new(FakeHooks::new(&timeline));
    let allowed = Harness::start(Some(hooks_of(&allow)), Duration::from_secs(30)).await;
    let reply = allowed.call(&read("#nothing")).await.unwrap();
    assert_eq!(reply["error"]["error"]["name"], "TimeoutError");
    assert_ne!(reply["log"], json!([]), "{reply}");
    assert!(!allowed.records(RunEnd::Settled).await[0].log.is_empty());

    let swap = Arc::new(FakeHooks::new(&timeline).before(|_, _| {
        decided(Ok(ActionDecision::ReplaceArgs(
            json!({ "selector": "#s3cret", "strict": true }),
        )))
    }));
    let swapped = Harness::start(Some(hooks_of(&swap)), Duration::from_secs(30)).await;
    let reply = swapped.call(&read("#nothing")).await.unwrap();
    assert_eq!(reply["error"]["error"]["name"], "TimeoutError");
    assert_eq!(reply["log"], json!([]), "{reply}");
    assert!(!reply.to_string().contains("s3cret"), "{reply}");
    let record = swapped.records(RunEnd::Settled).await.remove(0);
    assert!(record.log.is_empty());
    assert_eq!(record.params["selector"], "#nothing");
    let (seen, _) = swap.events(ActionHook::After).remove(0);
    assert_eq!(
        seen.locator,
        Some(LocatorRef {
            frame: frame_guid(),
            selector: "#s3cret".to_string(),
        }),
        "the hook's own arguments name the locator it sees after"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_replaced_result_and_a_note_reach_the_script_and_the_ledger() {
    let timeline = Timeline::default();
    let fake = Arc::new(FakeHooks::new(&timeline).after(|_, _| {
        decided(Ok(AfterDecision {
            replace: Some(Replaced {
                value: Some(json!({ "value": "Hooked" })),
                binaries: Vec::new(),
            }),
            notes: vec!["the title was redacted".to_string()],
        }))
    }));
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    let reply = harness
        .call(&message(1, &frame_guid(), "title", json!({}), json!({})))
        .await
        .unwrap();
    assert_eq!(reply, json!({ "id": 1, "result": { "value": "Hooked" } }));
    assert_eq!(fake.take_notes(), ["the title was redacted"]);
    let record = harness.records(RunEnd::Settled).await.remove(0);
    assert_eq!(record.result, CallResult::Ok);
    assert_eq!(
        record.hook,
        Some(CallHookRecord {
            before: Some(BeforeHookResult::Allowed),
            after: Some(AfterHookResult::Replaced),
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_replacement_of_the_wrong_shape_fails_the_call() {
    let timeline = Timeline::default();
    let fake = Arc::new(FakeHooks::new(&timeline).after(|_, _| decided(Ok(replaced(json!(42))))));
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    let reply = harness
        .call(&message(1, &frame_guid(), "title", json!({}), json!({})))
        .await
        .unwrap();
    assert_eq!(reply["error"]["error"]["name"], "Error");
    assert_eq!(reply.get("result"), None, "{reply}");
    let record = harness.records(RunEnd::Settled).await.remove(0);
    assert!(matches!(record.result, CallResult::Error { .. }));
    assert_eq!(
        record.hook.and_then(|hook| hook.after),
        Some(AfterHookResult::Failed)
    );
    assert_eq!(fake.take_failures().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_failing_hooks_fail_closed_around_an_input_call() {
    let failures = [
        HookFailure::Threw("Error: s3cret".to_string()),
        HookFailure::Timeout,
        HookFailure::BadDecision("{\"allow\":\"s3cret\"}".to_string()),
    ];
    for failure in failures {
        // Before: the click never runs.
        let timeline = Timeline::default();
        let failed = failure.clone();
        let fake =
            Arc::new(FakeHooks::new(&timeline).before(move |_, _| decided(Err(failed.clone()))));
        let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
        let reply = harness.call(&mouse_click(1, 10.0, 20.0)).await.unwrap();
        assert_eq!(reply["error"]["error"]["name"], "Error", "{failure:?}");
        assert!(!reply.to_string().contains("s3cret"), "{reply}");
        assert!(harness.mouse_events().is_empty(), "{failure:?}");
        let record = harness.records(RunEnd::Settled).await.remove(0);
        assert_eq!(record.delivery, Some(Delivery::NotSent), "{failure:?}");
        assert_eq!(record.hook, Some(before_record(BeforeHookResult::Failed)));
        assert_eq!(fake.take_failures().len(), 1);

        // After: the click ran; it fails, and its delivery still says it was sent.
        let failed = failure.clone();
        let fake = Arc::new(
            FakeHooks::new(&timeline)
                .wanting(|hook, _| hook == ActionHook::After)
                .after(move |_, _| decided(Err(failed.clone()))),
        );
        let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
        let reply = harness.call(&mouse_click(1, 10.0, 20.0)).await.unwrap();
        assert_eq!(reply["error"]["error"]["name"], "Error", "{failure:?}");
        assert!(!reply.to_string().contains("s3cret"), "{reply}");
        assert!(!harness.mouse_events().is_empty(), "{failure:?}");
        let record = harness.records(RunEnd::Settled).await.remove(0);
        assert_eq!(record.delivery, Some(Delivery::Done), "{failure:?}");
        assert_eq!(record.hook, Some(after_record(AfterHookResult::Failed)));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_replaced_arguments_the_command_refuses_block_the_call() {
    let timeline = Timeline::default();
    let fake = Arc::new(FakeHooks::new(&timeline).before(|_, _| {
        decided(Ok(ActionDecision::ReplaceArgs(
            json!({ "x": "s3cret", "y": 20 }),
        )))
    }));
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    let reply = harness.call(&mouse_click(1, 10.0, 20.0)).await.unwrap();
    assert_eq!(reply["error"]["error"]["name"], "Error");
    assert!(!reply.to_string().contains("s3cret"), "{reply}");
    assert!(harness.mouse_events().is_empty());
    let record = harness.records(RunEnd::Settled).await.remove(0);
    assert_eq!(record.delivery, Some(Delivery::NotSent));
    assert_eq!(record.hook, Some(before_record(BeforeHookResult::Failed)));
    assert_eq!(record.params, json!({ "x": 10.0, "y": 20.0 }));
    let trace = fake.take_failures();
    assert_eq!(trace.len(), 1, "{trace:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_hooked_calls_are_decided_in_call_order() {
    let timeline = Timeline::default();
    let decisions = timeline.clone();
    // The first call's hook takes longest.
    let fake = Arc::new(FakeHooks::new(&timeline).before(move |invocation, event| {
        let delay = Duration::from_millis(if invocation == 0 { 300 } else { 0 });
        let decisions = decisions.clone();
        async move {
            time::sleep(delay).await;
            decisions.push(format!("decided {}", tag(&event)));
            Ok(ActionDecision::Allow)
        }
        .boxed()
    }));
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    let title_on = |id: u64, line: u64| {
        message(
            id,
            &frame_guid(),
            "title",
            json!({}),
            json!({ "location": { "file": "run.js", "line": line } }),
        )
    };
    let (first, second) = (title_on(1, 1), title_on(2, 2));
    let (first, second) = tokio::join!(harness.call(&first), harness.call(&second));
    assert_eq!(first.unwrap()["result"], json!({ "value": "Fake page" }));
    assert_eq!(second.unwrap()["result"], json!({ "value": "Fake page" }));
    let lines: Vec<String> = timeline
        .lines()
        .into_iter()
        .filter(|line| !line.starts_with("after"))
        .collect();
    assert_eq!(
        lines,
        [
            "before line 1",
            "decided line 1",
            "before line 2",
            "decided line 2"
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_the_gate_wait_does_not_count_against_the_calls_timeout() {
    let timeline = Timeline::default();
    let fake = Arc::new(
        FakeHooks::new(&timeline)
            .before(|_, _| later(Duration::from_millis(1500), Ok(ActionDecision::Allow))),
    );
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    let reply = harness
        .call(&message(
            1,
            &frame_guid(),
            "title",
            json!({}),
            json!({ "timeout": 1000 }),
        ))
        .await
        .unwrap();
    assert_eq!(
        reply,
        json!({ "id": 1, "result": { "value": "Fake page" } })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_an_abort_at_the_gate_replies_abort_error() {
    let timeline = Timeline::default();
    let dropped = Arc::new(AtomicBool::new(false));
    let abandoned = Arc::clone(&dropped);
    let fake = Arc::new(FakeHooks::new(&timeline).before(move |_, _| never(&abandoned)));
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    let title = message(7, &frame_guid(), "title", json!({}), json!({}));
    let call = harness.call(&title);
    let abort = async {
        while fake.invocations() == 0 {
            task::yield_now().await;
        }
        harness.dispatcher.abort(7, "stop waiting".to_string());
    };
    let (reply, ()) = tokio::join!(call, abort);
    let reply = reply.unwrap();
    assert_eq!(reply["error"]["error"]["name"], "AbortError");
    assert_eq!(reply["error"]["error"]["message"], "stop waiting");
    assert!(
        dropped.load(Ordering::SeqCst),
        "the invocation is abandoned"
    );
    let record = harness.records(RunEnd::Settled).await.remove(0);
    assert_eq!(
        record.result,
        CallResult::Error {
            name: "AbortError".to_string(),
            message: "stop waiting".to_string(),
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_the_run_deadline_at_the_gate_ends_the_call() {
    let timeline = Timeline::default();
    let dropped = Arc::new(AtomicBool::new(false));
    let abandoned = Arc::clone(&dropped);
    let fake = Arc::new(FakeHooks::new(&timeline).before(move |_, _| never(&abandoned)));
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_millis(500)).await;
    let reply = harness
        .call(&message(1, &frame_guid(), "title", json!({}), json!({})))
        .await;
    assert!(matches!(reply, Err(BridgeError::NoReply)), "{reply:?}");
    assert!(
        dropped.load(Ordering::SeqCst),
        "the invocation is abandoned"
    );
    let record = harness.records(RunEnd::Timeout).await.remove(0);
    assert_eq!(
        record.result,
        CallResult::Unfinished {
            reason: RunEnd::Timeout
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_element_actions_unwanted_kinds_and_runs_without_hooks_are_not_hooked() {
    let timeline = Timeline::default();
    let fake = Arc::new(FakeHooks::new(&timeline));
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    let click = message(
        1,
        &frame_guid(),
        "click",
        json!({ "selector": "#go", "strict": true }),
        json!({}),
    );
    assert_eq!(harness.call(&click).await.unwrap(), json!({ "id": 1 }));
    let fill = message(
        2,
        &frame_guid(),
        "fill",
        json!({ "selector": "#go", "strict": true, "value": "x" }),
        json!({ "timeout": 100 }),
    );
    let _reply = harness.call(&fill).await.unwrap();
    assert_eq!(
        fake.asked.load(Ordering::SeqCst),
        0,
        "element actions are feature 4's"
    );
    assert_eq!(fake.invocations(), 0);
    let records = harness.records(RunEnd::Settled).await;
    assert!(records.iter().all(|record| record.hook.is_none()));

    let unwanted = Arc::new(FakeHooks::new(&timeline).wanting(|_, kind| kind != "title"));
    let harness = Harness::start(Some(hooks_of(&unwanted)), Duration::from_secs(30)).await;
    let title = message(1, &frame_guid(), "title", json!({}), json!({}));
    let reply = harness.call(&title).await.unwrap();
    assert_eq!(
        reply,
        json!({ "id": 1, "result": { "value": "Fake page" } })
    );
    assert_eq!(
        unwanted.asked.load(Ordering::SeqCst),
        2,
        "one question per hook"
    );
    assert_eq!(unwanted.invocations(), 0);
    assert_eq!(harness.records(RunEnd::Settled).await[0].hook, None);

    let harness = Harness::start(None, Duration::from_secs(30)).await;
    let reply = harness.call(&title).await.unwrap();
    assert_eq!(
        reply,
        json!({ "id": 1, "result": { "value": "Fake page" } })
    );
    assert_eq!(harness.records(RunEnd::Settled).await[0].hook, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_set_content_held_at_its_gate_lets_the_hooks_own_write_go_first() {
    let timeline = Timeline::default();
    let (release, released) = oneshot::channel::<()>();
    let released = Mutex::new(Some(released));
    // The hook realm's own write goes through its own dispatcher, which has no hooks.
    let fake = Arc::new(
        FakeHooks::new(&timeline)
            .wanting(|hook, _| hook == ActionHook::Before)
            .before(move |_, _| match released.lock().unwrap().take() {
                Some(released) => async move {
                    let _released = released.await;
                    Ok(ActionDecision::Allow)
                }
                .boxed(),
                None => decided(Ok(ActionDecision::Allow)),
            }),
    );
    let harness = Arc::new(Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await);
    let write = |id: u64, html: &str| {
        message(
            id,
            &frame_guid(),
            "setContent",
            json!({ "html": html, "waitUntil": "commit" }),
            json!({}),
        )
    };
    let agent = {
        let harness = Arc::clone(&harness);
        let message = write(1, "<p>agent</p>");
        tokio::spawn(async move { harness.call(&message).await })
    };
    while fake.invocations() == 0 {
        task::yield_now().await;
    }
    let hook_write = time::timeout(
        Duration::from_secs(10),
        harness.hook_call(&write(2, "<p>hook</p>")),
    )
    .await;
    assert!(
        matches!(hook_write, Ok(Ok(_))),
        "the hook's write never waited for the held one: {hook_write:?}"
    );
    assert!(
        !agent.is_finished(),
        "the agent's write is still held at its gate"
    );
    release.send(()).unwrap();
    let agent_reply = time::timeout(Duration::from_secs(10), agent).await;
    assert!(matches!(agent_reply, Ok(Ok(Ok(_)))), "{agent_reply:?}");
    assert_eq!(
        harness.writes(),
        [json!("<p>hook</p>"), json!("<p>agent</p>")]
    );
}

fn set_content(id: u64, html: &str) -> Value {
    message(
        id,
        &frame_guid(),
        "setContent",
        json!({ "html": html, "waitUntil": "commit" }),
        json!({}),
    )
}

fn set_item(id: u64, value: &str) -> Value {
    message(
        id,
        &page_guid(),
        "webStorageSetItem",
        json!({ "kind": "local", "name": "k", "value": value }),
        json!({}),
    )
}

/// A hook that replaces every call's arguments with `args`, and wants no `afterAction`.
fn replacing(timeline: &Timeline, args: &Value) -> Arc<FakeHooks> {
    let args = args.clone();
    Arc::new(
        FakeHooks::new(timeline)
            .wanting(|hook, _| hook == ActionHook::Before)
            .before(move |_, _| decided(Ok(ActionDecision::ReplaceArgs(args.clone())))),
    )
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_call_run_with_replaced_arguments_replies_none_of_its_error_text() {
    let timeline = Timeline::default();
    let fake = replacing(&timeline, &json!({ "name": "s3cret" }));
    let failures = [
        PlaywrightError::error("Unknown permission: s3cret"),
        PlaywrightError::timeout(5.0),
    ];
    for failure in failures {
        let run = run_scope(Duration::from_mins(1));
        let entry = entry();
        let mut hooks = admitted(&fake, "replaced", &Arc::default(), false);
        assert_eq!(
            gate(&mut hooks, &run, &entry).await,
            Gated::Run("settled rerouted s3cret".to_string())
        );
        // No afterAction wants the call; its reply still has work to do.
        let reply = hooks.ran(&run).unwrap();
        let Some(Err(replied)) = reply.finish(&entry, Err(failure.clone())).await else {
            panic!("{failure:?} must still fail the call");
        };
        assert_eq!(replied.name, failure.name, "the error keeps its name");
        assert!(!replied.message.contains("s3cret"), "{replied:?}");
        let trace = fake.take_failures();
        assert!(
            trace.iter().any(|line| line.contains(&failure.message)),
            "the host still reads the real text: {trace:?}"
        );
    }
    let run = run_scope(Duration::from_mins(1));
    let entry = entry();
    let mut hooks = admitted(&fake, "replaced", &Arc::default(), false);
    gate(&mut hooks, &run, &entry).await;
    let succeeded = Ok(Some(json!({ "value": "s3cret page" })));
    let replied = hooks
        .ran(&run)
        .unwrap()
        .finish(&entry, succeeded.clone())
        .await;
    assert_eq!(replied, Some(succeeded), "a result is the hook's to filter");
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_replaced_or_withheld_result_keeps_no_call_log() {
    let timeline = Timeline::default();
    let decisions = [
        (Ok(kept(Vec::new())), true),
        (Ok(replaced(json!({ "value": "Hooked" }))), false),
        (Ok(replaced(json!(42))), false),
        (Err(HookFailure::Threw("redactor broke".to_string())), false),
    ];
    for (decision, keeps_log) in decisions {
        let shown = format!("{decision:?}");
        let fake = Arc::new(
            FakeHooks::new(&timeline)
                .wanting(|hook, _| hook == ActionHook::After)
                .after(move |_, _| decided(decision.clone())),
        );
        let run = run_scope(Duration::from_mins(1));
        let entry = entry();
        let progress = Progress::new(
            Arc::clone(&entry),
            0.0,
            run.clone(),
            CancellationToken::new(),
        );
        progress.log("locator resolved to <span id=\"balance\">$12,345.67</span>");
        let reply = admitted(&fake, "read", &Arc::default(), false)
            .ran(&run)
            .unwrap();
        reply
            .finish(&entry, Ok(Some(json!({ "value": "$12,345.67" }))))
            .await;
        assert_eq!(!progress.folded_log().is_empty(), keeps_log, "{shown}");
    }
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_call_that_never_ran_gets_no_after_action() {
    let timeline = Timeline::default();
    let fake = Arc::new(FakeHooks::new(&timeline).wanting(|hook, _| hook == ActionHook::After));
    let run = run_scope(Duration::from_mins(1));
    let entry = entry();
    let mut reply = admitted(&fake, "aborted", &Arc::default(), false)
        .ran(&run)
        .unwrap();
    reply.never_ran();
    let aborted = Err(PlaywrightError::aborted("stop"));
    assert_eq!(reply.finish(&entry, aborted.clone()).await, Some(aborted));
    assert!(fake.events(ActionHook::After).is_empty());
    assert_eq!(record(&entry).hook, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_errors_quoting_replaced_arguments_never_reach_the_script() {
    let timeline = Timeline::default();
    // Neo refuses the permission by name: `Unknown permission: <value>`.
    let cases = [(
        message(
            1,
            "context@default",
            "grantPermissions",
            json!({ "permissions": ["geolocation"] }),
            json!({}),
        ),
        json!({ "permissions": ["s3cret"] }),
    )];
    for (call, swapped) in cases {
        let fake = replacing(&timeline, &swapped);
        let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
        let reply = harness.call(&call).await.unwrap();
        assert_eq!(reply["error"]["error"]["name"], "Error", "{reply}");
        assert!(!reply.to_string().contains("s3cret"), "{reply}");
        let record = harness.records(RunEnd::Settled).await.remove(0);
        assert_eq!(record.params, call["params"]);
        assert_eq!(
            record.hook,
            Some(before_record(BeforeHookResult::ArgsReplaced))
        );
        let CallResult::Error { name, message } = &record.result else {
            panic!("{call}: the call fails: {:?}", record.result);
        };
        assert_eq!(name, "Error");
        assert!(!message.contains("s3cret"), "{message}");
        let trace = fake.take_failures();
        assert!(
            trace.iter().any(|line| line.contains("s3cret")),
            "{call}: the host reads the real text: {trace:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_failed_goto_to_a_replaced_url_never_names_it() {
    let timeline = Timeline::default();
    let replies = Replies {
        navigate_error: Some("net::ERR_NAME_NOT_RESOLVED"),
        ..Replies::default()
    };
    let goto = message(
        1,
        &frame_guid(),
        "goto",
        json!({ "url": "https://example.test/public", "waitUntil": "load" }),
        json!({ "timeout": 2000 }),
    );
    // Without a swap, the error names the URL the script gave.
    let allow = Arc::new(FakeHooks::new(&timeline));
    let allowed =
        Harness::start_with(&replies, Some(hooks_of(&allow)), Duration::from_secs(30)).await;
    let reply = allowed.call(&goto).await.unwrap();
    let message = reply["error"]["error"]["message"].as_str().unwrap();
    assert!(message.contains("https://example.test/public"), "{reply}");

    let swap = replacing(
        &timeline,
        &json!({ "url": "https://example.test/s3cret", "waitUntil": "load" }),
    );
    let swapped =
        Harness::start_with(&replies, Some(hooks_of(&swap)), Duration::from_secs(30)).await;
    let reply = swapped.call(&goto).await.unwrap();
    assert_eq!(reply["error"]["error"]["name"], "Error");
    assert!(!reply.to_string().contains("s3cret"), "{reply}");
    let record = swapped.records(RunEnd::Settled).await.remove(0);
    assert!(!format!("{:?}", record.result).contains("s3cret"));
    let trace = swap.take_failures();
    assert!(
        trace
            .iter()
            .any(|line| line.contains("https://example.test/s3cret")),
        "{trace:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_replaced_or_withheld_result_leaves_no_page_preview_behind() {
    let timeline = Timeline::default();
    let bounding_box = message(
        1,
        &frame_guid(),
        "neo:boundingBox",
        json!({ "selector": "#go", "strict": true }),
        json!({ "timeout": 2000 }),
    );
    let decisions = [
        (Ok(kept(Vec::new())), Some(AfterHookResult::Kept)),
        (
            Ok(replaced(json!({ "value": {} }))),
            Some(AfterHookResult::Replaced),
        ),
        (Ok(replaced(json!(null))), Some(AfterHookResult::Failed)),
    ];
    for (decision, after) in decisions {
        let fake = Arc::new(
            FakeHooks::new(&timeline)
                .wanting(|hook, _| hook == ActionHook::After)
                .after(move |_, _| decided(decision.clone())),
        );
        let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
        let reply = harness.call(&bounding_box).await.unwrap();
        let record = harness.records(RunEnd::Settled).await.remove(0);
        assert_eq!(record.hook.and_then(|hook| hook.after), after);
        let previews = record.log.iter().any(|line| line.contains("<button"));
        assert_eq!(
            previews,
            after == Some(AfterHookResult::Kept),
            "{:?}",
            record.log
        );
        if after == Some(AfterHookResult::Failed) {
            assert_eq!(reply["log"], json!([]), "{reply}");
        }
    }

    // A failed afterAction withholds the log too, whether the read found its element or not.
    for (selector, timeout) in [("#go", 2000), ("#nothing", 300)] {
        let fake = Arc::new(
            FakeHooks::new(&timeline)
                .wanting(|hook, _| hook == ActionHook::After)
                .after(|_, _| decided(Err(HookFailure::Threw("redactor broke".to_string())))),
        );
        let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
        let read = message(
            1,
            &frame_guid(),
            "textContent",
            json!({ "selector": selector, "strict": true }),
            json!({ "timeout": timeout }),
        );
        let reply = harness.call(&read).await.unwrap();
        assert_eq!(
            reply["error"]["error"]["name"], "Error",
            "{selector}: {reply}"
        );
        assert_eq!(reply["log"], json!([]), "{selector}: {reply}");
        assert!(
            harness.records(RunEnd::Settled).await[0].log.is_empty(),
            "{selector}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_call_waiting_for_its_after_action_holds_no_write_turn() {
    let timeline = Timeline::default();
    let (release, released) = oneshot::channel::<()>();
    let released = Mutex::new(Some(released));
    // The agent's write is hooked after it runs; the second is the hook realm's own.
    let fake = Arc::new(FakeHooks::new(&timeline).after(move |_, _| {
        match released.lock().unwrap().take() {
            Some(released) => async move {
                let _released = released.await;
                Ok(kept(Vec::new()))
            }
            .boxed(),
            None => decided(Ok(kept(Vec::new()))),
        }
    }));
    let harness = Arc::new(Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await);
    let agent = {
        let harness = Arc::clone(&harness);
        tokio::spawn(async move { harness.call(&set_content(1, "<p>agent</p>")).await })
    };
    while fake.events(ActionHook::After).is_empty() {
        task::yield_now().await;
    }
    let hook_write = time::timeout(
        Duration::from_secs(10),
        harness.hook_call(&set_content(2, "<p>hook</p>")),
    )
    .await;
    assert!(matches!(hook_write, Ok(Ok(_))), "{hook_write:?}");
    assert!(
        !agent.is_finished(),
        "the agent's write is still in its afterAction"
    );
    release.send(()).unwrap();
    let agent_reply = time::timeout(Duration::from_secs(10), agent).await;
    assert!(matches!(agent_reply, Ok(Ok(Ok(_)))), "{agent_reply:?}");
    assert_eq!(
        harness.writes(),
        [json!("<p>agent</p>"), json!("<p>hook</p>")]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_call_aborted_as_its_hook_allows_it_never_runs_its_after_action() {
    let timeline = Timeline::default();
    let slot: Arc<Mutex<Option<Dispatcher>>> = Arc::default();
    let aborter = Arc::clone(&slot);
    let fake = Arc::new(FakeHooks::new(&timeline).before(move |_, _| {
        let aborter = Arc::clone(&aborter);
        async move {
            // Inside the decision's own poll: the gate's race saw no abort, and returns
            // this decision; the call's task then finds the abort before its handler.
            let dispatcher = aborter.lock().unwrap().clone().unwrap();
            dispatcher.abort(1, "aborted as it was decided".to_string());
            Ok(ActionDecision::Allow)
        }
        .boxed()
    }));
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    *slot.lock().unwrap() = Some(harness.dispatcher.clone());
    let agent_reply = harness.call(&set_content(1, "<p>agent</p>")).await.unwrap();
    assert_eq!(agent_reply["error"]["error"]["name"], "AbortError");
    assert!(
        fake.events(ActionHook::After).is_empty(),
        "the call never ran"
    );
    let hook_write = time::timeout(
        Duration::from_secs(10),
        harness.hook_call(&set_content(2, "<p>hook</p>")),
    )
    .await;
    assert!(matches!(hook_write, Ok(Ok(_))), "{hook_write:?}");
    assert_eq!(
        harness.writes(),
        [json!("<p>hook</p>")],
        "the aborted write never ran"
    );
    let record = harness.records(RunEnd::Settled).await.remove(0);
    assert_eq!(record.hook, Some(before_record(BeforeHookResult::Allowed)));
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_replaced_arguments_are_admitted_like_the_scripts() {
    let timeline = Timeline::default();
    // Past the in-flight byte cap on their own.
    let padded = json!({ "padding": "x".repeat(9 << 20) });
    let fake = replacing(&timeline, &padded);
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    let reply = harness
        .call(&message(1, &frame_guid(), "title", json!({}), json!({})))
        .await
        .unwrap();
    assert_eq!(reply["error"]["error"]["name"], "Error", "{reply}");
    let record = harness.records(RunEnd::Settled).await.remove(0);
    assert_eq!(record.hook, Some(before_record(BeforeHookResult::Failed)));

    // A replacement may not change what the call ends among the calls in flight.
    let release = |cursor: u64| {
        message(
            1,
            &frame_guid(),
            "neo:releaseNavigation",
            json!({ "cursor": cursor }),
            json!({}),
        )
    };
    for (cursor, before) in [
        (6, BeforeHookResult::Failed),
        (5, BeforeHookResult::ArgsReplaced),
    ] {
        let fake = replacing(&timeline, &json!({ "cursor": cursor }));
        let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
        let reply = harness.call(&release(5)).await.unwrap();
        let record = harness.records(RunEnd::Settled).await.remove(0);
        assert_eq!(
            record.hook,
            Some(before_record(before)),
            "cursor {cursor}: {reply}"
        );
        assert_eq!(
            reply.get("error").is_some(),
            before == BeforeHookResult::Failed,
            "cursor {cursor}: {reply}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_hooked_writes_into_one_frame_go_in_arrival_order() {
    let timeline = Timeline::default();
    // The first write's hook takes longest.
    let fake = Arc::new(
        FakeHooks::new(&timeline)
            .wanting(|hook, _| hook == ActionHook::Before)
            .before(|invocation, _| {
                let delay = Duration::from_millis(if invocation == 0 { 300 } else { 0 });
                later(delay, Ok(ActionDecision::Allow))
            }),
    );
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    let (first, second) = (
        set_content(1, "<p>first</p>"),
        set_content(2, "<p>second</p>"),
    );
    let (first, second) = tokio::join!(harness.call(&first), harness.call(&second));
    assert!(first.is_ok() && second.is_ok());
    assert_eq!(
        harness.writes(),
        [json!("<p>first</p>"), json!("<p>second</p>")]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_storage_call_held_at_its_gate_lets_the_hooks_own_storage_call_go_first() {
    let timeline = Timeline::default();
    let (release, released) = oneshot::channel::<()>();
    let released = Mutex::new(Some(released));
    // The hook realm's own call goes through its own dispatcher, which has no hooks.
    let fake = Arc::new(
        FakeHooks::new(&timeline)
            .wanting(|hook, _| hook == ActionHook::Before)
            .before(move |_, _| match released.lock().unwrap().take() {
                Some(released) => async move {
                    let _released = released.await;
                    Ok(ActionDecision::Allow)
                }
                .boxed(),
                None => decided(Ok(ActionDecision::Allow)),
            }),
    );
    let harness = Arc::new(Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await);
    let agent = {
        let harness = Arc::clone(&harness);
        tokio::spawn(async move { harness.call(&set_item(1, "agent")).await })
    };
    while fake.invocations() == 0 {
        task::yield_now().await;
    }
    let hook_call = time::timeout(
        Duration::from_secs(10),
        harness.hook_call(&set_item(2, "hook")),
    )
    .await;
    assert!(matches!(hook_call, Ok(Ok(_))), "{hook_call:?}");
    assert!(
        !agent.is_finished(),
        "the agent's call is still held at its gate"
    );
    release.send(()).unwrap();
    let agent_reply = time::timeout(Duration::from_secs(10), agent).await;
    assert!(matches!(agent_reply, Ok(Ok(Ok(_)))), "{agent_reply:?}");
    assert_eq!(
        harness.glue_calls("webStorageSetItem", "value"),
        [json!("hook"), json!("agent")]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_replaced_call_that_ends_calls_in_flight_is_never_refused_by_the_caps() {
    let timeline = Timeline::default();
    // Only the context's close is hooked: its arguments are replaced.
    let fake = Arc::new(
        FakeHooks::new(&timeline)
            .wanting(|hook, kind| hook == ActionHook::Before && kind == "close")
            .before(|_, _| {
                decided(Ok(ActionDecision::ReplaceArgs(
                    json!({ "reason": "the hook closes it" }),
                )))
            }),
    );
    let harness = Arc::new(Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await);
    // An unhooked wait whose message holds the whole byte cap.
    let mut filler = message(
        1,
        &frame_guid(),
        "waitForTimeout",
        json!({ "waitTimeout": 60_000, "padding": "" }),
        json!({}),
    );
    let room = (8 << 20) - filler.to_string().len();
    filler["params"]["padding"] = json!("x".repeat(room));
    assert_eq!(filler.to_string().len(), 8 << 20);
    // Admitted as it is sent, before anything below.
    let waiting = tokio::spawn(harness.dispatcher.call(&filler.to_string()));
    // A call that needs a share is refused now: the wait holds the cap.
    let refused = harness
        .call(&message(2, &frame_guid(), "title", json!({}), json!({})))
        .await
        .unwrap();
    assert_eq!(refused["error"]["error"]["name"], "Error", "{refused}");

    let close = message(3, "context@default", "close", json!({}), json!({}));
    let reply = time::timeout(Duration::from_secs(10), harness.call(&close)).await;
    let reply = reply.unwrap().unwrap();
    assert_eq!(
        reply,
        json!({ "id": 3 }),
        "the close ran with the hook's arguments"
    );
    let waited = time::timeout(Duration::from_secs(10), waiting).await;
    let (waited, _) = waited.unwrap().unwrap().unwrap();
    let waited: Value = serde_json::from_str(&waited).unwrap();
    assert_eq!(
        waited["error"]["error"]["name"], "TargetClosedError",
        "the close let the run go: {waited}"
    );
    // The wait's 8 MiB of params pass the run's record budget, so later calls keep no record;
    // the hook saw the close, and nothing else.
    let (seen, _) = fake.events(ActionHook::Before).remove(0);
    assert_eq!(seen.method, "close");
    assert_eq!(fake.invocations(), 1);
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_call_meeting_its_runs_closed_hooks_stops_without_a_hook_failure() {
    let timeline = Timeline::default();
    let gone = || {
        decided(Err(HookFailure::Realm(
            "the hook realm is gone".to_string(),
        )))
    };
    let fake = Arc::new(
        FakeHooks::new(&timeline)
            .before(move |_, _| gone())
            .after(move |_, _| {
                decided(Err(HookFailure::Realm(
                    "the hook realm is gone".to_string(),
                )))
            }),
    );
    fake.closed.store(true, Ordering::SeqCst);
    let run = run_scope(Duration::from_mins(1));

    let entry_before = entry();
    let mut hooks = admitted(&fake, "late", &Arc::default(), false);
    assert_eq!(
        gate(&mut hooks, &run, &entry_before).await,
        Gated::Stopped,
        "the call stops as one the run ended"
    );
    assert_eq!(record(&entry_before).hook, None, "no hook failed");

    let entry_after = entry();
    let reply = admitted(&fake, "late", &Arc::default(), false)
        .ran(&run)
        .unwrap();
    assert_eq!(reply.finish(&entry_after, Ok(None)).await, None, "no reply");
    assert_eq!(record(&entry_after).hook, None);
    assert!(fake.take_failures().is_empty(), "no failure line");

    // While the run still has its hooks, the same answer is a hook failure.
    fake.closed.store(false, Ordering::SeqCst);
    let entry_open = entry();
    let mut hooks = admitted(&fake, "open", &Arc::default(), false);
    assert!(matches!(
        gate(&mut hooks, &run, &entry_open).await,
        Gated::Refused(_)
    ));
    assert_eq!(
        record(&entry_open).hook,
        Some(before_record(BeforeHookResult::Failed))
    );
    assert_eq!(fake.take_failures().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_a_replacement_is_charged_unless_its_call_holds_a_claim() {
    let timeline = Timeline::default();
    // A binding call's answer names a claim, but for a binding call nobody waits on it takes a
    // share, so its replacement's bytes are charged, and a 9 MiB result is past the cap.
    let padded = json!({ "result": { "value": { "s": "x".repeat(9 << 20) }, "handles": [] } });
    let fake = replacing(&timeline, &padded);
    let harness = Harness::start(Some(hooks_of(&fake)), Duration::from_secs(30)).await;
    let small = json!({ "result": { "value": { "v": "undefined" }, "handles": [] } });
    let resolve = message(1, "bindingcall@nobody", "resolve", small, json!({}));
    let reply = harness.call(&resolve).await.unwrap();
    assert_eq!(reply["error"]["error"]["name"], "Error", "{reply}");
    let record = harness.records(RunEnd::Settled).await.remove(0);
    assert_eq!(record.hook, Some(before_record(BeforeHookResult::Failed)));
    let failures = fake.take_failures();
    assert_eq!(failures.len(), 1, "{failures:?}");
}
