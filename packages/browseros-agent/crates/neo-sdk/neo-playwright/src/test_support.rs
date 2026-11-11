//! A live browser for the crate's own live tests: one CDP connection with a `Playwright` over
//! it, as claw-sdk builds them, tabs it opens and closes, and hooks that record what the
//! sweeps told the host. Needs BrowserOS with CDP on
//! `BROWSEROS_CDP_PORT` (default 9005). Also the scripted browsers unit tests run an op against
//! ([`one_tab`], and [`ScriptedTab`] for ops that install and call a sealed world); they live
//! here because core's `page_js_guard` reads every other non-test file as production code, and
//! the scripts answer `Runtime.evaluate`.

#[path = "../tests/support/fixture_server.rs"]
pub(crate) mod fixture_server;

use std::env;
use std::error::Error;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use browseros_cdp::{CdpClient, CdpError, CdpEvent, ConnectOptions, TargetedEvents};
use browseros_core::sensitive_fields::{FieldValues, JournalEntry};
use browseros_core::testing::TestConnection;
use browseros_core::{
    BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, PageId, ProtocolSession,
    SessionId, TabId, TargetId,
};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};
use tokio::time;

use crate::op::Op;
use crate::probe::PROBE_PROTOCOL;
use crate::target::PageRef;
use crate::test_env::TestEnv;
use crate::{Playwright, PlaywrightHooks, Refusal, TypedBy};

pub(crate) type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// Hooks that remember every value a sweep told the host.
#[derive(Default)]
pub(crate) struct RecordingHooks {
    swept: Mutex<Vec<String>>,
}

