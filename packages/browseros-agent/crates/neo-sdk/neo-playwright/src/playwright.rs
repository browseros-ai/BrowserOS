//! The crate's facade: one value per browser connection that runs every operation.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use browseros_core::{BrowserSession, TabId, TargetId};
use neo_trace::Effect;
use serde_json::Value;
use tokio::time;

use crate::action::action_op;
use crate::events::tab_frames::TabFrameList;
use crate::events::{
    BINDING_CALLED, EventFence, EventHub, EventScope, Fence, TargetWatcher, WatcherHandle,
    WatcherProgress, event_op,
};
use crate::expect::expect_op;
use crate::input::input_op;
use crate::op::{Op, OpKind};
use crate::op_input::OpInput;
use crate::op_scope::OpScope;
use crate::page::{MediaSessions, PageRegistry, RemovalFences, context_op, page_op};
use crate::private_entry::PrivateTabs;
use crate::read::read_op;
use crate::snapshot::{SnapshotBaselines, snapshot_op};
use crate::target::CallHeader;
use crate::target_crashes::TargetCrashes;
use crate::world::{Epoch, FrameAttachLog};
use crate::{OpEnv, PlaywrightError, TimedOut, locator_handler};

/// How long an op that ends may wait for its events to reach the run's listeners first
/// ([`Playwright::events_before_reply`]). Both steps are usually immediate; this only bounds a
/// watcher or a pump that fell far behind.
const EVENTS_BEFORE_REPLY: Duration = Duration::from_secs(2);

/// How long a `waitForFunction` that timed out waits for its polls' events to reach the run's
/// listeners first (WA-005). Upstream's wait awaits its poller's `cancel` before it throws, and its
/// protocol messages arrive in order, so a poll that logged before the timeout has its console
/// event delivered before the error. Short: the caller's timeout has already passed, and the
/// fence is usually immediate.
const POLLED_EVENTS_BEFORE_TIMEOUT: Duration = Duration::from_millis(500);

/// One per browser connection; runs every Playwright operation. The host builds it once at
/// startup (claw-sdk: next to its browser link) and calls [`Playwright::invoke`] from each
/// registered method's handler.
pub struct Playwright {
    /// The unbound session behind every run's view: frame sessions, raw protocol sessions and
    /// CDP events. Each op still admits its page through its own run's view first.
    browser: Arc<BrowserSession>,
    /// Page events since this facade started; the watcher below is its only writer.
    events: Arc<EventHub>,
    /// Records CDP's page events into `events`. Dropping the facade stops it.
    watcher: TargetWatcher,
    /// How far the watcher has read the connection's events.
    progress: Arc<WatcherProgress>,
    /// This instance's identity to the worlds it boots.
    epoch: Epoch,
    /// Every tab the instance acted on.
    pages: Arc<PageRegistry>,
    /// Replays page-owned media into newly attached iframe sessions.
    media: MediaSessions,
    /// Each tab's last whole-page v1 snapshot text its caller was given, for
    /// `ariaSnapshot({ delta: true })`. It outlives runs on purpose; the watcher drops a tab's
    /// when its document changes.
    snapshot_baselines: Arc<SnapshotBaselines>,
    /// The tabs a private entry typed into, for the host's camera.
    private_tabs: PrivateTabs,
    /// Admission facts survive individual runs and eviction of public crash events.
    crashes: Arc<TargetCrashes>,
    /// The tabs with an ended run's init-script removal still in flight, which every op's
    /// admission on such a tab waits out. Here, not in a run's state, because the next run's op
    /// is the one that waits.
    removals: Arc<RemovalFences>,
}

// A hand-written `Debug`: the session is not worth printing.
impl fmt::Debug for Playwright {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Playwright").finish_non_exhaustive()
    }
}

impl Playwright {
    /// The facade over `browser`. Starts the watcher that records page events (popups,
    /// dialogs, closes) for the instance's lifetime, and one read of the open tabs, which decides
    /// where an already-open tab's seqs start. Needs a tokio runtime, as `BrowserSession` does.
    #[must_use]
    pub fn new(browser: Arc<BrowserSession>) -> Arc<Self> {
        let (events, writer) = EventHub::new();
        let attach_log = Arc::new(FrameAttachLog::default());
        let progress = Arc::new(WatcherProgress::default());
        let snapshot_baselines = Arc::new(SnapshotBaselines::default());
        let crashes = Arc::new(TargetCrashes::observed(Arc::clone(&progress)));
        let watcher = TargetWatcher::spawn(
            Arc::clone(&browser),
            writer,
            Arc::clone(&attach_log),
            Arc::clone(&progress),
            Arc::clone(&snapshot_baselines),
            Arc::clone(&crashes),
        );
        let pages = Arc::new(PageRegistry::new(attach_log));
        let (registry, reader) = (Arc::clone(&pages), Arc::clone(&browser));
        drop(tokio::spawn(async move {
            registry.note_open_tabs(&reader).await;
        }));
        let media = MediaSessions::new(&browser);
        let removals = Arc::new(RemovalFences::new(Arc::clone(&crashes)));
        Arc::new(Self {
            browser,
            events,
            watcher,
            progress,
            epoch: Epoch::random(),
            pages,
            media,
            snapshot_baselines,
            private_tabs: PrivateTabs::default(),
            crashes,
            removals,
        })
    }

