//! What claw-sdk builds, over a throwaway BrowserOS on `BROWSEROS_CDP_PORT`: a
//! `BrowserSession`, one `Playwright`, and an `OpEnv` that records what the crate asked of its
//! host. Tests drive the crate through `Playwright::invoke` only; they set a page up (a trusted
//! click, a script) on the page's own CDP session, as a user or the page itself would. The
//! connection passes through a [`Tap`], so a test can step in at one CDP command of an op.

use std::error::Error;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::{env, future};

use browseros_cdp::{CdpClient, CdpError, CdpEvent, ConnectOptions};
use browseros_core::sensitive_fields::{FieldValues, JournalEntry};
use browseros_core::{
    BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, PageId, ProtocolSession,
    SessionId, TabId,
};
use browseros_playwright::{
    ErrorCode, Op, OpEnv, Playwright, PlaywrightError, PlaywrightHooks, Refusal, RunState, TypedBy,
};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};

/// A live test's result.
pub(crate) type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// One request the crate made of its host, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EnvCall {
    Recheck,
    MarkSent,
    MarkSensitiveTarget,
    Note(String),
}

/// What the [`Tap`] does to the first command the crate sends with the method a test named.
#[derive(Clone, Debug)]
pub(crate) enum Intercept {
    /// The command goes through, and the op's next `recheck` refuses with this, as if the owner
    /// took over while the command was in flight.
    Refuse(Refusal),
    /// The browser never answers the command, so the op waits there until the test drops it.
    Hang,
}

/// The host's answer to the next `recheck`, which the [`Tap`] can set; `None` allows.
type Policy = Arc<Mutex<Option<Refusal>>>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The run's CDP connection: every command and event passes to the browser unchanged, except
/// the one command a test asked to intercept.
struct Tap {
    browser: Arc<dyn CdpConnection>,
    watch: Mutex<Option<(&'static str, Intercept)>>,
    policy: Policy,
}

impl Tap {
    /// Acts on `method` if it is the watched one, once; whether the command must hang.
    fn hangs(&self, method: &str) -> bool {
        let mut watch = lock(&self.watch);
        if watch.as_ref().is_none_or(|(watched, _)| *watched != method) {
            return false;
        }
        match watch.take() {
            Some((_, Intercept::Refuse(refusal))) => {
                *lock(&self.policy) = Some(refusal);
                false
            }
            Some((_, Intercept::Hang)) => true,
            None => false,
        }
    }
}

impl CdpConnection for Tap {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        if self.hangs(method) {
            return Box::pin(future::pending());
        }
        self.browser.send(method, params, session)
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        if self.hangs(method) {
            return Box::pin(future::pending());
        }
        self.browser.send_raw_json(method, params_json, session)
    }

    fn send_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        if self.hangs(method) {
            return Box::pin(future::pending());
        }
        self.browser
            .send_at_epoch(expected_epoch, method, params, session)
    }

    fn send_raw_json_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        if self.hangs(method) {
            return Box::pin(future::pending());
        }
        self.browser
            .send_raw_json_at_epoch(expected_epoch, method, params_json, session)
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.browser.events()
    }

    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        self.browser.events_targeted(method)
    }

    fn is_connected(&self) -> bool {
        self.browser.is_connected()
    }

    fn connection_epoch(&self) -> u64 {
        self.browser.connection_epoch()
    }
}

/// The run's side of every op: an unbound view, one `RunState`, and a record of the calls.
struct LiveEnv {
    view: BrowserView,
    run_state: Arc<RunState>,
    calls: Mutex<Vec<EnvCall>>,
    policy: Policy,
}

impl LiveEnv {
    fn record(&self, call: EnvCall) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }
}

impl OpEnv for LiveEnv {
    fn view(&self) -> &BrowserView {
        &self.view
    }

    fn run_state(&self) -> &Arc<RunState> {
        &self.run_state
    }

    /// Allows, unless the tap set a refusal for this one call.
    fn recheck(&self) -> Result<(), Refusal> {
        self.record(EnvCall::Recheck);
        lock(&self.policy).take().map_or(Ok(()), Err)
    }

