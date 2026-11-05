//! A fake Chromium for the fit, with state the fit's calls change and read back: tabs with
//! histories and openers in one window, the window's state (openbox maximizes a normal window,
//! and Chromium refuses `maximized` on a fullscreen one), each page's emulation, the active
//! tab, and the boxes of a focused field. A page that is not emulated shows Neo's screen as
//! the fake Xvnc has it, less 87 px of browser chrome unless the window is fullscreen.
//!
//! It is not core's scripted `TestConnection`: canned replies per method cannot model a window
//! whose state changes after `setWindowBounds`, or tell core's viewport readback from the owner
//! view's script, which are both `Runtime.evaluate`.
//!
//! Every call is recorded with the number of screen resizes the fake Xvnc had taken by then,
//! so a test reads the order of browser and screen changes from one list.
//!
//! A test can make an address unreachable, and can hold every navigation until it lets them go,
//! to catch work that is still moving a tab.
//!
//! It also answers what the control journal and the sweep send, for the takeover flow's tests:
//! each new-document script gets its own identifier, the seal installs, and a sweep's read of a
//! frame's fields answers what the test put in them. A page can have child frames, for a walk
//! that takes a while. Those reads can be held, any method can be held until the test lets it go
//! or made to hang, and a probe the test sets is read at each call and kept with it.

use browseros_cdp::{CdpError, CdpEvent, SessionId};
use browseros_core::CdpConnection;
use browseros_core::sensitive_fields::seal_source;
use browseros_takeover::testing::FakeXvnc;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::sync::{broadcast, watch};

/// Session ids are unique across every fake in the process: core keys its capture lock by
/// session id, so two tests' `session-1` would wait on each other.
static SESSIONS: AtomicU64 = AtomicU64::new(0);

/// How tall the browser's toolbar is in a window that is not fullscreen.
pub const CHROME_HEIGHT: u16 = 87;

/// The seal's read of a frame's sensitive fields, which only a sweep sends.
pub const SWEEP_READ: &str = "__clawSeal.sensitiveValues()";

/// Read at each call; what it returns is kept with the call.
type Probe = Box<dyn Fn() -> bool + Send + Sync>;

pub struct FakeBrowser {
    xvnc: Arc<FakeXvnc>,
    state: Mutex<State>,
    events: broadcast::Sender<CdpEvent>,
    /// `false` while a test holds navigations: each `Page.navigate` then waits, subscribed, until
    /// it is `true` again.
    navigations: watch::Sender<bool>,
    /// The same, for a sweep's reads of a frame's fields.
    sweep_reads: watch::Sender<bool>,
    /// The same, per method a test held with [`Self::hold`].
    holds: Mutex<HashMap<String, watch::Sender<bool>>>,
    probe: Mutex<Option<Probe>>,
}

/// One command as the fake received it.
#[derive(Clone, Debug)]
pub struct Call {
    pub method: String,
    pub params: Value,
    /// Screen resizes the fake Xvnc had received when this call arrived.
    pub resizes: usize,
    /// The tab whose session the call came on.
    pub tab: Option<i64>,
    /// What the test's probe read when the call arrived, if it set one.
    pub probed: Option<bool>,
}

/// What a page reports about its viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub width: u16,
    pub height: u16,
    pub device_scale: f64,
    pub touch: bool,
}

struct Tab {
    id: i64,
    /// `(url, transitionType)` per history entry.
    history: Vec<(String, String)>,
    index: usize,
    opener: Option<i64>,
}

#[derive(Default)]
struct Emulation {
    metrics: Option<Value>,
    touch: bool,
}

