//! Neo: the shared boundary between action policy and a browser tab.
//! `FrameTab` implements these cancel-safe steps over CDP; the test adapter scripts the same
//! steps. Input turns, interceptors and delivery reports retain ownership until their work ends.
//! This seam and the input bridge exchange typed plans and reports; neither owns pointer policy.

use std::future::Future;

use neo_runtime_spec::injected::{ElementState, HitTargetAction, Point};

use crate::chromium_bridge::mouse_and_touch_input::{Devices, Plan};
use crate::progress::{AckedInput, CallError, InputTicket, PlaywrightError, Progress};

/// What element-action policy needs from its target tab. Each method is one Playwright step:
/// one page call, one CDP command, or one wait. Every returned future is cancel-safe: dropping
/// it in [`Progress::race`] loses only the answer. `FrameTab` implements the browser adapter;
/// the tests use the same seam through `FakeTab`.
pub(crate) trait Tab: Send + Sync {
    /// One resolved element, held for one loop-A iteration. Dropping it releases it (the real
    /// one owns a CDP object group and releases it on the run's tracker).
    type Element: Send + Sync;
    /// An armed hit-target interceptor. Dropping one without a successful [`Tab::verdict`]
    /// disarms it in the background, so every path out of an attempt disarms it: a leaked
    /// armed interceptor would keep swallowing mismatched clicks in the user's tab, because the
    /// injected script's slot outlives the run.
    ///
    /// It has one owner at a time, from the setup's send to the stop's answer: [`Tab::arm`]'s
    /// task until a waiting caller takes it, the attempt until it presses, the writer until
    /// the last mouse event is acknowledged ([`Tab::press`]), then the attempt again for the
    /// verdict. Whichever owner goes away drops it, so a stop never lands before input it was
    /// armed to judge. It holds a copy of its click's input turn until the stop has answered.
    type Armed: Send;
    /// A turn at the tab's input ([`Tab::input`]). Copies go to everything that operates the
    /// click's input: the interceptor's setup and the interceptor it arms, and the writer. The
    /// next click's turn starts only once the last copy drops, so a call that stops early never
    /// frees the tab under its own writer or its interceptor's stop.
    type Input: Devices;
    /// Navigation requests recorded from the moment [`Tab::watch_navigations`] returns.
    type Navigations: Send;

    /// Prepares this page's domains and watcher once, on its first click or beforeunload call.
    /// Later calls find it done. Playwright's page is
    /// ready before any call, so this comes after the call names its locator, and a failure
    /// retries like any page step.
    fn prepare(&self) -> impl Future<Output = Result<(), PageError>> + Send;

    /// `describeSelector`: the `waiting for` text, or the parse error.
    fn describe(
        &self,
        selector: &str,
    ) -> impl Future<Output = Result<SelectorFacts, PageError>> + Send;

    /// `resolve`: the first match with the match count and its preview, `None` while nothing
    /// matches. A strict violation is `PageError::Threw` with V8's `Error: strict mode …`.
    fn resolve(
        &self,
        selector: &str,
        strict: bool,
    ) -> impl Future<Output = Result<Option<Resolved<Self::Element>>, PageError>> + Send;

    /// `checkElementStates` with `frameVisible` inherited from the selector's iframe hop
    /// owners (`true` when the selector has no frame hops). With `scroll` (not Playwright's
    /// `scroll: 'none'`), an element in a child frame first has its frames' owners scrolled
    /// into view, so an offscreen frame paints and `stable` can resolve.
    fn check_states(
        &self,
        element: &Self::Element,
        states: &[ElementState],
        scroll: bool,
    ) -> impl Future<Output = Result<StatesCheck, PageError>> + Send;

    /// [`Scroll::Protocol`]: CDP `DOM.scrollIntoViewIfNeeded` (`position` as a zero rect); the
    /// other modes: the `scrollIntoView` glue. Returns why it could not scroll, if so.
    fn scroll(
        &self,
        element: &Self::Element,
        scroll: Scroll,
        position: Option<&Point>,
    ) -> impl Future<Output = Result<Option<NotReady>, PageError>> + Send;