    /// The page event log and the dialogs held for runs.
    pub(crate) fn events(&self) -> &EventHub {
        &self.events
    }

    /// The page event log, shared, for a run that keeps it past one op (its bindings' later
    /// adds, `page/init_script.rs`).
    pub(crate) fn event_hub(&self) -> &Arc<EventHub> {
        &self.events
    }

    /// Live dedicated workers owned by a native page target, for ordered host page facts.
    #[must_use]
    pub fn workers(&self, target: &TargetId) -> Vec<crate::WorkerInfo> {
        self.events.workers.list(target)
    }

    /// Every CDP event received until now, as a fence for [`Playwright::synchronize_events`].
    /// A host takes it when an op's last reply is in, before it queues for its turn to
    /// observe, so what it waits for is fixed then and not stretched by what arrives later.
    #[must_use]
    pub fn event_fence(&self) -> EventFence {
        self.watcher.fence()
    }

    /// Brings the independent event watcher up to `fence` (every event received before it was
    /// taken), with none of `scope`'s events held back, and the leases in line with who listens,
    /// waiting at most 2 s. Whether the watcher answered in time: `false` means it is still
    /// behind `fence` for `scope`.
    pub async fn synchronize_events(&self, fence: EventFence, scope: &EventScope) -> bool {
        self.watcher.synchronize(None, fence, scope.clone()).await
    }

    /// The event scope of `tabs`: their current targets, so a wait covers their held events and
    /// not another tab's ([`EventScope::Tabs`]). A tab core's registry does not list, not yet (a
    /// popup whose line the watcher holds before core has it) or no longer, has no target to wait
    /// for and is left out: its held events do not hold this wait, and the next reply that reads
    /// it catches up with them.
    pub async fn tab_scope(&self, tabs: &[TabId]) -> EventScope {
        let mut targets = Vec::with_capacity(tabs.len());
        for &tab in tabs {
            if let Some(info) = self.browser.pages.get_info_by_tab(tab).await {
                targets.push(info.target_id);
            }
        }
        EventScope::Tabs(targets)
    }

    /// After an op that ended with no answer from its tab's renderer: pings the tab's source
    /// sessions (`TargetWatcher::order`), then takes the fence and waits for the tab's events
    /// before it, so every frame event the tab sent before the ping's answer is in its list when
    /// this returns, held ones included. Only that tab. Whether both the ping and the wait
    /// answered in time.
    pub(crate) async fn order_tab(&self, target: TargetId) -> bool {
        let ordered = self.watcher.order(target.clone()).await;
        // Taken after the ping's answer: an event the tab sent before that answer is before it.
        let fence = self.event_fence();
        let synchronized = self
            .synchronize_events(fence, &EventScope::Tabs(vec![target]))
            .await;
        ordered && synchronized
    }

    /// Asks the watcher to read again each session of `target`'s tab whose tree has not landed,
    /// and the tab's own tree whatever landed when `own_tree`, without waiting for it
    /// (`TargetWatcher::reseed`).
    pub(crate) fn reseed_frames(&self, target: TargetId, own_tree: bool) {
        self.watcher.reseed(target, own_tree);
    }

    /// The host stopped keeping `target`'s tab for itself (`OpEnv::hides_tab` reads false
    /// for it from now on, unless the run's own scope hides it): the owner browsed in it, at
    /// `url`. Scripts hear of it as of a tab
    /// the owner just opened, a context's `page` event, after every event received until now.
    /// Whether the watcher answered within 2 s.
    pub async fn hand_back(&self, target: TargetId, url: String) -> bool {
        self.watcher.hand_back(target, url).await
    }

    /// Whether every event before `fence` is in the log for `scope`, without waiting: read by
    /// the watcher and none of `scope`'s held back in a tab's line.
    #[must_use]
    pub fn events_handled_through(&self, fence: EventFence, scope: &EventScope) -> bool {
        self.progress.applied_through(scope) >= fence.0
    }

    /// A handle on the event watcher for what outlives an op (`events::log_lease`).
    pub(crate) fn watcher(&self) -> WatcherHandle {
        self.watcher.handle()
    }

    /// Retire workers after a successful explicit page close; their outstanding promises
    /// must reject before the native renderer's eventual protocol shutdown.
    pub(crate) async fn workers_closed(&self, target: TargetId) {
        self.crashes.confirmed_closed(&target);
        let scope = EventScope::Tabs(vec![target.clone()]);
        self.watcher
            .synchronize(Some(target), self.watcher.fence(), scope)
            .await;
    }