struct State {
    tabs: Vec<Tab>,
    window: &'static str,
    /// Session id to the tab it is attached to.
    sessions: HashMap<String, i64>,
    emulation: HashMap<i64, Emulation>,
    /// What the page with focus reports as its fields' boxes.
    boxes: Vec<[f64; 4]>,
    /// Addresses a navigation fails to load.
    unreachable: Vec<String>,
    /// What every frame's password, card and code fields hold, as the seal reports it to a sweep.
    fields: Vec<String>,
    /// Methods whose calls never answer.
    hung: Vec<String>,
    /// How long a sweep's read of a frame's fields takes.
    sweep_read_time: Duration,
    /// How many child frames each page's frame tree has below its own.
    child_frames: usize,
    /// New-document scripts added so far, which numbers each one's identifier.
    scripts: usize,
    calls: Vec<Call>,
}

fn target(tab: i64) -> String {
    format!("target-{tab}")
}

impl FakeBrowser {
    /// Tabs with these ids, each at `https://example.test/<id>`, in one maximized window.
    pub fn new(xvnc: Arc<FakeXvnc>, tabs: &[i64]) -> Arc<Self> {
        Arc::new(Self {
            xvnc,
            state: Mutex::new(State {
                tabs: tabs
                    .iter()
                    .map(|id| Tab {
                        id: *id,
                        history: vec![(format!("https://example.test/{id}"), "typed".into())],
                        index: 0,
                        opener: None,
                    })
                    .collect(),
                window: "maximized",
                sessions: HashMap::new(),
                emulation: HashMap::new(),
                boxes: Vec::new(),
                unreachable: Vec::new(),
                fields: Vec::new(),
                hung: Vec::new(),
                sweep_read_time: Duration::ZERO,
                child_frames: 0,
                scripts: 0,
                calls: Vec::new(),
            }),
            events: broadcast::channel(16).0,
            navigations: watch::channel(true).0,
            sweep_reads: watch::channel(true).0,
            holds: Mutex::default(),
            probe: Mutex::default(),
        })
    }

    /// Each page has `count` child frames from now on, below its own frame. The journal and the
    /// sweep put the sealed world in every one, one after another.
    pub fn add_child_frames(&self, count: usize) {
        self.state().child_frames = count;
    }

    /// Calls of `method` wait from now on, unanswered, until [`Self::let_go`]. A call is recorded
    /// once answered, so one whose caller gave up first, as a frame step past its 2 s budget
    /// does, is never recorded.
    pub fn hold(&self, method: &str) {
        self.holds()
            .entry(method.to_owned())
            .or_insert_with(|| watch::channel(true).0)
            .send_replace(false);
    }

    /// Answers every held call of `method`, and every later one.
    pub fn let_go(&self, method: &str) {
        if let Some(gate) = self.holds().get(method) {
            gate.send_replace(true);
        }
    }

    /// How many calls of `method` are waiting to be answered.
    pub fn held(&self, method: &str) -> usize {
        self.holds()
            .get(method)
            .map_or(0, watch::Sender::receiver_count)
    }

    fn holds(&self) -> MutexGuard<'_, HashMap<String, watch::Sender<bool>>> {
        self.holds.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The owner typed these into sensitive fields: every frame's sweep reads them from now on.
    pub fn type_into_fields(&self, values: &[&str]) {
        self.state().fields = values.iter().map(|value| (*value).to_owned()).collect();
    }

    /// Holds every sweep's read of a frame's fields from now on, until
    /// [`Self::let_sweep_reads_go`].
    pub fn hold_sweep_reads(&self) {
        self.sweep_reads.send_replace(false);
    }

    pub fn let_sweep_reads_go(&self) {
        self.sweep_reads.send_replace(true);
    }

    /// How many sweep reads are waiting to be answered.
    pub fn held_sweep_reads(&self) -> usize {
        self.sweep_reads.receiver_count()
    }

    /// Each sweep's read of a frame's fields takes `time` from now on, as Chromium's does, so a
    /// sweep that was waited for can be told from one that was not.
    pub fn slow_sweep_reads(&self, time: Duration) {
        self.state().sweep_read_time = time;
    }

    /// Calls of `method` never answer from now on.
    pub fn hang(&self, method: &str) {
        self.state().hung.push(method.to_string());
    }