impl RecordingHooks {
    /// Every value `values_swept` was told so far, long enough to register, in order.
    pub(crate) fn swept(&self) -> Vec<String> {
        self.swept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PlaywrightHooks for RecordingHooks {
    fn field_entered(&self, _page: TabId, _entry: JournalEntry, _by: TypedBy) {}

    fn values_swept(&self, _page: TabId, values: &FieldValues) {
        self.swept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(values.registrable().map(str::to_owned));
    }

    fn capture_allowed(&self, _page: TabId) -> Result<(), Refusal> {
        Ok(())
    }
}

/// One CDP connection with a `Playwright` over it.
pub(crate) struct LiveBrowser {
    pub(crate) browser: Arc<BrowserSession>,
    pub(crate) playwright: Arc<Playwright>,
}

impl LiveBrowser {
    pub(crate) async fn connect() -> TestResult<Self> {
        let port = env::var("BROWSEROS_CDP_PORT")
            .ok()
            .and_then(|port| port.parse().ok())
            .unwrap_or(9005);
        let client: Arc<dyn CdpConnection> =
            Arc::new(CdpClient::connect(ConnectOptions::new(port)).await?);
        let browser = BrowserSession::new(client, BrowserSessionHooks::default());
        let hooks = Arc::new(RecordingHooks::default());
        let playwright = Playwright::new(
            Arc::clone(&browser),
            Arc::clone(&hooks) as Arc<dyn PlaywrightHooks>,
        );
        // The start-up read of open tabs, so a tab the test opens next counts as new.
        playwright.pages().note_open_tabs(&browser).await;
        Ok(Self {
            browser,
            playwright,
        })
    }

    /// Opens `url` in a new tab and waits for it to load.
    pub(crate) async fn open(&self, url: &str) -> TestResult<Tab> {
        let created = self
            .browser
            .cdp("Target.createTarget", json!({ "url": url }), None)
            .await?;
        let target = TargetId(
            created["targetId"]
                .as_str()
                .ok_or("no target id")?
                .to_owned(),
        );
        let tab = self.tab(&target).await?;
        tab.wait_for(url).await?;
        Ok(tab)
    }

    /// The tab `target` is, as this connection's core numbers it.
    pub(crate) async fn tab(&self, target: &TargetId) -> TestResult<Tab> {
        let info = self
            .browser
            .pages
            .list()
            .await?
            .into_iter()
            .find(|info| info.target_id == *target)
            .ok_or("the tab is not listed")?;
        let session = self.browser.pages.get_session(info.page_id.clone()).await?;
        Ok(Tab {
            target: target.clone(),
            page: info.page_id,
            id: info.tab_id,
            session: session.session,
        })
    }

    /// The host side of one op on this connection: an unbound view, no policies.
    pub(crate) fn env(&self) -> TestEnv {
        TestEnv::over(BrowserView::unbound(Arc::clone(&self.browser), None))
    }

    pub(crate) async fn close(&self, tab: &Tab) {
        let _closed = self
            .browser
            .cdp(
                "Target.closeTarget",
                json!({ "targetId": tab.target.as_str() }),
                None,
            )
            .await;
    }
}

/// A tab a test opened, with core's session for main-world scripts and input.
pub(crate) struct Tab {
    pub(crate) target: TargetId,
    pub(crate) page: PageId,
    /// The browser's tab id, the page brand's `tab`.
    pub(crate) id: TabId,
    pub(crate) session: ProtocolSession,
}

impl Tab {
    /// `{"$neo":"page","tab":1234}`, as a target names this tab.
    pub(crate) fn brand(&self) -> Value {
        json!({ "$neo": "page", "tab": self.id.0 })
    }

    /// The page's own main world, which the seal leaves alone.
    pub(crate) async fn main_world(&self, expression: &str) -> TestResult<Value> {
        let result: Value = self
            .session
            .send(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
            )
            .await?;
        Ok(result["result"]["value"].clone())
    }

    pub(crate) async fn navigate(&self, url: &str) -> TestResult {
        self.session
            .send::<_, Value>("Page.navigate", json!({ "url": url }))
            .await?;
        self.wait_for(url).await
    }

    pub(crate) async fn wait_for(&self, url: &str) -> TestResult {
        let loaded = format!(
            "document.readyState === 'complete' && location.href === {}",
            serde_json::to_string(url)?
        );
        for _ in 0..100 {
            if matches!(self.main_world(&loaded).await, Ok(Value::Bool(true))) {
                return Ok(());
            }
            time::sleep(Duration::from_millis(50)).await;
        }
        Err(format!("{url} did not load within 5 s").into())
    }
}

/// The op live tests open their scopes with; any read op works.
pub(crate) fn count_op() -> TestResult<Op> {
    Ok(Op::from_name("locator.count").ok_or("locator.count is an op")?)
}

/// The scripted browser [`one_tab`] builds, and its one tab.
pub(crate) struct OneTab {
    pub(crate) browser: Arc<BrowserSession>,
}

/// A browser over `connection` with one tab, tab 7 on `https://example.test/`, scripted to
/// answer what admitting the tab and preparing a capture send. A test scripts the capture
/// itself.
pub(crate) async fn one_tab(connection: &Arc<TestConnection>) -> Result<OneTab, Box<dyn Error>> {
    connection.always(
        "Browser.getTabs",
        json!({ "tabs": [{ "tabId": 7, "targetId": "target-7", "windowId": 1,
            "url": "https://example.test/", "title": "Account", "isActive": true,
            "isLoading": false, "loadProgress": 1.0, "isPinned": false, "isHidden": false }] }),
    )?;
    for setup in [
        "Page.enable",
        "DOM.enable",
        "Runtime.enable",
        "Accessibility.enable",
    ] {
        connection.always(setup, json!({}))?;
    }
    connection.always("Runtime.evaluate", json!({ "result": { "value": true } }))?;
    let cdp: Arc<dyn CdpConnection> = Arc::clone(connection) as Arc<dyn CdpConnection>;
    let browser = BrowserSession::new(cdp, BrowserSessionHooks::default());
    browser
        .pages
        .list()
        .await?
        .first()
        .ok_or("the tab is listed")?;
    Ok(OneTab { browser })
}

/// What a [`ScriptedTab`] answers one probe request with, after `after` on the tokio clock.
pub(crate) struct ProbeAnswer {
    pub(crate) after: Duration,
    /// The probe's reply object, or the protocol error Chromium fails the call with.
    pub(crate) reply: Result<Value, &'static str>,
}

impl ProbeAnswer {
    /// `reply`, at once.
    pub(crate) fn reply(reply: Value) -> Self {
        Self {
            after: Duration::ZERO,
            reply: Ok(reply),
        }
    }

    /// The call fails at once with Chromium's protocol error `message`.
    pub(crate) fn error(message: &'static str) -> Self {
        Self {
            after: Duration::ZERO,
            reply: Err(message),
        }
    }

    /// A sweep of a document that holds no sensitive field.
    pub(crate) fn empty_sweep() -> Self {
        Self::reply(json!({ "kind": "sweep", "values": [] }))
    }
}

/// A tab an opener link connects to a [`ScriptedTab`], as the browser lists it.
#[derive(Clone, Copy)]
pub(crate) struct ScriptedLink {
    /// Its page target, which the browser's target list names with the scripted tab as its
    /// `openerId`.
    pub(crate) target: &'static str,
    /// Its URL, which decides whether a sweep of the scripted tab's site reads it.
    pub(crate) url: &'static str,
    /// Its tab id in `Browser.getTabs`, `None` when the browser lists no tab for it.
    pub(crate) tab: Option<i64>,
}

/// How a test's [`ScriptedTab`] answers the probe.
pub(crate) trait TabScript: Send + Sync + 'static {
    /// The answer to `request`, one probe request as the crate encoded it (`{"op": ...}`), in
    /// the world of `frame`: [`SCRIPTED_MAIN`], or [`SCRIPTED_CHILD`] or [`SCRIPTED_OOPIF`]
    /// when the tab has it.
    fn probe(&self, frame: &str, request: &Value) -> ProbeAnswer;

    /// Whether the main frame holds [`SCRIPTED_CHILD`], a frame of its site in its process.
    fn child(&self) -> bool {
        false
    }

    /// Whether the main frame holds [`SCRIPTED_OOPIF`], a frame of its site in a process of its
    /// own, which the browser lists as a target of its own.
    fn oopif(&self) -> bool {
        false
    }

    /// A tab an opener link connects to this one, for every list of targets and tabs from now
    /// on; none by default.
    fn linked(&self) -> Option<ScriptedLink> {
        None
    }

    /// How long the tab takes to answer `method`, any command but a probe request: a renderer
    /// slow to scan a new world's document, say.
    fn delay(&self, _method: &str) -> Duration {
        Duration::ZERO
    }

    /// Whether the tab answers this `method` only after the CDP client gave up on it: the call
    /// then fails as the client fails it, `CdpError::Timeout` once [`client_timeout`] passed,
    /// and the late answer reaches no one. Asked once for every command.
    fn outlasts_client(&self, _method: &str) -> bool {
        false
    }

    /// Whether the tab's main frame has left for a document no world can be made in yet: an
    /// install then fails as for a frame that is gone.
    fn left(&self) -> bool {
        false
    }

    /// The reply to one call into the page's main world, `function` called on `object`: the
    /// main frame's document as `DOM.resolveNode` names it ([`SCRIPTED_DOCUMENT`]), or an object
    /// an earlier reply handed out, such as a poller. `Err` is the protocol error Chromium fails
    /// the call with. `None` answers as for any other object the tab holds.
    fn main_world(&self, _object: &str, _function: &str) -> Option<Result<Value, &'static str>> {
        None
    }
}