    /// The newest event recorded for this browser connection.
    #[must_use]
    pub fn event_head(&self) -> u64 {
        self.events.head()
    }

    /// The newest retained close of a native target, for the host's cache-liveness check.
    #[must_use]
    pub fn closed_target(&self, target: &TargetId) -> Option<u64> {
        self.events.closed_at(target)
    }

    /// The newest retained main-frame navigation of a native target, for the host's URL facts.
    #[must_use]
    pub fn navigated_target(&self, target: &TargetId) -> Option<u64> {
        self.events.navigated_at(target)
    }

    /// Every tab's frame-tree version, for every reply: a run learns that a tab's frames changed
    /// even from a reply about another tab.
    #[must_use]
    pub fn frame_versions(&self) -> Vec<(TabId, u64)> {
        self.events.frame_versions()
    }

    /// A tab's frames as the target watcher's events and seeds left them, main frame first,
    /// with the tab's frame version, whether its seed landed and the newest event-log seq the
    /// list includes; `None` for a target the watcher keeps nothing for. Nothing in the product
    /// reads it yet: the comparison row checks it against the browser's own tree.
    #[must_use]
    pub fn tab_frames(&self, target: &TargetId) -> Option<TabFrameList> {
        self.events.tab_frames(target)
    }

    /// The browser connection, for what an op does after it admitted its page: raw sessions,
    /// frame sessions, core's frame registry and events.
    pub(crate) fn browser(&self) -> &BrowserSession {
        &self.browser
    }

    /// This instance's epoch.
    pub(crate) fn epoch(&self) -> &Epoch {
        &self.epoch
    }

    /// Session replay for preferences owned by a tab, independent of any one run.
    pub(crate) fn media(&self) -> &MediaSessions {
        &self.media
    }

    /// The tabs a private entry typed into.
    pub(crate) fn private_tabs(&self) -> &PrivateTabs {
        &self.private_tabs
    }

    /// Every tab the instance acted on.
    pub(crate) fn pages(&self) -> &PageRegistry {
        &self.pages
    }

    pub(crate) fn crashes(&self) -> &TargetCrashes {
        &self.crashes
    }

    pub(crate) fn removal_fences(&self) -> &Arc<RemovalFences> {
        &self.removals
    }

    /// Each tab's last whole-page v1 snapshot text its caller was given; shared with the runs'
    /// pending texts, which commit to it when their runs end.
    pub(crate) fn snapshot_baselines(&self) -> &Arc<SnapshotBaselines> {
        &self.snapshot_baselines
    }

    /// Lets a host refuse remembered renderer crashes before its own native-tab reads.
    /// Call only for renderer operations, after host policy. Unknown targets perform no CDP
    /// work and pass through, so the host must still run every ordinary guard afterwards.
    /// `api` preserves the caller's method name when several public methods share one op.
    pub async fn check_cached_crash(
        &self,
        env: &dyn OpEnv,
        op: Op,
        tab: TabId,
        api: Option<&str>,
    ) -> Result<(), PlaywrightError> {
        if let Some(info) = self.browser.pages.get_info_by_tab(tab).await {
            let call = CallHeader {
                api: api.map(str::to_owned),
                ..CallHeader::default()
            };
            OpScope::new(env, op, call)
                .tracking(&self.crashes, &self.browser, &self.removals)
                .check_cached_crash(&info)?;
        }
        Ok(())
    }

    /// Runs one operation. `input` is the JSON the prelude sent: its `call` header opens the
    /// op's scope (the API name for messages, the op's own Playwright timeout), and the rest
    /// is decoded once by the op's family. A bad input is `script / invalid_input`, with
    /// nothing sent.
    ///
    /// The op re-runs the host's policies and marks itself sent right before its first write
    /// to the page, and never otherwise. The host races the returned future with the run's
    /// cancel and deadline and may drop it at any await; whatever the op left in the page is
    /// cleaned up on that drop.
    pub async fn invoke(
        &self,
        env: &dyn OpEnv,
        op: Op,
        input: Value,
    ) -> Result<Value, PlaywrightError> {
        let unordered = ends_unordered(op, &input);
        let OpInput { call, args } = OpInput::decode(op, input)?;
        let scope =
            OpScope::new(env, op, call).tracking(&self.crashes, &self.browser, &self.removals);
        let answered = match op.kind() {
            OpKind::Action(op) => action_op::invoke(self, &scope, op, args).await,
            OpKind::Read(op) => read_op::invoke(self, &scope, op, args).await,
            OpKind::Snapshot(op) => snapshot_op::invoke(self, &scope, op, args).await,
            OpKind::Expect(op) => expect_op::invoke(self, &scope, op, args).await,
            OpKind::Input(op) => input_op::invoke(self, &scope, op, args).await,
            OpKind::Page(op) => page_op::invoke(self, &scope, op, args).await,
            OpKind::Context(op) => context_op::invoke(self, &scope, op, args).await,
            // A trigger is upstream's `locatorHandlerTriggered` event: listeners for what the
            // page did before it run first, as upstream's one ordered channel runs them.
            OpKind::LocatorHandler(op) => locator_handler::invoke(self, &scope, op, args).await,
            // The event ops are the pump and the waits themselves: none waits for the pump.
            OpKind::Event(op) => {
                return scope.settle(event_op::invoke(self, &scope, op, args).await);
            }
        };
        // The frame events such an op's effect caused may still be on their way: ping the tab's
        // sources, so the reply's fence covers them and its frame version and the next
        // `page.frames` see them. Only the tab the op acted on, never another.
        if unordered
            && answered.is_ok()
            && let Some(target) = scope.admitted_target()
        {
            let _ordered = self.order_tab(target).await;
        }
        // Never both waits and never neither: a handler that took the wait inside itself
        // (`OpScope::mark_ordered_inside`, a whole page's text in Neo's format, which must read
        // and record its baseline after the wait) is not held again here.
        if !scope.refused_crash_before_sending()
            && !scope.ordered_inside()
            && let Some(bound) = ordering_wait(op, &answered)
        {
            self.events_before_reply(&scope, bound).await;
        }
        scope.settle(answered)
    }