    /// The raw facts the click point is computed from: `DOM.getContentQuads` with the
    /// `viewportSize` glue, or for `position`, `DOM.getBoxModel` with `getElementBorderWidth`.
    fn geometry(
        &self,
        element: &Self::Element,
        position: Option<&Point>,
        force: bool,
    ) -> impl Future<Output = Result<Geometry, PageError>> + Send;

    /// Waits for a turn at the tab's input: its one mouse and its document's one hit-target
    /// slot, which every call and every run on the tab share. Without turns, one click's setup
    /// replaces another's armed interceptor, and their events are judged by the wrong one. An
    /// attempt waits only after every wait for its element, so a click waiting for content never
    /// holds up the click that would produce it. Turns go in arrival order; a wait dropped before
    /// its turn holds nothing.
    fn input(&self) -> impl Future<Output = Self::Input> + Send;

    /// `setupHitTargetInterceptor(element, action, hit_point, false)`: the hit check at
    /// `hit_point`, then the interceptor armed for the coming input, judging the events of
    /// `action` (a hover's `mousemove`, a tap's touches, a click's presses). The page installs the
    /// interceptor before it answers, so once the setup is sent its answer is always read, on a
    /// task of its own: a caller that stopped waiting leaves an `Armed` nobody takes, which
    /// disarms as it drops. A future dropped before its first poll sends nothing. `input` is
    /// held until the answer, and an armed interceptor keeps it until its stop has answered.
    fn arm(
        &self,
        element: &Self::Element,
        hit_point: Point,
        action: HitTargetAction,
        input: Self::Input,
    ) -> impl Future<Output = Result<Arming<Self::Armed>, PageError>> + Send;

    /// Starts writing the action's input (a move, a click's mouse events, or a tap's touches) on a
    /// detached writer task *before it returns*, and gives back a future of the writer's report.
    /// The ticket moves into the writer, which marks it as it goes, so delivery stays true however
    /// the caller ends: dropping the returned future (the call stopped) stops the wait, never the
    /// writes. A press is always followed by its release, a touch start by its end, and a `delay`
    /// is cut short. Starting eagerly matters: a future that started the writer only when first
    /// polled could be dropped unpolled by a [`Progress::race`] that had already lost, leaving a
    /// begun ticket nobody marks.
    ///
    /// The armed interceptor moves into the writer too, and comes back with the report once
    /// every event is acknowledged. A caller that stopped waiting never gets it back, and the
    /// writer drops it only then: disarming any earlier would let the release of a press it
    /// blocked, and the page's `click`, through to whatever is under the pointer. The writer
    /// holds `input` until its last event is answered, which under `force`, with nothing armed,
    /// is all that keeps the next click from pressing at once.
    fn press(
        &self,
        plan: Plan,
        ticket: InputTicket,
        armed: Option<Self::Armed>,
        input: Self::Input,
    ) -> impl Future<Output = (PressReport, Option<Self::Armed>)> + Send;

    /// Disarms `armed` and reports what it saw: `stopHitTargetInterceptor`, raced against a
    /// dialog opening or a navigation being requested since it was armed (Playwright's
    /// `raceAgainstEvaluationStallingEvents`), either of which reads as `Done`, as does any
    /// error (Playwright's `.catch(() => 'done')`). Never fails.
    fn verdict(&self, armed: Self::Armed) -> impl Future<Output = Verdict> + Send;

    /// Playwright's pre-check: the URL of a main-frame navigation requested and not yet
    /// committed, finished in the document, or stopped.
    fn pending_navigation(&self) -> Option<String>;

    /// Resolves when no navigation is pending, with the URL it committed to, if any.
    fn navigation_settled(&self) -> impl Future<Output = Result<Option<String>, PageError>> + Send;

    /// Playwright's `performActionPreChecks` locator-handler checkpoint (`page.ts`
    /// `_performLocatorHandlersCheckpoint`), between its two navigation checks and before the
    /// attempt takes the input turn. A tab without handlers has none to run.
    fn locator_handlers_checkpoint(
        &self,
        _progress: &Progress,
    ) -> impl Future<Output = Result<(), Stop>> + Send {
        async { Ok(()) }
    }