    /// Reads `probe` at each call from now on and keeps the answer with the call.
    pub fn probe_each_call(&self, probe: impl Fn() -> bool + Send + Sync + 'static) {
        *self.probe.lock().unwrap_or_else(PoisonError::into_inner) = Some(Box::new(probe));
    }

    /// Every call so far, in the order they arrived.
    pub fn calls(&self) -> Vec<Call> {
        self.state().calls.clone()
    }

    /// Holds every `Page.navigate` from now on, until [`Self::release_navigations`].
    pub fn hold_navigations(&self) {
        self.navigations.send_replace(false);
    }

    /// Lets every held `Page.navigate` go ahead, and every later one.
    pub fn release_navigations(&self) {
        self.navigations.send_replace(true);
    }

    /// How many `Page.navigate` calls are waiting to go ahead.
    pub fn held_navigations(&self) -> usize {
        self.navigations.receiver_count()
    }

    /// A navigation to `url` fails as one to an unknown host does: `Page.navigate` answers with
    /// an `errorText`, and the page stays where it was.
    pub fn unreachable(&self, url: &str) {
        self.state().unreachable.push(url.to_string());
    }

    /// The window's state now.
    pub fn window(&self) -> &'static str {
        self.state().window
    }

    /// What tab `tab`'s page reports, as core's viewport readback reads it.
    pub fn metrics(&self, tab: i64) -> Metrics {
        let state = self.state();
        let value = self.metrics_value(&state, tab);
        let side = |key: &str| {
            value[key]
                .as_u64()
                .and_then(|side| u16::try_from(side).ok())
                .unwrap_or_default()
        };
        Metrics {
            width: side("width"),
            height: side("height"),
            device_scale: value["deviceScaleFactor"].as_f64().unwrap_or_default(),
            touch: value["touchPoints"].as_i64().unwrap_or_default() > 0,
        }
    }

    /// Tab `tab`'s current URL.
    pub fn url(&self, tab: i64) -> Option<String> {
        let state = self.state();
        let tab = state.tabs.iter().find(|open| open.id == tab)?;
        tab.history.get(tab.index).map(|(url, _)| url.clone())
    }

    /// Every call so far that changes something, as `<method> [<detail>] after <n> resizes`.
    pub fn changes(&self) -> Vec<String> {
        self.state()
            .calls
            .iter()
            .filter_map(|call| {
                let detail = match call.method.as_str() {
                    "Browser.activateTab" => format!("tab {}", call.params["tabId"]),
                    "Input.dispatchKeyEvent" => {
                        format!(
                            "{} {}",
                            call.params["type"].as_str()?,
                            call.params["code"].as_str()?
                        )
                    }
                    "Browser.setWindowBounds" => {
                        call.params["bounds"]["windowState"].as_str()?.to_string()
                    }
                    "Emulation.setDeviceMetricsOverride" => {
                        format!("{}x{}", call.params["width"], call.params["height"])
                    }
                    "Emulation.clearDeviceMetricsOverride" => String::new(),
                    "Page.navigate" => call.params["url"].as_str()?.to_string(),
                    "Browser.closeTab" => format!("tab {}", call.params["tabId"]),
                    _ => return None,
                };
                Some(format!("{} {detail} after {}", call.method, call.resizes))
            })
            .collect()
    }

    /// Calls of `method` so far.
    pub fn calls_of(&self, method: &str) -> Vec<Call> {
        self.state()
            .calls
            .iter()
            .filter(|call| call.method == method)
            .cloned()
            .collect()
    }

    /// The page with focus reports these boxes for its text fields.
    pub fn focus_fields(&self, boxes: &[[f64; 4]]) {
        self.state().boxes = boxes.to_vec();
    }

    /// Tab `tab` navigates to `url` by `transition` (`link`, `form_submit`, ...).
    pub fn navigate(&self, tab: i64, url: &str, transition: &str) {
        if let Some(tab) = self.state().tabs.iter_mut().find(|open| open.id == tab) {
            tab.history.truncate(tab.index + 1);
            tab.history.push((url.to_string(), transition.to_string()));
            tab.index = tab.history.len() - 1;
        }
    }

    /// Tab `opener` opens tab `tab` at `url`, reached by `transition`.
    pub fn open_popup(&self, opener: i64, tab: i64, url: &str, transition: &str) {
        self.state().tabs.push(Tab {
            id: tab,
            history: vec![(url.to_string(), transition.to_string())],
            index: 0,
            opener: Some(opener),
        });
    }

    pub fn close_tab(&self, tab: i64) {
        self.state().tabs.retain(|open| open.id != tab);
    }

    pub fn is_open(&self, tab: i64) -> bool {
        self.state().tabs.iter().any(|open| open.id == tab)
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn tab_json(tab: &Tab) -> Value {
        json!({
            "tabId": tab.id,
            "targetId": target(tab.id),
            "windowId": 1,
            "url": tab.history.get(tab.index).map(|(url, _)| url.clone()).unwrap_or_default(),
            "title": format!("Tab {}", tab.id),
            "isActive": false,
            "isLoading": false,
            "loadProgress": 1.0,
            "isPinned": false,
            "isHidden": false,
        })
    }

    /// Core's metrics expression, answered from the tab's emulation, or from Neo's screen and
    /// the window when it has none.
    fn metrics_value(&self, state: &State, tab: i64) -> Value {
        let emulation = state.emulation.get(&tab);
        match emulation.and_then(|emulation| emulation.metrics.as_ref()) {
            Some(metrics) => json!({
                "width": metrics["width"],
                "height": metrics["height"],
                "screenWidth": metrics["screenWidth"],
                "screenHeight": metrics["screenHeight"],
                "deviceScaleFactor": metrics["deviceScaleFactor"],
                "touchPoints": if emulation.is_some_and(|emulation| emulation.touch) { 5 } else { 0 },
                "mobile": metrics["mobile"],
            }),
            None => {
                let screen = self.xvnc.size();
                let chrome = if state.window == "fullscreen" {
                    0
                } else {
                    CHROME_HEIGHT
                };
                json!({
                    "width": screen.width,
                    "height": screen.height.saturating_sub(chrome),
                    "screenWidth": screen.width,
                    "screenHeight": screen.height,
                    "deviceScaleFactor": 1,
                    "touchPoints": 0,
                    "mobile": false,
                })
            }
        }
    }

    fn reply(
        &self,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CdpError> {
        let mut state = self.state();
        let tab = session.and_then(|session| state.sessions.get(session.as_str()).copied());
        let probed = self
            .probe
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|probe| probe());
        state.calls.push(Call {
            method: method.to_string(),
            params: params.clone(),
            resizes: self.xvnc.resizes().len(),
            tab,
            probed,
        });
        let gone = || CdpError::Protocol {
            code: -32000,
            message: "No tab with given id".into(),
        };
        let reply = match method {
            "Browser.getTabs" => {
                let tabs = state.tabs.iter().map(Self::tab_json).collect::<Vec<_>>();
                json!({ "tabs": tabs })
            }
            "Browser.getTabInfo" => {
                let found = state
                    .tabs
                    .iter()
                    .find(|tab| params["tabId"] == tab.id)
                    .ok_or_else(gone)?;
                json!({ "tab": Self::tab_json(found) })
            }
            "Target.attachToTarget" => {
                let id = format!(
                    "fit-session-{}",
                    SESSIONS.fetch_add(1, Ordering::Relaxed) + 1
                );
                let attached = state
                    .tabs
                    .iter()
                    .find(|tab| params["targetId"] == target(tab.id))
                    .map(|tab| tab.id)
                    .ok_or_else(gone)?;
                state.sessions.insert(id.clone(), attached);
                json!({ "sessionId": id })
            }
            "Page.enable"
            | "DOM.enable"
            | "Runtime.enable"
            | "Accessibility.enable"
            | "Runtime.runIfWaitingForDebugger"
            | "Target.setAutoAttach"
            | "Emulation.setFocusEmulationEnabled"
            | "Emulation.setEmitTouchEventsForMouse"
            | "Emulation.setUserAgentOverride"
            | "Emulation.setVisibleSize"
            | "Input.dispatchKeyEvent"
            | "Page.reload" => json!({}),
            "Browser.getVersion" => json!({ "product": "Chrome/155.0.8315.26" }),
            "Emulation.setDeviceMetricsOverride" => {
                let tab = tab.ok_or(CdpError::SessionGone)?;
                state.emulation.entry(tab).or_default().metrics = Some(params);
                json!({})
            }
            "Emulation.clearDeviceMetricsOverride" => {
                let tab = tab.ok_or(CdpError::SessionGone)?;
                state.emulation.entry(tab).or_default().metrics = None;
                json!({})
            }
            "Emulation.setTouchEmulationEnabled" => {
                let tab = tab.ok_or(CdpError::SessionGone)?;
                state.emulation.entry(tab).or_default().touch = params["enabled"] == true;
                json!({})
            }
            "Page.getFrameTree" => {
                let tab = tab.unwrap_or_default();
                let children = (1..=state.child_frames)
                    .map(|child| json!({ "frame": { "id": format!("frame-{tab}-{child}") } }))
                    .collect::<Vec<_>>();
                json!({ "frameTree": { "frame": { "id": format!("frame-{tab}") }, "childFrames": children } })
            }
            "Page.createIsolatedWorld" => json!({ "executionContextId": 77 }),
            "Runtime.addBinding" | "Page.removeScriptToEvaluateOnNewDocument" => json!({}),
            "Page.addScriptToEvaluateOnNewDocument" => {
                state.scripts += 1;
                json!({ "identifier": format!("script-{}", state.scripts) })
            }
            "DOM.getDocument" => json!({ "root": { "nodeName": "#document" } }),
            "Runtime.evaluate" => {
                let tab = tab.ok_or(CdpError::SessionGone)?;
                let expression = params["expression"].as_str().unwrap_or_default();
                let value = if expression == seal_source() {
                    // The seal reports its version once it is in place.
                    json!(1)
                } else if expression == SWEEP_READ {
                    json!(state.fields)
                } else if expression.contains("screenWidth") {
                    self.metrics_value(&state, tab)
                } else if expression.contains("visualViewport") {
                    let metrics = self.metrics_value(&state, tab);
                    json!({
                        "boxes": state.boxes,
                        "screen": [metrics["screenWidth"], metrics["screenHeight"]],
                        "touch": metrics["touchPoints"].as_i64().unwrap_or_default() > 0,
                    })
                } else {
                    // Core's readiness probe after a navigation: the page is showing and still.
                    json!({ "n": 1, "installed": false, "showing": true, "complete": true })
                };
                json!({ "result": { "type": "object", "value": value } })
            }
            "Browser.activateTab" => json!({}),
            "Browser.getWindowForTarget" => {
                json!({ "windowId": 1, "bounds": { "windowState": state.window } })
            }
            "Browser.getWindowBounds" => json!({ "bounds": { "windowState": state.window } }),
            "Browser.getWindows" => {
                json!({ "windows": [{ "windowId": 1, "bounds": { "windowState": state.window } }] })
            }
            "Browser.setWindowBounds" => {
                state.window = match params["bounds"]["windowState"].as_str() {
                    Some("fullscreen") => "fullscreen",
                    // Openbox maximizes every normal window.
                    Some("normal" | "maximized") if state.window != "fullscreen" => "maximized",
                    Some("normal") => "maximized",
                    Some("maximized") => {
                        return Err(CdpError::Protocol {
                            code: -32000,
                            message: "The 'maximized' state cannot be set on a fullscreen window"
                                .into(),
                        });
                    }
                    _ => state.window,
                };
                json!({})
            }
            "Page.getNavigationHistory" => {
                let tab = tab.ok_or(CdpError::SessionGone)?;
                let open = state
                    .tabs
                    .iter()
                    .find(|open| open.id == tab)
                    .ok_or_else(gone)?;
                let entries = open
                    .history
                    .iter()
                    .enumerate()
                    .map(|(id, (url, transition))| {
                        json!({ "id": id, "url": url, "title": url, "transitionType": transition })
                    })
                    .collect::<Vec<_>>();
                json!({ "currentIndex": open.index, "entries": entries })
            }
            "Page.navigate" => {
                let tab = tab.ok_or(CdpError::SessionGone)?;
                let url = params["url"].as_str().unwrap_or_default().to_string();
                let frame = format!("frame-{tab}");
                if state.unreachable.contains(&url) {
                    return Ok(json!({
                        "frameId": frame,
                        "loaderId": "loader",
                        "errorText": "net::ERR_NAME_NOT_RESOLVED",
                    }));
                }
                drop(state);
                self.navigate(tab, &url, "typed");
                return Ok(json!({ "frameId": frame, "loaderId": "loader" }));
            }
            "Page.navigateToHistoryEntry" => {
                let tab = tab.ok_or(CdpError::SessionGone)?;
                let entry = params["entryId"].as_u64().unwrap_or_default();
                if let Some(open) = state.tabs.iter_mut().find(|open| open.id == tab) {
                    open.index = usize::try_from(entry).unwrap_or_default();
                }
                json!({})
            }
            "Target.getTargets" => {
                let infos = state
                    .tabs
                    .iter()
                    .map(|tab| {
                        let mut info = json!({
                            "targetId": target(tab.id),
                            "type": "page",
                            "url": tab.history.get(tab.index).map(|(url, _)| url.clone()).unwrap_or_default(),
                        });
                        if let Some(opener) = tab.opener {
                            info["openerId"] = json!(target(opener));
                        }
                        info
                    })
                    .collect::<Vec<_>>();
                json!({ "targetInfos": infos })
            }
            "Browser.closeTab" => {
                state.tabs.retain(|tab| params["tabId"] != tab.id);
                json!({})
            }
            _ => {
                return Err(CdpError::Protocol {
                    code: -1,
                    message: format!("unexpected fake CDP call: {method}"),
                });
            }
        };
        Ok(reply)
    }

    /// Answers one call; a `Page.navigate`, a sweep read or a held method waits first while the
    /// test holds those, and a hung method never answers.
    async fn answer(
        &self,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CdpError> {
        let held = self
            .holds()
            .get(method)
            .filter(|gate| !*gate.borrow())
            .map(watch::Sender::subscribe);
        if let Some(mut held) = held {
            let _ = held.wait_for(|open| *open).await;
        }
        let open = *self.navigations.borrow();
        if method == "Page.navigate" && !open {
            let mut held = self.navigations.subscribe();
            let _ = held.wait_for(|open| *open).await;
        }
        if method == "Runtime.evaluate" && params["expression"] == SWEEP_READ {
            if !*self.sweep_reads.borrow() {
                let mut held = self.sweep_reads.subscribe();
                let _ = held.wait_for(|open| *open).await;
            }
            let time = self.state().sweep_read_time;
            tokio::time::sleep(time).await;
        }
        let hung = self.state().hung.iter().any(|hung| hung == method);
        if hung {
            std::future::pending::<()>().await;
        }
        self.reply(method, params, session)
    }

    async fn answer_raw(
        &self,
        method: &str,
        params_json: &str,
        session: Option<&SessionId>,
    ) -> Result<String, CdpError> {
        let reply = self
            .answer(method, serde_json::from_str(params_json)?, session)
            .await?;
        serde_json::to_string(&reply).map_err(CdpError::from)
    }
}

impl CdpConnection for FakeBrowser {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(self.answer(method, params, session))
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(self.answer_raw(method, params_json, session))
    }

    fn send_at_epoch<'a>(
        &'a self,
        _expected: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(self.answer(method, params, session))
    }

    fn send_raw_json_at_epoch<'a>(
        &'a self,
        _expected: u64,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(self.answer_raw(method, params_json, session))
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn connection_epoch(&self) -> u64 {
        1
    }
}