    /// Upstream handles protocol messages in order, so a `page.on('console')` listener has run
    /// for the message an evaluate logged before that evaluate resolves, and an exposed binding
    /// has been called for a call the page made, and none runs for it after. Neo's events and
    /// binding calls reach the script through the run's pump, beside the op, so an op that ends
    /// waits, within [`EVENTS_BEFORE_REPLY`], until
    ///
    /// ```text
    ///  the watcher handled every CDP event and binding call that came before
    ///  the op's last reply                                                       (fence)
    ///  ── the run's pump read every listener past the log's head then, and
    ///     took every binding call queued for the run                             (taken)
    /// ```
    ///
    /// That pump reply is then counted (`RunState::pump_replies`) before this op's reply leaves,
    /// whose envelope carries the count (`pumped`), and the prelude holds this op's result until
    /// it has dispatched that many pump replies. So the listeners run before the script continues
    /// past this op, whatever order the host's promises settle in (the Node add-on settles each
    /// on a threadsafe function of its own, in no fixed order). A run with no listener has
    /// nothing to order and does not wait. An op that timed out does not wait either, except a
    /// `waitForFunction`, whose polls ran page script up to its deadline ([`ordering_wait`]).
    ///
    /// `invoke` runs it after the handler. One handler runs it inside itself instead, at the
    /// point after its last browser reply: a whole page's text in Neo's format, whose baseline
    /// is read and recorded after the wait (`snapshot_op.rs`, `order_events_inside`). It is this
    /// one function in both places, so the two waits cannot drift apart.
    pub(crate) async fn events_before_reply(&self, scope: &OpScope<'_>, bound: Duration) {
        let run = scope.run_state();
        if !run.events.has_listeners() {
            return;
        }
        let fence = Fence {
            events: self.browser.cdp_events_broadcast(),
            calls: self.browser.cdp_events_targeted_delivered(BINDING_CALLED),
        };
        // Only the listened tabs' held events: the pump reads nothing of another tab's.
        let listened = run.events.listened_scope();
        let ordered = async {
            self.progress.reached(fence, &listened).await;
            run.events.taken_through(self.events.head()).await;
        };
        let _bounded = time::timeout(bound, ordered).await;
    }
}

/// Whether `op` can end with no answer from its tab's renderer after its effect, so the frame
/// events that effect caused may still be on their way when it answers: the raw device calls
/// (`page.mouse`, `page.keyboard`, `page.touchscreen`), which never read the page, and an action
/// that is forced or told not to wait after, which arms no interceptor or reads no verdict. Every
/// other action ends with its verdict, a renderer read in the element's own frame, which already
/// orders what came before it; a trial presses nothing.
fn ends_unordered(op: Op, input: &Value) -> bool {
    let flag = |name: &str| input.get(name).and_then(Value::as_bool) == Some(true);
    match op.kind() {
        OpKind::Input(_) => op.spec().effect == Effect::Write,
        OpKind::Action(_) => (flag("force") || flag("noWaitAfter")) && !flag("trial"),
        _ => false,
    }
}

/// How long an op that `answered` may wait for event ordering before its reply leaves, if at
/// all. Every op waits up to [`EVENTS_BEFORE_REPLY`], except one whose own timeout passed: the
/// caller's timeout is how long its call takes, so a timed-out op answers at once. Its listeners
/// still run, only maybe after the script has its error. A slow watcher held a timed-out 5 s
/// snapshot here for up to 2 s in the owner's run. An op that succeeds keeps the whole wait, even
/// near its deadline, so code after it never runs before the listeners for what it did.
///
/// A `waitForFunction` that timed out waits too, up to [`POLLED_EVENTS_BEFORE_TIMEOUT`]: its
/// predicate ran in the page at every poll up to the deadline, and a listener must hear what those
/// polls logged before the error, as upstream's does (WA-005). No poll runs after it: the wait
/// releases its poller before it answers (`page::evaluate`).
fn ordering_wait(op: Op, answered: &Result<Value, PlaywrightError>) -> Option<Duration> {
    // Crash admission must not wait behind the renderer events that caused refusal. The
    // independent pump still delivers them; successful operations keep their ordering fence.
    let timed_out = match answered {
        Err(error) => error.timed_out(),
        Ok(_) => TimedOut::No,
    };
    ordering_wait_for(op, timed_out)
}