    fn mark_sent(&self) {
        self.record(EnvCall::MarkSent);
    }

    fn mark_sensitive_target(&self) {
        self.record(EnvCall::MarkSensitiveTarget);
    }

    fn note(&self, line: String) {
        self.record(EnvCall::Note(line));
    }
}

/// The host's privacy layer: every capture is allowed until a test refuses them, and every value
/// the evaluate guard swept is kept, as the host would register it.
#[derive(Default)]
struct LiveHooks {
    capture: Mutex<Option<Refusal>>,
    swept: Mutex<Vec<String>>,
}

impl PlaywrightHooks for LiveHooks {
    fn field_entered(&self, _page: PageId, _entry: JournalEntry, _by: TypedBy) {}

    fn values_swept(&self, _page: PageId, values: &FieldValues) {
        self.swept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(values.registrable().map(str::to_owned));
    }

    fn capture_allowed(&self, _page: PageId) -> Result<(), Refusal> {
        self.capture
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .map_or(Ok(()), Err)
    }
}

/// A tab a test drives, as a page handle names it: core's page id and the browser's tab id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TestPage {
    pub(crate) id: PageId,
    pub(crate) tab: TabId,
}

/// One run against the live browser.
pub(crate) struct LiveRun {
    browser: Arc<BrowserSession>,
    playwright: Arc<Playwright>,
    env: LiveEnv,
    hooks: Arc<LiveHooks>,
    tap: Arc<Tap>,
}

impl LiveRun {
    /// Connects to `BROWSEROS_CDP_PORT` (default 9005) and builds the crate's objects.
    pub(crate) async fn connect() -> TestResult<Self> {
        Self::connect_through(|client| Arc::new(client) as Arc<dyn CdpConnection>).await
    }

    /// As [`LiveRun::connect`], with every CDP call going through the connection `wrap` builds
    /// over the client, so a test can stand between the crate and the browser.
    pub(crate) async fn connect_through(
        wrap: impl FnOnce(CdpClient) -> Arc<dyn CdpConnection>,
    ) -> TestResult<Self> {
        let port = env::var("BROWSEROS_CDP_PORT")
            .ok()
            .and_then(|port| port.parse().ok())
            .unwrap_or(9005);
        let client = CdpClient::connect(ConnectOptions::new(port)).await?;
        let policy = Policy::default();
        // The tap sits above whatever `wrap` built, so a test's interception and its own
        // connection compose.
        let tap = Arc::new(Tap {
            browser: wrap(client),
            watch: Mutex::default(),
            policy: Arc::clone(&policy),
        });
        let browser = BrowserSession::new(
            Arc::clone(&tap) as Arc<dyn CdpConnection>,
            BrowserSessionHooks::default(),
        );
        let hooks = Arc::new(LiveHooks::default());
        let playwright = Playwright::new(
            Arc::clone(&browser),
            Arc::clone(&hooks) as Arc<dyn PlaywrightHooks>,
        );
        let env = LiveEnv {
            view: BrowserView::unbound(Arc::clone(&browser), None),
            run_state: Arc::default(),
            calls: Mutex::default(),
            policy,
        };
        Ok(Self {
            browser,
            playwright,
            env,
            hooks,
            tap,
        })
    }

    /// The browser session the crate runs over, for a test that reads core's registries.
    pub(crate) fn browser(&self) -> &BrowserSession {
        &self.browser
    }

    /// The next command the crate sends with `method` gets `intercept`; later ones pass.
    pub(crate) fn intercept(&self, method: &'static str, intercept: Intercept) {
        *lock(&self.tap.watch) = Some((method, intercept));
    }

    /// Runs op `name` with `input`, as claw-sdk's handler does.
    pub(crate) async fn invoke(&self, name: &str, input: Value) -> Result<Value, PlaywrightError> {
        let op = Op::from_name(name).ok_or_else(|| {
            PlaywrightError::new(ErrorCode::Internal, format!("{name} is not an op"))
        })?;
        self.playwright.invoke(&self.env, op, input).await
    }

