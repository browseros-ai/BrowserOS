//! The crate's facade: one value per browser connection that runs every operation.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use browseros_core::{BrowserSession, TargetId};
use serde_json::Value;
use tokio::time;

use crate::action::action_op;
use crate::events::{
    BINDING_CALLED, EventFence, EventHub, Fence, TargetWatcher, WatcherHandle, WatcherProgress,
    event_op,
};
use crate::expect::expect_op;
use crate::input::input_op;
use crate::op::{Op, OpKind};
use crate::op_input::OpInput;
use crate::op_scope::OpScope;
use crate::page::{MediaSessions, PageRegistry, context_op, page_op};
use crate::private_entry::PrivateTabs;
use crate::read::read_op;
use crate::snapshot::{SnapshotBaselines, snapshot_op};
use crate::world::{Epoch, FrameAttachLog};
use crate::{OpEnv, PlaywrightError, TimedOut, locator_handler};

/// How long an op that ends may wait for its events to reach the run's listeners first
/// ([`Playwright::events_before_reply`]). Both steps are usually immediate; this only bounds a
/// watcher or a pump that fell far behind.
const EVENTS_BEFORE_REPLY: Duration = Duration::from_secs(2);

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
        let watcher = TargetWatcher::spawn(
            Arc::clone(&browser),
            writer,
            Arc::clone(&attach_log),
            Arc::clone(&progress),
            Arc::clone(&snapshot_baselines),
        );
        let pages = Arc::new(PageRegistry::new(attach_log));
        let (registry, reader) = (Arc::clone(&pages), Arc::clone(&browser));
        drop(tokio::spawn(async move {
            registry.note_open_tabs(&reader).await;
        }));
        let media = MediaSessions::new(&browser);
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
    /// taken) and the leases in line with who listens, waiting at most 2 s. Whether the watcher
    /// answered in time: `false` means it is still behind `fence`.
    pub async fn synchronize_events(&self, fence: EventFence) -> bool {
        self.watcher.synchronize(None, fence).await
    }

    /// The host stopped keeping `target`'s tab for itself (`OpEnv::is_reserved_tab` reads false
    /// for it from now on): the owner browsed in it, at `url`. Scripts hear of it as of a tab
    /// the owner just opened, a context's `page` event, after every event received until now.
    /// Whether the watcher answered within 2 s.
    pub async fn hand_back(&self, target: TargetId, url: String) -> bool {
        self.watcher.hand_back(target, url).await
    }

    /// Whether every event before `fence` is in the log, without waiting: read by the watcher
    /// and none of them held back in a tab's line.
    #[must_use]
    pub fn events_handled_through(&self, fence: EventFence) -> bool {
        self.progress.applied_through() >= fence.0
    }

    /// Read-only topology checks must also reject a stopped watcher; its completed waiters
    /// otherwise look like current observation. This never synchronizes or enables domains.
    pub(crate) fn events_live_through(&self, fence: EventFence) -> bool {
        self.progress.live_through(fence.0)
    }

    /// A handle on the event watcher for what outlives an op (`events::log_lease`).
    pub(crate) fn watcher(&self) -> WatcherHandle {
        self.watcher.handle()
    }

    /// Retire workers after a successful explicit page close; their outstanding promises
    /// must reject before the native renderer's eventual protocol shutdown.
    pub(crate) async fn workers_closed(&self, target: TargetId) {
        self.watcher
            .synchronize(Some(target), self.watcher.fence())
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

    /// Each tab's last whole-page v1 snapshot text its caller was given; shared with the runs'
    /// pending texts, which commit to it when their runs end.
    pub(crate) fn snapshot_baselines(&self) -> &Arc<SnapshotBaselines> {
        &self.snapshot_baselines
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
        let OpInput { call, args } = OpInput::decode(op, input)?;
        let scope = OpScope::new(env, op, call);
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
        if waits_for_ordering(&answered) {
            self.events_before_reply(&scope).await;
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
    /// nothing to order and does not wait. An op that timed out does not wait either
    /// ([`waits_for_ordering`]).
    async fn events_before_reply(&self, scope: &OpScope<'_>) {
        let run = scope.run_state();
        if !run.events.has_listeners() {
            return;
        }
        let fence = Fence {
            events: self.browser.cdp_events_broadcast(),
            calls: self.browser.cdp_events_targeted_delivered(BINDING_CALLED),
        };
        let ordered = async {
            self.progress.reached(fence).await;
            run.events.taken_through(self.events.head()).await;
        };
        let _bounded = time::timeout(EVENTS_BEFORE_REPLY, ordered).await;
    }
}

/// Whether an op that `answered` waits for event ordering before its reply leaves. Every op
/// does, except one whose own timeout passed: the caller's timeout is how long its call takes,
/// so a timed-out op answers at once. Its listeners still run, only maybe after the script has
/// its error. A slow watcher held a timed-out 5 s snapshot here for up to 2 s in the owner's
/// run. An op that succeeds keeps the whole wait, even near its deadline, so code after it never
/// runs before the listeners for what it did.
fn waits_for_ordering(answered: &Result<Value, PlaywrightError>) -> bool {
    !matches!(answered, Err(error) if error.timed_out() == TimedOut::Yes)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::error::Error;
    use std::time::Duration;

    use neo_trace::ErrorOwner;
    use serde_json::{Value, json};
    use tokio::time;

    use super::Playwright;
    use crate::test_env::{self, TestEnv};
    use crate::{ErrorCode, Op, PlaywrightError, Refusal};

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
    fn tests_that_only_an_op_that_timed_out_skips_the_ordering_wait() {
        let timed_out = Err(PlaywrightError::new(
            ErrorCode::WaitTimeout,
            "page.ariaSnapshot: Timeout 2000ms exceeded.".to_owned(),
        ));
        assert!(
            !super::waits_for_ordering(&timed_out),
            "a timed-out op answers at its timeout"
        );
        assert!(
            super::waits_for_ordering(&Ok(json!(null))),
            "a successful op keeps its listeners ahead of the script, even near its deadline"
        );
        let refused = Err(PlaywrightError::new(
            ErrorCode::ElementDetached,
            "e17 is not on the page any more".to_owned(),
        ));
        assert!(
            super::waits_for_ordering(&refused),
            "any other failure still orders its events"
        );
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