/// The main frame's document in its main world, as the scripted tab's `DOM.resolveNode` names
/// it.
pub(crate) const SCRIPTED_DOCUMENT: &str = "main-doc";

/// The CDP client's own request timeout, browseros-cdp's default: a command unanswered by then
/// fails `CdpError::Timeout`.
pub(crate) fn client_timeout() -> Duration {
    ConnectOptions::default().request_timeout
}

/// The scripted tab's target id, which is also its main frame's id.
pub(crate) const SCRIPTED_MAIN: &str = "scripted-target";

/// The scripted tab's child frame, on `https://a.test/child`.
pub(crate) const SCRIPTED_CHILD: &str = "scripted-child";

/// The scripted tab's out-of-process frame, on `https://pay.a.test/`: its target id, which is
/// also its frame's id.
pub(crate) const SCRIPTED_OOPIF: &str = "scripted-oopif";

/// Numbers the scripted tabs' sessions: core serializes captures by session id across the
/// process, so no two tabs may share one.
static SCRIPTED_SESSIONS: AtomicU64 = AtomicU64::new(0);

/// A scripted browser with one tab, tab 7 on `https://a.test/`, and a same-site child frame in
/// its process, or one in a process of its own, when the script asks for one. It answers what
/// admitting the tab, attaching its out-of-process frame, installing a frame's sealed world and
/// listing its frames and targets send, as for a page with no closed shadow root, and hands
/// every probe request, a sweep included, to its [`TabScript`].
pub(crate) struct ScriptedTab<S> {
    script: S,
    session: String,
    /// `Browser.createTab` opened tab 9, which `Browser.getTabs` lists from then on.
    created: AtomicBool,
    /// Commands the script slowed ([`TabScript::delay`]) and that have not been answered or
    /// dropped yet.
    slowed: AtomicUsize,
    /// Every command sent, by method, in order.
    sent: Mutex<Vec<String>>,
    events: broadcast::Sender<CdpEvent>,
    targeted: TargetedEvents,
}