    /// Ends this run and starts the next on the same `Playwright`, as claw-sdk does between
    /// runs: the old `RunState` drops, so its listeners stop counting and a dialog it held is
    /// left to the grace and the reap.
    pub(crate) fn next_run(&mut self) {
        self.env.run_state = Arc::default();
    }

    /// A new background tab, from `context.newPage`.
    pub(crate) async fn new_page(&self) -> TestResult<TestPage> {
        let opened = self.invoke("context.newPage", json!({})).await?;
        test_page(opened.get("page").ok_or("newPage returns a page")?)
    }

    /// Closes `page`, whatever state the test left it in.
    pub(crate) async fn close_page(&self, page: &TestPage) {
        let _ = self
            .invoke("page.close", json!({ "page": brand(page) }))
            .await;
    }

    /// Runs `expression` in the page's own world, with a user gesture, by value.
    pub(crate) async fn eval(&self, page: &TestPage, expression: &str) -> TestResult<Value> {
        let session = self.session(page).await?;
        let reply = session
            .send_value(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true, "awaitPromise": true,
                        "userGesture": true }),
            )
            .await?;
        if let Some(details) = reply.get("exceptionDetails") {
            return Err(format!("the page threw: {details}").into());
        }
        Ok(reply
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// A trusted left click at the center of the first element matching `selector`.
    pub(crate) async fn click(&self, page: &TestPage, selector: &str) -> TestResult {
        let center = self
            .eval(
                page,
                &format!(
                    "(() => {{ const r = document.querySelector({selector:?}).getBoundingClientRect(); \
                     return [r.x + r.width / 2, r.y + r.height / 2]; }})()"
                ),
            )
            .await?;
        let coordinate = |index: usize| center.get(index).and_then(Value::as_f64);
        let (Some(x), Some(y)) = (coordinate(0), coordinate(1)) else {
            return Err(format!("{selector} has no box: {center}").into());
        };
        let session = self.session(page).await?;
        for kind in ["mousePressed", "mouseReleased"] {
            session
                .send_value(
                    "Input.dispatchMouseEvent",
                    json!({ "type": kind, "x": x, "y": y, "button": "left", "clickCount": 1 }),
                )
                .await?;
        }
        Ok(())
    }

    /// The values the evaluate guard reported so far, long enough to register.
    pub(crate) fn swept(&self) -> Vec<String> {
        self.hooks
            .swept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Every capture from now on is refused with `refusal`.
    pub(crate) fn refuse_captures(&self, refusal: Refusal) {
        *self
            .hooks
            .capture
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(refusal);
    }

    /// What the crate asked of the host so far.
    pub(crate) fn calls(&self) -> Vec<EnvCall> {
        self.env
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Forgets the calls recorded so far.
    pub(crate) fn clear_calls(&self) {
        self.env
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// The page's attached session, the one core and the crate use.
    async fn session(&self, page: &TestPage) -> TestResult<ProtocolSession> {
        Ok(self
            .browser
            .pages
            .get_session(page.id.clone())
            .await?
            .session)
    }
}

/// `{"$neo":"page","id":7,"tab":1234}` for `page`.
pub(crate) fn brand(page: &TestPage) -> Value {
    json!({ "$neo": "page", "id": page.id.0, "tab": page.tab.0 })
}

/// The page a `{"$neo":"page","id":7,"tab":1234}` handle names.
pub(crate) fn test_page(handle: &Value) -> TestResult<TestPage> {
    if handle.get("$neo").and_then(Value::as_str) != Some("page") {
        return Err(format!("not a page handle: {handle}").into());
    }
    let id = handle
        .get("id")
        .and_then(Value::as_u64)
        .ok_or("a page id is a number")?;
    let tab = handle
        .get("tab")
        .and_then(Value::as_i64)
        .ok_or("a page handle carries its tab")?;
    Ok(TestPage {
        id: PageId(u32::try_from(id)?),
        tab: TabId(tab),
    })
}