    /// Starts recording navigation requests. Called before `press`, so none of the input's own
    /// requests can be missed.
    fn watch_navigations(&self) -> Self::Navigations;

    /// Playwright's input epilogue (`waitForSignalsCreatedBy`): a flush after which every
    /// navigation the input requested has been reported, then a wait until each requested
    /// main-frame navigation commits or stops. Returns the URLs committed, for the log.
    fn settle_after_input(
        &self,
        watched: Self::Navigations,
    ) -> impl Future<Output = Result<Vec<String>, PageError>> + Send;
}

/// What `describeSelector` learned, once per call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectorFacts {
    /// `asLocator('javascript', selector)`, for the first log line.
    pub(crate) locator: String,
    /// The parse error's message (no `Error: `), which ends the call at once.
    pub(crate) invalid: Option<String>,
}

/// The first element a selector matched.
#[derive(Debug)]
pub(crate) struct Resolved<E> {
    pub(crate) element: E,
    /// How many elements matched; above one only without strict mode.
    pub(crate) count: u32,
    /// `InjectedScript.previewNode` of the element.
    pub(crate) preview: String,
}

/// `checkElementStates`' answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StatesCheck {
    Ready,
    NotConnected,
    Missing(ElementState),
}

/// Raw geometry from which action policy computes its target point.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Geometry {
    /// `getContentQuads` or `getBoxModel` failed or found nothing: `error:notvisible`
    /// (Playwright's `_sendMayFail`).
    NoQuads,
    Quads {
        quads: Vec<[f64; 8]>,
        viewport: (f64, f64),
    },
    /// With `position`: the border box quad and the element's border widths.
    Offset {
        border_box: [f64; 8],
        border: (f64, f64),
    },
}

/// Why an attempt did not reach input: Playwright's `PerformActionResult` minus `done`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NotReady {
    /// `error:notconnected`: back to loop A to resolve again.
    NotConnected,
    NotVisible,
    NotInViewport,
    Missing(ElementState),
    /// The hit target is another element: its preview.
    Intercepted(String),
}

/// `setupHitTargetInterceptor`'s answer.
#[derive(Debug)]
pub(crate) enum Arming<A> {
    Armed(A),
    NotConnected,
    /// The preliminary hit test found another element: its preview.
    Intercepted(String),
}

/// What the armed interceptor saw at the first trusted pointer event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// The event hit the element, or no event arrived, or a dialog or navigation made reading
    /// it moot, or reading it failed: Playwright's `'done'`.
    Done,
    /// The event hit another element, and the interceptor blocked it and every later event.
    Blocked(String),
}

/// How far the writer got. The ticket's state already says the same; this is what the call
/// task acts on.
#[derive(Debug)]
pub(crate) enum PressReport {
    /// Every event that was written was acknowledged.
    Acked(AckedInput),
    /// Nothing was written (the writer called `InputTicket::unsent`).
    NothingWritten,
    /// Something may have been written, then CDP failed; delivery stays `Unknown`. `closed`
    /// when the session or the connection is gone, which reads as the tab closing.
    Failed { closed: bool, message: String },
}

/// Why a page step could not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PageError {
    /// Any protocol error other than a closed session: the context, document or object is
    /// gone, or the frame is mid-commit. Playwright retries these ("Execution context was
    /// destroyed" and kin), so does Neo, without matching error text.
    NotConnected,
    /// The page threw: `exceptionDetails.exception.description` (`Error: <message>` for
    /// Playwright's stackless errors). Ends the call (`JavaScriptErrorInEvaluate`).
    Threw(String),
    /// The tab closed or the session is gone.
    Closed,
    /// The page answered outside the spec: a bundle/spec mismatch. Ends the call.
    Contract(String),
}

/// The scroll of one loop-B attempt, by retry index (`dom.ts` `_retryPointerAction`): CDP
/// first, then the page's `scrollIntoView` with `end`, `center`, `start`, which gets an element
/// out from under a sticky header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scroll {
    Protocol,
    End,
    Center,
    Start,
}