/// The tab `Browser.createTab` opens on a [`ScriptedTab`]: tab 9, in the background.
pub(crate) const SCRIPTED_NEW: &str = "scripted-new";

/// [`SCRIPTED_NEW`]'s tab, as the browser describes it.
fn created_tab() -> Value {
    json!({ "tabId": 9, "targetId": SCRIPTED_NEW, "windowId": 1, "url": "about:blank",
        "title": "", "isActive": false, "isLoading": false, "loadProgress": 1.0,
        "isPinned": false, "isHidden": false })
}

/// One slowed command in flight: counted while it waits, uncounted when answered or dropped.
struct Slowed<'a>(&'a AtomicUsize);

impl<'a> Slowed<'a> {
    fn new(count: &'a AtomicUsize) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Self(count)
    }
}

impl Drop for Slowed<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl<S: TabScript> ScriptedTab<S> {
    pub(crate) fn new(script: S) -> Arc<Self> {
        Arc::new(Self {
            script,
            session: format!(
                "scripted-session-{}",
                SCRIPTED_SESSIONS.fetch_add(1, Ordering::Relaxed)
            ),
            created: AtomicBool::new(false),
            slowed: AtomicUsize::new(0),
            sent: Mutex::default(),
            events: broadcast::channel(16).0,
            targeted: TargetedEvents::default(),
        })
    }

    pub(crate) fn script(&self) -> &S {
        &self.script
    }

    /// How many commands the script slowed are still waiting for their answer.
    pub(crate) fn slowed_in_flight(&self) -> usize {
        self.slowed.load(Ordering::SeqCst)
    }

