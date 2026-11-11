//! What claw-sdk builds, over a throwaway BrowserOS on `BROWSEROS_CDP_PORT`: a
//! `BrowserSession`, one `Playwright`, and an `OpEnv` that records what the crate asked of its
//! host. Tests drive the crate through `Playwright::invoke` only; they set a page up (a trusted
//! click, a script) on the page's own CDP session, as a user or the page itself would. The
//! connection passes through a [`Tap`], so a test can step in at one CDP command of an op.

use std::error::Error;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use std::{env, future};

use browseros_cdp::{CdpClient, CdpError, CdpEvent, ConnectOptions};
use browseros_core::{
    BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, PageId, ProtocolSession,
    SessionId, TabId,
};
use futures_util::future::BoxFuture;
use neo_playwright::{ErrorCode, Op, OpEnv, Playwright, PlaywrightError, Refusal, RunState};
use serde_json::{Map, Value, json};
use tokio::sync::{broadcast, mpsc};
use tokio::time;

/// A live test's result.
pub(crate) type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// One request the crate made of its host, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EnvCall {
    Recheck,
    MarkSent,
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

/// The same owned browser for SDK operations and independent fixture observations.
fn cdp_port() -> u16 {
    env::var("BROWSEROS_CDP_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(9005)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The run's CDP connection: every command and event passes to the browser unchanged, except
/// the one command a test asked to intercept.
struct Tap {
    browser: Arc<dyn CdpConnection>,
    watch: Mutex<Option<(&'static str, Intercept)>>,
    policy: Policy,
    /// The `Input.*` commands sent while a test records them, as `{ method, params }`.
    inputs: Mutex<Option<Vec<Value>>>,
    /// Every command's method sent while a test records them.
    methods: Mutex<Option<Vec<String>>>,
    /// The params of every `DOM.setFileInputFiles` the crate sent, for the paths that reach
    /// Chromium.
    file_inputs: Mutex<Vec<Value>>,
}

impl Tap {
    /// Keeps `method` while a test records methods, its params too when it is an input command
    /// and a test records input, and every `DOM.setFileInputFiles`.
    fn record(&self, method: &str, params: impl FnOnce() -> Value) {
        if let Some(methods) = lock(&self.methods).as_mut() {
            methods.push(method.to_owned());
        }
        if method == "DOM.setFileInputFiles" {
            lock(&self.file_inputs).push(params());
            return;
        }
        if !method.starts_with("Input.") {
            return;
        }
        if let Some(inputs) = lock(&self.inputs).as_mut() {
            inputs.push(json!({ "method": method, "params": params() }));
        }
    }

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
        self.record(method, || params.clone());
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
        self.record(method, || {
            serde_json::from_str(params_json).unwrap_or(Value::Null)
        });
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
        self.record(method, || params.clone());
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
        self.record(method, || {
            serde_json::from_str(params_json).unwrap_or(Value::Null)
        });
        self.browser
            .send_raw_json_at_epoch(expected_epoch, method, params_json, session)
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.browser.events()
    }

    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        self.browser.events_targeted(method)
    }

    // The counts the watcher's fence reads: without them every op would skip the wait for
    // events that came before its reply (`Playwright::events_before_reply`).
    fn events_broadcast(&self) -> u64 {
        self.browser.events_broadcast()
    }

    fn events_counted(&self) -> (broadcast::Receiver<CdpEvent>, u64) {
        self.browser.events_counted()
    }

    fn events_targeted_counted(&self, method: &str) -> (mpsc::UnboundedReceiver<CdpEvent>, u64) {
        self.browser.events_targeted_counted(method)
    }

    fn events_targeted_delivered(&self, method: &str) -> u64 {
        self.browser.events_targeted_delivered(method)
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
    /// What `recheck` refuses with, as a takeover would, until a test allows it again.
    refusal: Mutex<Option<Refusal>>,
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

    /// Allows, unless the tap set a refusal for this one call, or a test refuses every recheck.
    fn recheck(&self) -> Result<(), Refusal> {
        self.record(EnvCall::Recheck);
        if let Some(refusal) = lock(&self.policy).take() {
            return Err(refusal);
        }
        lock(&self.refusal).clone().map_or(Ok(()), Err)
    }

    fn mark_sent(&self) {
        self.record(EnvCall::MarkSent);
    }

    fn note(&self, line: String) {
        self.record(EnvCall::Note(line));
    }
}

/// The op `name` names.
fn op(name: &str) -> Result<Op, PlaywrightError> {
    Op::from_name(name)
        .ok_or_else(|| PlaywrightError::new(ErrorCode::Internal, format!("{name} is not an op")))
}

/// A test-owned native tab. Core IDs are resolved only when a fixture needs raw CDP.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TestPage {
    pub(crate) tab: TabId,
}

/// One run against the live browser.
pub(crate) struct LiveRun {
    browser: Arc<BrowserSession>,
    playwright: Arc<Playwright>,
    env: LiveEnv,
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
        let port = cdp_port();
        let client = CdpClient::connect(ConnectOptions::new(port)).await?;
        let policy = Policy::default();
        // The tap sits above whatever `wrap` built, so a test's interception and its own
        // connection compose.
        let tap = Arc::new(Tap {
            browser: wrap(client),
            watch: Mutex::default(),
            policy: Arc::clone(&policy),
            inputs: Mutex::default(),
            methods: Mutex::default(),
            file_inputs: Mutex::default(),
        });
        let browser = BrowserSession::new(
            Arc::clone(&tap) as Arc<dyn CdpConnection>,
            BrowserSessionHooks::default(),
        );
        let playwright = Playwright::new(Arc::clone(&browser));
        let env = LiveEnv {
            view: BrowserView::unbound(Arc::clone(&browser), None),
            run_state: Arc::default(),
            calls: Mutex::default(),
            policy,
            refusal: Mutex::default(),
        };
        Ok(Self {
            browser,
            playwright,
            env,
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

    /// Starts recording the `Input.*` commands the crate sends, forgetting earlier ones.
    pub(crate) fn record_input(&self) {
        *lock(&self.tap.inputs) = Some(Vec::new());
    }

    /// The `Input.*` commands sent since [`LiveRun::record_input`], as `{ method, params }`;
    /// recording goes on.
    pub(crate) fn recorded_input(&self) -> Vec<Value> {
        lock(&self.tap.inputs).clone().unwrap_or_default()
    }

    /// The params of every `DOM.setFileInputFiles` sent so far, in order.
    pub(crate) fn file_inputs_sent(&self) -> Vec<Value> {
        lock(&self.tap.file_inputs).clone()
    }

    /// Starts recording the method of every command the crate sends, forgetting earlier ones.
    pub(crate) fn record_methods(&self) {
        *lock(&self.tap.methods) = Some(Vec::new());
    }

    /// The methods sent since [`LiveRun::record_methods`], in order; recording stops.
    pub(crate) fn recorded_methods(&self) -> Vec<String> {
        lock(&self.tap.methods).take().unwrap_or_default()
    }

    /// The event head sampled before Runner N reads its page facts.
    pub(crate) fn event_head(&self) -> u64 {
        self.playwright.event_head()
    }

    /// Mirrors the door's cache observation for live tests, including watcher/core lag.
    pub(crate) async fn page_facts(&self, tabs: Vec<TabId>) -> Vec<Value> {
        let mut facts = Vec::new();
        for tab in tabs {
            let info = self
                .browser
                .pages
                .get_info_by_tab(tab)
                .await
                .filter(|info| self.playwright.closed_target(&info.target_id).is_none());
            facts.push(match info {
                Some(info) => {
                    json!({ "tab": tab.0, "url": info.url, "title": info.title, "closed": false })
                }
                None => json!({ "tab": tab.0, "url": null, "title": null, "closed": true }),
            });
        }
        facts
    }

    /// The production wire reply, including metadata the real prelude consumes.
    pub(crate) async fn invoke_wire(
        &self,
        name: &str,
        input: Value,
    ) -> Result<Value, PlaywrightError> {
        self.playwright.invoke(&self.env, op(name)?, input).await
    }

    /// Direct Rust fixtures inspect the script value. The JS adapter uses `invoke_wire`
    /// instead so frame metadata and serialized values are unwrapped by the prelude once.
    pub(crate) async fn invoke(&self, name: &str, input: Value) -> Result<Value, PlaywrightError> {
        let result = self.invoke_wire(name, input).await?;
        // The browser tests below inspect the script value. Supplemental frame metadata is
        // consumed by the real prelude and covered by the Node navigation regression suite.
        Ok(if name == "page.evaluate" {
            result.get("value").cloned().unwrap_or(Value::Null)
        } else {
            result
        })
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

    /// Types `text` into the first element matching `selector`, as the owner's keyboard would:
    /// the page focuses it, then the browser inserts the text.
    pub(crate) async fn type_text(
        &self,
        page: &TestPage,
        selector: &str,
        text: &str,
    ) -> TestResult {
        self.eval(
            page,
            &format!("document.querySelector({selector:?}).focus()"),
        )
        .await?;
        self.session(page)
            .await?
            .send_value("Input.insertText", json!({ "text": text }))
            .await?;
        Ok(())
    }

    /// Every recheck from now on is refused with `refusal`, as the owner taking over the tab
    /// would; `None` allows them again.
    pub(crate) fn refuse_rechecks(&self, refusal: Option<Refusal>) {
        *self
            .env
            .refusal
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = refusal;
    }

    /// A background tab on `url`, opened the way the owner or another session opens one, so
    /// core did not create it for the agent. Returns once it loaded.
    pub(crate) async fn open_adopted(&self, url: &str) -> TestResult<TestPage> {
        let created = self
            .browser
            .cdp(
                "Target.createTarget",
                json!({ "url": url, "background": true }),
                None,
            )
            .await?;
        let target = created
            .get("targetId")
            .and_then(Value::as_str)
            .ok_or("no target id")?
            .to_owned();
        let info = self
            .browser
            .pages
            .list()
            .await?
            .into_iter()
            .find(|info| info.target_id.as_str() == target)
            .ok_or("the tab is not listed")?;
        let page = TestPage { tab: info.tab_id };
        for _ in 0..100 {
            if self.eval(&page, "document.readyState").await? == json!("complete") {
                return Ok(page);
            }
            time::sleep(Duration::from_millis(50)).await;
        }
        Err(format!("{url} did not load within 5 s").into())
    }

    /// Evaluates `expression` by value in the main world of `page`'s out-of-process frame whose
    /// URL contains `url_part`, through a session of the test's own: what that frame's page
    /// itself sees, without the crate. The frame is picked from `page`'s own `page.frames`, never
    /// from the browser's whole target list: other tests leave tabs open on the same fixture, and
    /// their frames carry the same URLs. An OOPIF's frame id is its target id.
    pub(crate) async fn frame_eval(
        &self,
        page: &TestPage,
        url_part: &str,
        expression: &str,
    ) -> TestResult<Value> {
        let frames = self
            .invoke("page.frames", json!({ "call": {}, "page": brand(page) }))
            .await?;
        let target = frames
            .get("frames")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|frame| {
                frame
                    .get("url")
                    .and_then(Value::as_str)
                    .is_some_and(|url| url.contains(url_part))
            })
            .and_then(|frame| frame.get("id").cloned())
            .ok_or(format!("no frame on {url_part} in this page"))?;
        // This is an independent observation of the fixture, not an SDK session. Sharing
        // the SDK socket makes its frame initializer adopt our temporary attach, then race
        // its detach; a pending setup reply can stall later iframe startup on that socket.
        let client = Arc::new(CdpClient::connect(ConnectOptions::new(cdp_port())).await?);
        let transport: Arc<dyn CdpConnection> = Arc::<CdpClient>::clone(&client);
        let result: TestResult<Value> = async {
            let attached = transport
                .send(
                    "Target.attachToTarget",
                    json!({ "targetId": target, "flatten": true }),
                    None,
                )
                .await?;
            let session = attached
                .get("sessionId")
                .and_then(Value::as_str)
                .ok_or("no session id")?
                .to_owned();
            let reply = ProtocolSession::for_session(
                Arc::clone(&transport),
                SessionId::from(session.as_str()),
            )
            .send_value(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
            )
            .await;
            let _detached = transport
                .send(
                    "Target.detachFromTarget",
                    json!({ "sessionId": session }),
                    None,
                )
                .await;
            Ok(reply?
                .pointer("/result/value")
                .cloned()
                .unwrap_or(Value::Null))
        }
        .await;
        // Dropping CdpClient does not stop its reader/supervisor tasks. Disconnect on every
        // result path, including a failed attach, before returning the fixture observation.
        client.disconnect().await;
        result
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

    /// Resolves the fixture's tab only at a raw core call; test handles retain no core ID.
    pub(crate) async fn core_page(&self, page: &TestPage) -> TestResult<PageId> {
        Ok(self
            .browser
            .pages
            .get_info_by_tab(page.tab)
            .await
            .ok_or("the fixture tab is not open")?
            .page_id)
    }

    /// The page's attached session, the one core and the crate use.
    async fn session(&self, page: &TestPage) -> TestResult<ProtocolSession> {
        let core_page = self.core_page(page).await?;
        Ok(self.browser.pages.get_session(core_page).await?.session)
    }
}

/// `{"$neo":"page","tab":1234}` for `page`.
pub(crate) fn brand(page: &TestPage) -> Value {
    json!({ "$neo": "page", "tab": page.tab.0 })
}

/// The page a `{"$neo":"page","tab":1234}` handle names.
pub(crate) fn test_page(handle: &Value) -> TestResult<TestPage> {
    if handle.get("$neo").and_then(Value::as_str) != Some("page") {
        return Err(format!("not a page handle: {handle}").into());
    }
    let tab = handle
        .get("tab")
        .and_then(Value::as_i64)
        .ok_or("a page handle carries its tab")?;
    Ok(TestPage { tab: TabId(tab) })
}

/// A value in Playwright's in-page format as plain JSON: arrays and objects unwrapped, a
/// special value as its name, for the assertions.
pub(crate) fn plain(value: Value) -> Value {
    match value {
        Value::Object(mut tagged) => {
            if let Some(Value::Array(items)) = tagged.remove("a") {
                return Value::Array(items.into_iter().map(plain).collect());
            }
            if let Some(Value::Array(members)) = tagged.remove("o") {
                let mut object = Map::new();
                for member in members {
                    let key = member
                        .get("k")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    object.insert(key, plain(member.get("v").cloned().unwrap_or(Value::Null)));
                }
                return Value::Object(object);
            }
            if let Some(special) = tagged.remove("v") {
                return special;
            }
            Value::Object(tagged)
        }
        other => other,
    }
}