/// Why one loop-A iteration ended without its action.
#[derive(Debug)]
pub(crate) enum Stop {
    /// A page step answered [`PageError::NotConnected`]: resolve again, as Playwright retries
    /// every error its `isNonRetriableError` does not name.
    Retry,
    /// The call ends with this error.
    End(CallError),
}

impl From<CallError> for Stop {
    fn from(error: CallError) -> Self {
        Self::End(error)
    }
}

impl From<PlaywrightError> for Stop {
    fn from(error: PlaywrightError) -> Self {
        Self::End(error.into())
    }
}

impl From<PageError> for Stop {
    fn from(error: PageError) -> Self {
        match error {
            PageError::NotConnected => Self::Retry,
            PageError::Threw(message) | PageError::Contract(message) => {
                PlaywrightError::error(message).into()
            }
            PageError::Closed => PlaywrightError::target_closed().into(),
        }
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! Neo: a scripted implementation of the tab seam for action-policy tests.
    //! It models detached writers and shared input ownership under paused Tokio time, so policy
    //! tests can observe the same delivery and cleanup boundaries without a running browser.

    use std::collections::VecDeque;
    use std::future::{self, Future};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use browseros_cdp::SessionId;
    use neo_runtime_spec::injected::{ElementState, HitTargetAction, Point};
    use serde_json::Value;
    use tokio::sync::oneshot;
    use tokio::time;

    use super::{
        Arming, Geometry, NotReady, PageError, PressReport, Resolved, Scroll, SelectorFacts,
        StatesCheck, Stop, Tab, Verdict,
    };
    use crate::chromium_bridge::keyboard_input::Platform;
    use crate::chromium_bridge::mouse_and_touch_input::{
        Devices, Effect, EffectSink, InputDevices, Owner, Plan,
    };
    use crate::playwright_methods::forms::tests::FormSteps;
    use crate::progress::{InputTicket, Progress};

    /// Neo: one millisecond models a scripted page round trip.
    pub(crate) const ROUND_TRIP: Duration = Duration::from_millis(1);
    /// Neo: the fixture models two animation frames at sixty hertz.
    pub(crate) const STABLE: Duration = Duration::from_millis(33);
    /// Neo: the shared fixture targets its submit button.
    pub(crate) const SELECTOR: &str = "internal:role=button[name=\"Submit\"i]";
    /// Neo: the fixture display name for the submit button.
    pub(crate) const LOCATOR: &str = "getByRole('button', { name: 'Submit' })";
    /// Neo: the fixture preview of the resolved submit button.
    pub(crate) const PREVIEW: &str = "<button>Submit</button>";
    /// Neo: the fixture preview of an intercepting overlay.
    pub(crate) const OVERLAY: &str = "<div id=\"overlay\"></div>";
    /// Neo: the fixture gives a visible box inside its viewport.
    pub(crate) const OK_QUAD: [f64; 8] = [8.0, 8.0, 65.5, 8.0, 65.5, 29.0, 8.0, 29.0];

    /// One scripted step: each call takes `duration` and pops the next answer; the last answer
    /// repeats.
    pub(crate) struct Step<T> {
        pub(crate) duration: Duration,
        answers: Mutex<VecDeque<T>>,
    }

    impl<T: Clone> Step<T> {
        pub(crate) fn new(duration: Duration, answers: impl IntoIterator<Item = T>) -> Self {
            Self {
                duration,
                answers: Mutex::new(answers.into_iter().collect()),
            }
        }

        pub(crate) fn always(duration: Duration, answer: T) -> Self {
            Self::new(duration, [answer])
        }

        pub(crate) fn next(&self) -> T {
            let mut answers = self.answers.lock().expect("a fake step's lock");
            if answers.len() > 1 {
                answers.pop_front().expect("checked non-empty")
            } else {
                answers.front().cloned().expect("a fake step has an answer")
            }
        }

        pub(crate) async fn answer(&self) -> T {
            let answer = self.next();
            time::sleep(self.duration).await;
            answer
        }
    }

    /// The writer's scripted outcome.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum Writes {
        Ack,
        Nothing,
        Fail,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub(crate) enum ArmAnswer {
        Armed,
        NotConnected,
        Intercepted(String),
    }

    /// A dropped armed interceptor, so tests see every path disarm. It holds a copy of its
    /// click's input turn, as the real one does until its stop has answered.
    pub(crate) struct FakeArmed {
        calls: Arc<Mutex<Vec<String>>>,
        _input: FakeInput,
    }

    impl Drop for FakeArmed {
        fn drop(&mut self) {
            self.calls
                .lock()
                .expect("calls")
                .push("disarmed".to_string());
        }
    }

    /// A turn at the fake tab's input, shared by its copies like the real one. The tab records
    /// `input` when it grants a turn and `input released` when the turn's last copy drops, so
    /// tests see where each input phase starts and ends.
    #[derive(Clone)]
    pub(crate) struct FakeInput {
        _turn: Arc<FakeTurn>,
        /// The fake tab's devices, which the turn reaches as the real one reaches its tab's.
        devices: Arc<Mutex<InputDevices>>,
    }

    /// The run that holds every fake turn.
    pub(crate) fn fake_owner() -> Owner {
        SessionId("RUN".to_string())
    }

    impl EffectSink for FakeInput {
        fn apply(&self, effect: &Effect, answered: bool) {
            // Each write is its own turn here.
            let mut devices = self.devices.lock().expect("devices");
            devices.apply(effect, answered);
            devices.forget_ended();
        }

        fn devices(&self) -> InputDevices {
            self.devices.lock().expect("devices").clone()
        }
    }

    impl Devices for FakeInput {
        fn snapshot(&self) -> Result<InputDevices, PageError> {
            Ok(self.devices.lock().expect("devices").clone())
        }

        fn owner(&self) -> Owner {
            fake_owner()
        }
    }

    /// What a plan writes, as the fake tab records it: `tap (x, y)`, `press (x, y)`, `press twice
    /// (x, y)` or `move (x, y)`, at the plan's last pointer point.
    pub(crate) fn plan_summary(plan: &Plan) -> String {
        let events: Vec<Value> = plan
            .commands
            .iter()
            .filter(|command| command.method().is_some())
            .map(|command| serde_json::from_str(command.params()).expect("params are JSON"))
            .collect();
        let kind = |event: &Value| event["type"].as_str().unwrap_or_default().to_string();
        if let Some(start) = events.iter().find(|event| kind(event) == "touchStart") {
            let at = &start["touchPoints"][0];
            return format!("tap ({}, {})", at["x"], at["y"]);
        }
        let presses = events
            .iter()
            .filter(|event| kind(event) == "mousePressed")
            .count();
        let verb = match presses {
            0 => "move",
            2 => "press twice",
            _ => "press",
        };
        let last = events
            .iter()
            .rev()
            .find(|event| kind(event).starts_with("mouse"))
            .map_or((0.0, 0.0), |event| {
                (
                    event["x"].as_f64().unwrap_or_default(),
                    event["y"].as_f64().unwrap_or_default(),
                )
            });
        format!("{verb} ({}, {})", last.0, last.1)
    }

    struct FakeTurn(Arc<Mutex<Vec<String>>>);

    impl Drop for FakeTurn {
        fn drop(&mut self) {
            self.0
                .lock()
                .expect("calls")
                .push("input released".to_string());
        }
    }

    /// A scripted tab. Every page step records its name in `calls`.
    pub(crate) struct FakeTab {
        pub(crate) forms: FormSteps,
        pub(crate) platform: Step<Result<Platform, PageError>>,
        /// Instant, like every preparation after the run's first.
        pub(crate) prepare: Step<Result<(), PageError>>,
        pub(crate) describe: Step<Result<SelectorFacts, PageError>>,
        pub(crate) resolve: Step<Result<Option<(u32, String)>, PageError>>,
        pub(crate) check_states: Step<Result<StatesCheck, PageError>>,
        pub(crate) scroll: Step<Result<Option<NotReady>, PageError>>,
        pub(crate) geometry: Step<Result<Geometry, PageError>>,
        pub(crate) arm: Step<Result<ArmAnswer, PageError>>,
        pub(crate) press: Step<Writes>,
        pub(crate) verdict: Step<Verdict>,
        pub(crate) settle: Step<Result<Vec<String>, PageError>>,
        pub(crate) pending: Mutex<Option<String>>,
        pub(crate) calls: Arc<Mutex<Vec<String>>>,
        /// The states each `check_states` asked for, in call order.
        pub(crate) states: Mutex<Vec<Vec<ElementState>>>,
        /// The tab's devices, shared with every turn it grants.
        pub(crate) devices: Arc<Mutex<InputDevices>>,
        /// Every plan `press` was given, in call order.
        pub(crate) plans: Mutex<Vec<Plan>>,
        /// Each locator handlers' checkpoint, as how many plans had been pressed before it.
        pub(crate) checkpoints: Mutex<Vec<usize>>,
    }

    impl FakeTab {
        /// The `ok` page: one button that every step finds ready.
        pub(crate) fn ready() -> Self {
            Self {
                forms: FormSteps::ready(),
                platform: Step::always(ROUND_TRIP, Ok(Platform::Mac)),
                prepare: Step::always(Duration::ZERO, Ok(())),
                describe: Step::always(
                    ROUND_TRIP,
                    Ok(SelectorFacts {
                        locator: LOCATOR.to_string(),
                        invalid: None,
                    }),
                ),
                resolve: Step::always(ROUND_TRIP, Ok(Some((1, PREVIEW.to_string())))),
                check_states: Step::always(STABLE, Ok(StatesCheck::Ready)),
                scroll: Step::always(ROUND_TRIP, Ok(None)),
                geometry: Step::always(
                    ROUND_TRIP,
                    Ok(Geometry::Quads {
                        quads: vec![OK_QUAD],
                        viewport: (516.0, 475.0),
                    }),
                ),
                arm: Step::always(ROUND_TRIP, Ok(ArmAnswer::Armed)),
                press: Step::always(ROUND_TRIP, Writes::Ack),
                verdict: Step::always(ROUND_TRIP, Verdict::Done),
                settle: Step::always(ROUND_TRIP, Ok(Vec::new())),
                pending: Mutex::new(None),
                calls: Arc::default(),
                states: Mutex::default(),
                devices: Arc::default(),
                plans: Mutex::default(),
                checkpoints: Mutex::default(),
            }
        }

        pub(crate) fn record(&self, call: impl Into<String>) {
            self.calls.lock().expect("calls").push(call.into());
        }

        pub(crate) fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("calls").clone()
        }

        pub(crate) fn count(&self, name: &str) -> usize {
            self.calls().iter().filter(|call| *call == name).count()
        }
    }