    /// How many `method` commands were sent so far.
    pub(crate) fn sent(&self, method: &str) -> usize {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|sent| *sent == method)
            .count()
    }

    /// Sends the browser event `method` with `params`, as Chromium sends it on the root session.
    pub(crate) fn emit(&self, method: &str, params: Value) {
        let event = CdpEvent {
            method: method.to_owned(),
            params,
            session_id: None,
        };
        if let Some(event) = self.targeted.deliver(event) {
            let _ = self.events.send(event);
        }
    }

    /// `Browser.getTabs`: the main tab, then the one `Browser.createTab` opened, then the
    /// linked tab when the browser lists a tab for it.
    fn tabs(&self, main_tab: Value, linked: Option<ScriptedLink>) -> Value {
        let mut tabs = vec![main_tab];
        if self.created.load(Ordering::SeqCst) {
            tabs.push(created_tab());
        }
        if let Some(ScriptedLink {
            target,
            url,
            tab: Some(tab),
        }) = linked
        {
            tabs.push(
                json!({ "tabId": tab, "targetId": target, "windowId": 1, "url": url,
                "title": "", "isActive": false, "isLoading": false, "loadProgress": 1.0,
                "isPinned": false, "isHidden": false }),
            );
        }
        json!({ "tabs": tabs })
    }

    /// `Target.getTargets`: the main frame's target, the out-of-process frame's when the tab
    /// has one, and the linked tab's, opened by the main tab.
    fn targets(&self, linked: Option<ScriptedLink>) -> Value {
        let mut listed = vec![json!({ "targetId": SCRIPTED_MAIN, "type": "page",
            "url": "https://a.test/" })];
        if self.script.oopif() {
            listed.push(json!({ "targetId": SCRIPTED_OOPIF, "type": "iframe",
                "url": "https://pay.a.test/", "parentId": SCRIPTED_MAIN }));
        }
        if let Some(link) = linked {
            listed.push(
                json!({ "targetId": link.target, "type": "page", "url": link.url,
                "openerId": SCRIPTED_MAIN }),
            );
        }
        json!({ "targetInfos": listed })
    }

    /// The session core attaches to `target`: one per target, as Chromium hands them out.
    fn session_of(&self, target: &str) -> String {
        format!("{}-{target}", self.session)
    }

    async fn answer(
        &self,
        method: &str,
        params: &Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CdpError> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(method.to_owned());
        if self.script.outlasts_client(method) {
            let _slowed = Slowed::new(&self.slowed);
            time::sleep(client_timeout()).await;
            return Err(CdpError::Timeout {
                method: method.to_owned(),
            });
        }
        let delay = self.script.delay(method);
        if !delay.is_zero() {
            let _slowed = Slowed::new(&self.slowed);
            time::sleep(delay).await;
        }
        let on_oopif = session.is_some_and(|session| {
            self.script.oopif() && session.as_str() == self.session_of(SCRIPTED_OOPIF)
        });
        // Each frame's world: its context id, and its global's object id, which names it.
        let child = |frame: &Value| frame.as_str() == Some(SCRIPTED_CHILD);
        let oopif = |frame: &Value| frame.as_str() == Some(SCRIPTED_OOPIF);
        let main_tab = json!({ "tabId": 7, "targetId": SCRIPTED_MAIN, "windowId": 1,
            "url": "https://a.test/", "title": "A", "isActive": true, "isLoading": false,
            "loadProgress": 1.0, "isPinned": false, "isHidden": false });
        let linked = self.script.linked();
        Ok(match method {
            "Browser.getTabs" => self.tabs(main_tab, linked),
            "Browser.createTab" => {
                self.created.store(true, Ordering::SeqCst);
                json!({ "tab": created_tab() })
            }
            "Browser.getTabInfo" if params["tabId"] == 9 => json!({ "tab": created_tab() }),
            "Browser.getTabInfo" => json!({ "tab": main_tab }),
            "Target.attachToTarget" => {
                let target = params["targetId"].as_str().unwrap_or_default();
                json!({ "sessionId": self.session_of(target) })
            }
            "Target.getTargets" => self.targets(linked),
            "Page.getFrameTree" if on_oopif => {
                json!({ "frameTree": { "frame": { "id": SCRIPTED_OOPIF, "loaderId": "LO",
                    "url": "https://pay.a.test/", "domainAndRegistry": "a.test" } } })
            }
            "Page.getFrameTree" => {
                let children: Vec<Value> = self
                    .script
                    .child()
                    .then(|| {
                        json!({ "frame": { "id": SCRIPTED_CHILD, "parentId": SCRIPTED_MAIN,
                            "loaderId": "LC", "url": "https://a.test/child",
                            "domainAndRegistry": "a.test" } })
                    })
                    .into_iter()
                    .collect();
                json!({ "frameTree": { "frame": { "id": SCRIPTED_MAIN, "loaderId": "L",
                    "url": "https://a.test/", "domainAndRegistry": "a.test" },
                    "childFrames": children } })
            }
            "Page.createIsolatedWorld" if child(&params["frameId"]) => {
                json!({ "executionContextId": 8 })
            }
            "Page.createIsolatedWorld" if oopif(&params["frameId"]) => {
                json!({ "executionContextId": 9 })
            }
            "Page.createIsolatedWorld" if self.script.left() => {
                return Err(protocol_error("No frame for given id found"));
            }
            "Page.createIsolatedWorld" => json!({ "executionContextId": 7 }),
            "Runtime.evaluate" if params["contextId"] == 8 => {
                json!({ "result": { "type": "object", "objectId": "g-child" } })
            }
            "Runtime.evaluate" if params["contextId"] == 9 => {
                json!({ "result": { "type": "object", "objectId": "g-oopif" } })
            }
            "Runtime.evaluate" => json!({ "result": { "type": "object", "objectId": "g" } }),
            // The hidden-root scan finds no shadow root.
            "DOM.describeNode" => json!({ "node": { "backendNodeId": 1,
                "nodeName": "#document", "childNodeCount": 0, "children": [] } }),
            // The document's node in its main world, where a page script runs.
            "DOM.resolveNode" => json!({ "object": { "type": "object", "subtype": "node",
                "objectId": SCRIPTED_DOCUMENT } }),
            "Runtime.callFunctionOn" => return self.call(params).await,
            _ => json!({}),
        })
    }

    /// One `Runtime.callFunctionOn`: a probe request goes to the script, and the install's
    /// boot, the seal's watch and the world's document answer as for a page with no closed
    /// shadow root.
    async fn call(&self, params: &Value) -> Result<Value, CdpError> {
        let first = &params["arguments"][0]["value"];
        if first.get("op").is_some() {
            let frame = match params["objectId"].as_str() {
                Some("g-child") => SCRIPTED_CHILD,
                Some("g-oopif") => SCRIPTED_OOPIF,
                _ => SCRIPTED_MAIN,
            };
            let ProbeAnswer { after, reply } = self.script.probe(frame, first);
            time::sleep(after).await;
            return reply
                .map(|reply| json!({ "result": { "type": "object", "value": reply } }))
                .map_err(protocol_error);
        }
        if first.get("frameSeq").is_some() {
            return Ok(
                json!({ "result": { "type": "object", "value": { "kind": "described",
                "protocol": PROBE_PROTOCOL, "seq": first["frameSeq"],
                "url": "https://a.test/" } } }),
            );
        }
        let function = params["functionDeclaration"].as_str().unwrap_or_default();
        if let Some(answer) = params["objectId"]
            .as_str()
            .and_then(|object| self.script.main_world(object, function))
        {
            return answer.map_err(protocol_error);
        }
        Ok(if function.contains("takeHost") {
            // The seal reported no shadow host.
            json!({ "result": { "type": "object", "subtype": "null", "value": null } })
        } else if function.contains("detachedTops") {
            // It watches no tree out of the document.
            json!({ "result": { "type": "object", "subtype": "array", "objectId": "tops",
                "description": "Array(0)" } })
        } else {
            // The world's document, where the scan starts.
            json!({ "result": { "type": "object", "objectId": "d" } })
        })
    }
}

