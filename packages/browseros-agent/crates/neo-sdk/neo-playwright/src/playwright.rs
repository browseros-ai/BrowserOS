//! The crate's facade: one value per browser connection that runs every operation.

use std::fmt;
use std::sync::Arc;

use browseros_core::{BrowserSession, TargetId};
use serde_json::Value;

use crate::action::action_op;
use crate::events::{EventHub, TargetWatcher, event_op};
use crate::expect::expect_op;
use crate::input::input_op;
use crate::op::{Op, OpKind};
use crate::op_input::OpInput;
use crate::op_scope::OpScope;
use crate::page::{PageRegistry, context_op, page_op};
use crate::read::read_op;
use crate::snapshot::snapshot_op;
use crate::world::{Epoch, FrameAttachLog, SessionSetup};
use crate::{OpEnv, PlaywrightError, PlaywrightHooks, locator_handler};

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
    _watcher: TargetWatcher,
    /// This instance's identity to the worlds it boots.
    epoch: Epoch,
    /// Per-session setup for isolated world installs.
    sessions: Arc<SessionSetup>,
    /// Every tab the instance acted on.
    pages: Arc<PageRegistry>,
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
    /// where an already-open tab's seqs start. `hooks` is retained for host implementations; no
    /// op calls it. Needs a tokio runtime, as `BrowserSession` does.
    #[must_use]
    pub fn new(browser: Arc<BrowserSession>, hooks: Arc<dyn PlaywrightHooks>) -> Arc<Self> {
        drop(hooks);
        let (events, writer) = EventHub::new();
        let attach_log = Arc::new(FrameAttachLog::default());
        let watcher = TargetWatcher::spawn(Arc::clone(&browser), writer, Arc::clone(&attach_log));
        let sessions = Arc::new(SessionSetup::new(browser.connection()));
        let pages = Arc::new(PageRegistry::new(attach_log));
        let (registry, reader) = (Arc::clone(&pages), Arc::clone(&browser));
        drop(tokio::spawn(async move {
            registry.note_open_tabs(&reader).await;
        }));
        Arc::new(Self {
            browser,
            events,
            _watcher: watcher,
            epoch: Epoch::random(),
            sessions,
            pages,
        })
    }

    /// The page event log and the dialogs held for runs.
    pub(crate) fn events(&self) -> &EventHub {
        &self.events
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

    /// Per-session setup for sealed worlds.
    pub(crate) fn sessions(&self) -> &SessionSetup {
        &self.sessions
    }

    /// Every tab the instance acted on.
    pub(crate) fn pages(&self) -> &PageRegistry {
        &self.pages
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
            OpKind::Event(op) => event_op::invoke(self, &scope, op, args).await,
            OpKind::LocatorHandler(op) => locator_handler::invoke(&scope, op, args).await,
        };
        scope.settle(answered)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::error::Error;
    use std::sync::Arc;
    use std::time::Duration;

    use browseros_core::TabId;
    use browseros_core::sensitive_fields::{FieldValues, JournalEntry};
    use serde_json::{Value, json};
    use tokio::time;

    use super::Playwright;
    use crate::test_env::{self, TestEnv};
    use crate::{ErrorCode, Op, PlaywrightHooks, Refusal, TypedBy};

    struct NoHooks;

    impl PlaywrightHooks for NoHooks {
        fn field_entered(&self, _page: TabId, _entry: JournalEntry, _by: TypedBy) {}
        fn values_swept(&self, _page: TabId, _values: &FieldValues) {}
        fn capture_allowed(&self, _page: TabId) -> Result<(), Refusal> {
            Ok(())
        }
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
        let playwright = Playwright::new(test_env::offline_session(), Arc::new(NoHooks));
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
        let playwright = Playwright::new(test_env::offline_session(), Arc::new(NoHooks));
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

    /// Ops whose recorded input Rust refuses today. Each commit that makes one agree deletes
    /// its line, and the test fails while a fixed op is still listed.
    const DRIFT: &[&str] = &[];

    /// What the prelude sends decodes here. Offline, a built op that decodes fails at its page's
    /// admission or waits past the test's clock, an unbuilt one answers `not built yet`, and
    /// only an input Rust cannot decode is `script / invalid_input`. So a field renamed in
    /// wire.ts or in a family's input alone fails this test.
    #[tokio::test(start_paused = true)]
    async fn tests_that_every_prelude_example_decodes() -> Result<(), Box<dyn Error>> {
        let examples: Value = serde_json::from_str(WIRE_EXAMPLES)?;
        let ops = examples
            .get("ops")
            .and_then(Value::as_object)
            .ok_or("the examples are keyed by op")?;
        let playwright = Playwright::new(test_env::offline_session(), Arc::new(NoHooks));
        let mut refused = BTreeSet::new();
        let mut messages = Vec::new();
        for (name, inputs) in ops {
            let op = Op::from_name(name).ok_or(format!("{name} is not an op"))?;
            for input in inputs.as_array().ok_or("each op lists its inputs")? {
                let env = TestEnv::new();
                let invoked = playwright.invoke(&env, op, input.clone());
                if let Ok(Err(error)) = time::timeout(Duration::from_secs(60), invoked).await
                    && error.error_code() == Some(ErrorCode::InvalidInput)
                {
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