    impl Tab for FakeTab {
        type Element = u32;
        type Armed = FakeArmed;
        type Input = FakeInput;
        type Navigations = ();

        async fn prepare(&self) -> Result<(), PageError> {
            self.prepare.answer().await
        }

        async fn describe(&self, _selector: &str) -> Result<SelectorFacts, PageError> {
            self.record("describe");
            self.describe.answer().await
        }

        async fn resolve(
            &self,
            _selector: &str,
            _strict: bool,
        ) -> Result<Option<Resolved<u32>>, PageError> {
            self.record("resolve");
            let answer = self.resolve.answer().await?;
            Ok(answer.map(|(count, preview)| Resolved {
                element: 1,
                count,
                preview,
            }))
        }

        async fn check_states(
            &self,
            _element: &u32,
            states: &[ElementState],
            scroll: bool,
        ) -> Result<StatesCheck, PageError> {
            self.states.lock().expect("states").push(states.to_vec());
            self.record(if scroll {
                "check_states"
            } else {
                "check_states without scrolling"
            });
            self.check_states.answer().await
        }

        async fn scroll(
            &self,
            _element: &u32,
            scroll: Scroll,
            _position: Option<&Point>,
        ) -> Result<Option<NotReady>, PageError> {
            self.record(format!("scroll {scroll:?}"));
            self.scroll.answer().await
        }