/// [`ordering_wait`]'s rule by whether the op timed out: the one source of every event wait's
/// bound, the generic wait's after the handler and the snapshot handler's own inside it, which
/// asks with [`TimedOut::No`] since its tree is in hand.
pub(crate) fn ordering_wait_for(op: Op, timed_out: TimedOut) -> Option<Duration> {
    if timed_out == TimedOut::No {
        return Some(EVENTS_BEFORE_REPLY);
    }
    matches!(
        op.kind(),
        OpKind::Page(page_op::PageOp::WaitForFunction | page_op::PageOp::WaitForFunctionOnElement)
    )
    .then_some(POLLED_EVENTS_BEFORE_TIMEOUT)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::error::Error;
    use std::time::Duration;

    use neo_trace::ErrorOwner;
    use serde_json::{Value, json};
    use tokio::time;

    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    use browseros_core::{TabId, TargetId};

    use super::Playwright;
    use crate::events::EventScope;
    use crate::test_env::{self, TestEnv};
    use crate::test_support::{
        ProbeAnswer, SCRIPTED_MAIN, ScriptedRun, ScriptedTab, TabScript, TestResult,
    };
    use crate::{ErrorCode, Op, PlaywrightError, Refusal};

    /// The ops whose effect may leave frame events on their way when they answer are the ones the
    /// reply pings their tab for: the raw device calls, and an action forced or told not to wait
    /// after. An ordinary action ends with its verdict, a trial presses nothing, and a read
    /// changes nothing.
    #[test]
    fn tests_that_only_ops_that_end_without_a_renderer_answer_are_ordered()
    -> Result<(), Box<dyn Error>> {
        let ends = |name: &str, input: Value| -> Result<bool, Box<dyn Error>> {
            let op = Op::from_name(name).ok_or(format!("{name} is an op"))?;
            Ok(super::ends_unordered(op, &input))
        };
        assert!(ends("mouse.click", json!({}))?);
        assert!(ends("keyboard.press", json!({}))?);
        assert!(ends("locator.click", json!({ "force": true }))?);
        assert!(ends("locator.press", json!({ "noWaitAfter": true }))?);
        assert!(!ends("locator.click", json!({}))?, "its verdict orders it");
        assert!(!ends(
            "locator.click",
            json!({ "force": true, "trial": true })
        )?);
        assert!(!ends("page.title", json!({}))?, "a read");
        Ok(())
    }

    struct NoProbes;
    impl TabScript for NoProbes {
        fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
            ProbeAnswer::unexpected()
        }
    }

    /// `page.frames` answers from the watcher's list and sends the tab nothing of its own: no
    /// attach and no tree read, so a tab whose renderer is busy answers as fast as a healthy one.
    #[tokio::test]
    async fn tests_that_page_frames_sends_the_tab_nothing() -> TestResult {
        let tab = ScriptedTab::new(NoProbes);
        let run = ScriptedRun::over(&tab).await?;
        let playwright = &run.playwright;
        assert!(
            playwright
                .synchronize_events(playwright.event_fence(), &EventScope::Every)
                .await
        );
        // The watcher's seed of the tab goes out on a task of its own.
        time::sleep(Duration::from_millis(50)).await;
        let before = tab.sent_methods().len();
        let op = Op::from_name("page.frames").ok_or("page.frames is an op")?;
        let answer = playwright
            .invoke(&run.env, op, json!({ "call": {}, "page": run.page }))
            .await?;
        let sent = tab.sent_methods().split_off(before);
        assert!(
            !sent
                .iter()
                .any(|method| method.starts_with("Page.") || method == "Target.attachToTarget"),
            "the op asked the tab: {sent:?}"
        );
        assert_eq!(
            answer.pointer("/frames/0/id").and_then(Value::as_str),
            Some(SCRIPTED_MAIN),
            "the main frame, from the list: {answer}"
        );
        Ok(())
    }

    /// A tab whose tree reads fail while its renderer is busy.
    struct Unreadable {
        failing: AtomicBool,
    }
    impl TabScript for Unreadable {
        fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
            ProbeAnswer::unexpected()
        }
        fn cdp_error(&self, method: &str, _params: &Value) -> Option<&'static str> {
            (method == "Page.getFrameTree" && self.failing.load(Ordering::SeqCst)).then_some("busy")
        }
    }

    /// `page.frames` on a tab whose list is not whole answers at once, marked incomplete, and
    /// asks the watcher to read the tab again: once its renderer answers, a later `page.frames` is
    /// whole. The watcher's own retries are spent before the renderer answers, so only that ask
    /// reads the tab again.
    #[tokio::test]
    async fn tests_that_page_frames_on_a_list_not_whole_has_it_read_again() -> TestResult {
        let tab = ScriptedTab::new(Unreadable {
            failing: AtomicBool::new(true),
        });
        let run = ScriptedRun::over(&tab).await?;
        let playwright = &run.playwright;
        assert!(
            playwright
                .synchronize_events(playwright.event_fence(), &EventScope::Every)
                .await
        );
        // Past the watcher's retries of the failed seed (two, a tick apart) and the reseed's
        // backoff after the last of them.
        time::sleep(Duration::from_secs(2)).await;
        let op = Op::from_name("page.frames").ok_or("page.frames is an op")?;
        let frames = || playwright.invoke(&run.env, op, json!({ "call": {}, "page": run.page }));
        let whole =
            |answer: &Value| answer.get("incomplete").is_none() && answer.get("frames").is_some();
        let first = frames().await?;
        assert!(!whole(&first), "its seed failed: {first}");
        tab.script().failing.store(false, Ordering::SeqCst);
        let started = Instant::now();
        loop {
            let answer = frames().await?;
            if whole(&answer) {
                break;
            }
            if started.elapsed() > Duration::from_secs(2) {
                return Err(format!("never whole: {answer}").into());
            }
            time::sleep(Duration::from_millis(20)).await;
        }
        Ok(())
    }

    /// A raw mouse click ends with no answer from the tab's renderer, so before it answers it
    /// pings the tab's source: a `Page.getFrameTree` reaches the tab after the click's last input
    /// command.
    #[tokio::test]
    async fn tests_that_a_raw_click_pings_its_tab_before_it_answers() -> TestResult {
        let tab = ScriptedTab::new(NoProbes);
        let run = ScriptedRun::over(&tab).await?;
        let playwright = &run.playwright;
        assert!(
            playwright
                .synchronize_events(playwright.event_fence(), &EventScope::Every)
                .await
        );
        time::sleep(Duration::from_millis(50)).await;
        let before = tab.sent_methods().len();
        let op = Op::from_name("mouse.click").ok_or("mouse.click is an op")?;
        playwright
            .invoke(
                &run.env,
                op,
                json!({ "call": { "api": "mouse.click", "timeoutMs": 0 }, "page": run.page,
                        "x": 1, "y": 1 }),
            )
            .await?;
        let sent = tab.sent_methods().split_off(before);
        let last_input = sent
            .iter()
            .rposition(|method| method.starts_with("Input."))
            .ok_or(format!("the click sent no input: {sent:?}"))?;
        assert!(
            sent.iter()
                .skip(last_input)
                .any(|method| method == "Page.getFrameTree"),
            "no ping after the click's input: {sent:?}"
        );
        Ok(())
    }

    /// A tab whose tree reads fail, then answer only after a while.
    struct SlowTree {
        failing: AtomicBool,
        slow: AtomicBool,
    }
    impl TabScript for SlowTree {
        fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
            ProbeAnswer::unexpected()
        }
        fn cdp_error(&self, method: &str, _params: &Value) -> Option<&'static str> {
            (method == "Page.getFrameTree" && self.failing.load(Ordering::SeqCst)).then_some("busy")
        }
        fn delay(&self, method: &str, _params: &Value) -> Duration {
            if method == "Page.getFrameTree" && self.slow.load(Ordering::SeqCst) {
                Duration::from_millis(300)
            } else {
                Duration::ZERO
            }
        }
    }

    /// A frame event the tab sends after the op's last answer, while the ping is out, is in the
    /// tab's list once the ordering step returns, even while it waits in the tab's line behind a
    /// tree read: the fence is taken after the ping, so the wait on the tab covers it. The read
    /// (300 ms) outlasts the ping's budget, so a fence taken before the ping would let the step
    /// return with the event still held.
    #[tokio::test]
    async fn tests_that_a_frame_event_sent_during_the_ping_is_in_the_list_after_it() -> TestResult {
        let tab = ScriptedTab::new(SlowTree {
            failing: AtomicBool::new(true),
            slow: AtomicBool::new(false),
        });
        let run = ScriptedRun::over(&tab).await?;
        let playwright = &run.playwright;
        assert!(
            playwright
                .synchronize_events(playwright.event_fence(), &EventScope::Every)
                .await
        );
        // Past the watcher's retries of the failed seed (two, a tick apart) and the reseed's
        // backoff after the last of them.
        time::sleep(Duration::from_secs(2)).await;
        let target = TargetId(SCRIPTED_MAIN.to_owned());
        tab.script().failing.store(false, Ordering::SeqCst);
        tab.script().slow.store(true, Ordering::SeqCst);
        // A read of the tab goes out and holds its line for 300 ms.
        playwright.reseed_frames(target.clone(), false);
        time::sleep(Duration::from_millis(20)).await;
        let late = json!({ "frameId": "late", "parentFrameId": SCRIPTED_MAIN });
        let session = tab.session_for(SCRIPTED_MAIN);
        let emitted = async {
            time::sleep(Duration::from_millis(10)).await;
            tab.emit_on("Page.frameAttached", late, Some(session));
        };
        let ((), _ordered) = tokio::join!(emitted, playwright.order_tab(target.clone()));
        let listed = playwright.tab_frames(&target).ok_or("the tab has a list")?;
        assert!(
            listed.frames.iter().any(|frame| frame.id.0 == "late"),
            "the frame the tab announced during the ping: {:?}",
            listed.frames
        );
        Ok(())
    }

    /// A reply's wait names the tabs it reads by their current targets, so it covers their held
    /// events and no other tab's; a tab the browser does not list has no target to wait for.
    #[tokio::test]
    async fn tests_that_a_tab_scope_names_each_listed_tabs_target() -> TestResult {
        let tab = ScriptedTab::new(NoProbes);
        let run = ScriptedRun::over(&tab).await?;
        assert_eq!(
            run.playwright.tab_scope(&[TabId(7), TabId(404)]).await,
            EventScope::Tabs(vec![TargetId(SCRIPTED_MAIN.to_owned())])
        );
        Ok(())
    }

    /// An op either answers `not built yet` or decodes its input before it reaches the host, so
    /// an empty call names the missing field and nothing is sent or noted. The ops whose
    /// arguments are all optional are left out: an empty call is a whole call for them.
    #[tokio::test]
    async fn tests_that_an_empty_call_fails_before_it_reaches_the_host()
    -> Result<(), Box<dyn Error>> {
        const WHOLE_WITHOUT_ARGUMENTS: [&str; 4] = [
            "context.pages",
            "context.newPage",
            "events.next",
            "page.locatorHandlerTriggered",
        ];
        let playwright = Playwright::new(test_env::offline_session());
        let env = TestEnv::new();
        let ops = Op::all().filter(|op| !WHOLE_WITHOUT_ARGUMENTS.contains(&op.spec().name));
        for op in ops {
            let name = op.spec().name;
            let error = playwright
                .invoke(&env, op, json!({ "call": {} }))
                .await
                .err()
                .ok_or(format!("{name} needs arguments"))?;
            match error.error_code() {
                Some(ErrorCode::Internal) => {
                    assert_eq!(error.message(), format!("{name}: not built yet"));
                }
                Some(ErrorCode::InvalidInput) => assert!(
                    error
                        .message()
                        .starts_with(&format!("{name}: missing field")),
                    "{error}"
                ),
                other => panic!("{name}: {other:?}: {error}"),
            }
        }
        assert!(
            env.calls().is_empty(),
            "a call that does not decode neither sends nor notes"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_bad_header_never_reaches_a_family() -> Result<(), Box<dyn Error>> {
        let playwright = Playwright::new(test_env::offline_session());
        let env = TestEnv::new();
        let op = Op::from_name("locator.click").ok_or("locator.click is an op")?;
        let error = playwright
            .invoke(&env, op, json!({ "call": { "timeoutMs": "5s" } }))
            .await
            .err()
            .ok_or("a string timeout must fail")?;
        assert_eq!(error.error_code(), Some(ErrorCode::InvalidInput));
        assert!(
            error
                .message()
                .starts_with("locator.click: call.timeoutMs: "),
            "{error}"
        );
        Ok(())
    }

    /// Every op input the prelude sends, recorded from the built prelude by
    /// crates/neo-sdk/neo-playwright-js/scripts/wire-examples.ts; its build check fails when the
    /// file is stale.
    const WIRE_EXAMPLES: &str = include_str!("../tests/data/wire-examples.json");

    #[test]
    fn tests_that_only_an_op_that_timed_out_skips_the_ordering_wait() -> Result<(), String> {
        use super::{
            EVENTS_BEFORE_REPLY, POLLED_EVENTS_BEFORE_TIMEOUT, ordering_wait, ordering_wait_for,
        };
        use crate::TimedOut;
        let op = |name: &str| Op::from_name(name).ok_or(format!("no op {name}"));
        let timed_out = Err(PlaywrightError::new(
            ErrorCode::WaitTimeout,
            "page.ariaSnapshot: Timeout 2000ms exceeded.".to_owned(),
        ));
        assert_eq!(
            ordering_wait(op("page.ariaSnapshot")?, &timed_out),
            None,
            "a timed-out op answers at its timeout"
        );
        for wait in ["page.waitForFunction", "locator.waitForFunction"] {
            assert_eq!(
                ordering_wait(op(wait)?, &timed_out),
                Some(POLLED_EVENTS_BEFORE_TIMEOUT),
                "{wait}: its polls' events come before its timeout (WA-005)"
            );
        }
        assert_eq!(
            ordering_wait(op("page.ariaSnapshot")?, &Ok(json!(null))),
            Some(EVENTS_BEFORE_REPLY),
            "a successful op keeps its listeners ahead of the script, even near its deadline"
        );
        // The rule by itself, as the snapshot handler asks it for the wait inside it: the tree is
        // in hand, so not timed out. `waitForFunction`'s timed-out wait is unchanged by that.
        assert_eq!(
            ordering_wait_for(op("page.ariaSnapshot")?, TimedOut::No),
            Some(EVENTS_BEFORE_REPLY)
        );
        for wait in ["page.waitForFunction", "locator.waitForFunction"] {
            assert_eq!(
                ordering_wait_for(op(wait)?, TimedOut::Yes),
                Some(POLLED_EVENTS_BEFORE_TIMEOUT),
                "{wait}"
            );
        }
        let refused = Err(PlaywrightError::new(
            ErrorCode::ElementDetached,
            "e17 is not on the page any more".to_owned(),
        ));
        assert_eq!(
            ordering_wait(op("locator.click")?, &refused),
            Some(EVENTS_BEFORE_REPLY),
            "any other failure still orders its events"
        );
        Ok(())
    }

    /// Ops whose recorded input Rust refuses today. Each commit that makes one agree deletes
    /// its line, and the test fails while a fixed op is still listed.
    const DRIFT: &[&str] = &[];

    /// What the prelude sends decodes here. Offline, a built op that decodes fails at its page's
    /// admission or waits past the test's clock. A recorded `refused` script can intentionally
    /// answer `invalid_input`: only its exact message on that recorded op/example qualifies,
    /// and a poisoned copy must still fail schema validation. All other `invalid_input`
    /// replies remain decode drift, so a field renamed on one side alone fails this test.
    #[tokio::test(start_paused = true)]
    async fn tests_that_every_prelude_example_decodes() -> Result<(), Box<dyn Error>> {
        let examples: Value = serde_json::from_str(WIRE_EXAMPLES)?;
        let ops = examples
            .get("ops")
            .and_then(Value::as_object)
            .ok_or("the examples are keyed by op")?;
        let playwright = Playwright::new(test_env::offline_session());
        let mut refused = BTreeSet::new();
        let mut messages = Vec::new();
        for (name, inputs) in ops {
            let op = Op::from_name(name).ok_or(format!("{name} is not an op"))?;
            for input in inputs.as_array().ok_or("each op lists its inputs")? {
                // Decode the filesystem step, then refuse its first effect. Recorded paths
                // are examples, not destinations this offline contract test owns.
                let env = if name == "page.saveScreenshot" {
                    TestEnv::refusing(Refusal {
                        owner: ErrorOwner::Host,
                        code: "fixture_write_refused".to_owned(),
                        message: "the wire decoder does not write fixture files".to_owned(),
                        host_code: None,
                    })
                } else {
                    TestEnv::new()
                };
                let invoked = playwright.invoke(&env, op, input.clone());
                if let Ok(Err(error)) = time::timeout(Duration::from_secs(60), invoked).await
                    && error.error_code() == Some(ErrorCode::InvalidInput)
                {
                    let expected_refusal =
                        input.get("refused").and_then(Value::as_str).map(|text| {
                            // OpScope decorates every leaf error with the recorded API name.
                            // Compare the complete message, including that exact decoration.
                            let api = input
                                .get("call")
                                .and_then(|call| call.get("api"))
                                .and_then(Value::as_str)
                                .unwrap_or(name);
                            format!("{api}: {text}")
                        });
                    if expected_refusal.as_deref() == Some(error.message()) {
                        // `op` and this exact refused text belong to the same recorded input
                        // just invoked above. No op-wide exemption or message prefix matches.
                        const UNKNOWN: &str = "__wire_example_unknown_field";
                        let mut poisoned = input.clone();
                        poisoned
                            .as_object_mut()
                            .ok_or("a refused example is an object")?
                            .insert(UNKNOWN.to_owned(), Value::Bool(true));
                        let rejected = time::timeout(
                            Duration::from_secs(60),
                            playwright.invoke(&env, op, poisoned),
                        )
                        .await?
                        .err()
                        .ok_or("the poisoned refused example must be rejected")?;
                        assert_eq!(
                            rejected.error_code(),
                            Some(ErrorCode::InvalidInput),
                            "{name}: an unknown field must fail input decoding"
                        );
                        assert!(
                            rejected.message().contains("unknown field")
                                && rejected.message().contains(UNKNOWN),
                            "{name}: the poisoned copy must fail its schema, not replay its refusal: {}",
                            rejected.message()
                        );
                        continue;
                    }
                    refused.insert(name.as_str());
                    messages.push(error.message().to_owned());
                }
            }
        }
        let drift: BTreeSet<&str> = DRIFT.iter().copied().collect();
        assert_eq!(
            refused, drift,
            "the ops that refuse the prelude's input are exactly DRIFT: {messages:#?}"
        );
        Ok(())
    }
}