/// Chromium's failure of a call, with `message`.
fn protocol_error(message: &str) -> CdpError {
    CdpError::Protocol {
        code: -32000,
        message: message.to_owned(),
    }
}

impl<S: TabScript> CdpConnection for ScriptedTab<S> {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move { self.answer(method, &params, session).await })
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            let params = serde_json::from_str(params_json)?;
            let reply = self.answer(method, &params, session).await?;
            Ok(serde_json::to_string(&reply)?)
        })
    }

    // Core sends a creation's reads at the connection's epoch, which never changes here.
    fn send_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        if expected_epoch != 1 {
            return Box::pin(async move {
                Err(CdpError::ConnectionEpochChanged {
                    expected: expected_epoch,
                    actual: 1,
                })
            });
        }
        self.send(method, params, session)
    }

    fn send_raw_json_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        if expected_epoch != 1 {
            return Box::pin(async move {
                Err(CdpError::ConnectionEpochChanged {
                    expected: expected_epoch,
                    actual: 1,
                })
            });
        }
        self.send_raw_json(method, params_json, session)
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }

    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        self.targeted.subscribe(method)
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn connection_epoch(&self) -> u64 {
        1
    }
}

/// A `Playwright` over a [`ScriptedTab`], the hooks it reports to, the host's side of its ops
/// and the tab's brand.
pub(crate) struct ScriptedRun {
    pub(crate) playwright: Arc<Playwright>,
    pub(crate) hooks: Arc<RecordingHooks>,
    pub(crate) env: TestEnv,
    pub(crate) page: PageRef,
}

impl ScriptedRun {
    /// A run over `tab`, whose one tab it lists first.
    pub(crate) async fn over<S: TabScript>(tab: &Arc<ScriptedTab<S>>) -> TestResult<Self> {
        let connection: Arc<dyn CdpConnection> = Arc::clone(tab) as Arc<dyn CdpConnection>;
        let browser = BrowserSession::new(connection, BrowserSessionHooks::default());
        let hooks = Arc::new(RecordingHooks::default());
        let playwright = Playwright::new(
            Arc::clone(&browser),
            Arc::clone(&hooks) as Arc<dyn PlaywrightHooks>,
        );
        let listed = browser.pages.list().await?;
        let listed = listed.first().ok_or("the tab is listed")?;
        let page = serde_json::from_value(json!({ "$neo": "page", "tab": listed.tab_id }))?;
        Ok(Self {
            playwright,
            hooks,
            env: TestEnv::over(BrowserView::unbound(browser, None)),
            page,
        })
    }
}