        async fn geometry(
            &self,
            _element: &u32,
            _position: Option<&Point>,
            _force: bool,
        ) -> Result<Geometry, PageError> {
            self.record("geometry");
            self.geometry.answer().await
        }

        /// Forms can script a blocked input wait; click tests grant their turns at once.
        async fn input(&self) -> FakeInput {
            if !self.forms.input_wait.is_zero() {
                time::sleep(self.forms.input_wait).await;
            }
            self.record("input");
            FakeInput {
                _turn: Arc::new(FakeTurn(Arc::clone(&self.calls))),
                devices: Arc::clone(&self.devices),
            }
        }

        async fn arm(
            &self,
            _element: &u32,
            hit_point: Point,
            action: HitTargetAction,
            input: FakeInput,
        ) -> Result<Arming<FakeArmed>, PageError> {
            self.record(format!("arm {action:?} {hit_point:?}"));
            Ok(match self.arm.answer().await? {
                ArmAnswer::Armed => Arming::Armed(FakeArmed {
                    calls: Arc::clone(&self.calls),
                    _input: input,
                }),
                ArmAnswer::NotConnected => Arming::NotConnected,
                ArmAnswer::Intercepted(target) => Arming::Intercepted(target),
            })
        }

        fn press(
            &self,
            plan: Plan,
            ticket: InputTicket,
            armed: Option<FakeArmed>,
            input: FakeInput,
        ) -> impl Future<Output = (PressReport, Option<FakeArmed>)> + Send {
            self.record(plan_summary(&plan));
            self.plans.lock().expect("plans").push(plan.clone());
            let writes = self.press.next();
            let duration = self.press.duration;
            let calls = Arc::clone(&self.calls);
            let (report, receiver) = oneshot::channel();
            // Detached, like the real writer: it marks the ticket whether or not anyone waits,
            // and holds the interceptor until its last event is answered.
            tokio::spawn(async move {
                time::sleep(duration).await;
                calls.lock().expect("calls").push("released".to_string());
                let outcome = match writes {
                    Writes::Ack => {
                        // A perfect writer: every command written and answered.
                        for command in &plan.commands {
                            input.apply(&command.effect, true);
                        }
                        PressReport::Acked(ticket.acked())
                    }
                    Writes::Nothing => {
                        ticket.unsent();
                        PressReport::NothingWritten
                    }
                    Writes::Fail => {
                        drop(ticket);
                        PressReport::Failed {
                            closed: false,
                            message: "Input.dispatchMouseEvent timed out".to_string(),
                        }
                    }
                };
                // Done writing: the writer's copy of the turn goes, as the real writer's does.
                drop(input);
                let _waiting = report.send((outcome, armed));
            });
            async move {
                receiver.await.unwrap_or_else(|_| {
                    let failed = PressReport::Failed {
                        closed: false,
                        message: "the fake writer ended".to_string(),
                    };
                    (failed, None)
                })
            }
        }

        async fn verdict(&self, armed: FakeArmed) -> Verdict {
            self.record("verdict");
            let verdict = self.verdict.answer().await;
            drop(armed);
            verdict
        }

        fn pending_navigation(&self) -> Option<String> {
            self.pending.lock().expect("pending").clone()
        }

        async fn navigation_settled(&self) -> Result<Option<String>, PageError> {
            self.record("navigation_settled");
            time::sleep(ROUND_TRIP).await;
            Ok(self.pending.lock().expect("pending").take())
        }

        fn locator_handlers_checkpoint(
            &self,
            _progress: &Progress,
        ) -> impl Future<Output = Result<(), Stop>> + Send {
            let pressed = self.plans.lock().expect("plans").len();
            self.checkpoints.lock().expect("checkpoints").push(pressed);
            future::ready(Ok(()))
        }

        fn watch_navigations(&self) {
            self.record("watch_navigations");
        }

        async fn settle_after_input(&self, (): ()) -> Result<Vec<String>, PageError> {
            self.record("settle");
            self.settle.answer().await
        }
    }
}
