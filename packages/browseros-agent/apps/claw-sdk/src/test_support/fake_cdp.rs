//! A fake browser behind core's `CdpConnection`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::{future, mem};

use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{CdpConnection, SessionId, TabId};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tokio::sync::{broadcast, oneshot};

/// The one window every fake tab lives in.
const WINDOW_ID: i64 = 1;
/// The first tab id the fake hands out, far from core's page ids so a test cannot mix them up.
const FIRST_TAB_ID: i64 = 101;
/// Events a test may leave unread before its receivers see `Lagged`.
const EVENT_BACKLOG: usize = 1024;

/// A browser that answers the CDP calls core and claw-sdk make, from an in-memory list of tabs.
///
/// It models what the real seam does and core's own test connection cannot:
/// - tabs that stay open or closed (`Browser.getTabs`, `getTabInfo`, `createTab`, `closeTab`);
/// - target events only after `Target.setDiscoverTargets` on the current socket, which first
///   announces every target that exists, as Chromium sends them; a tab can change target
///   and stay open ([`FakeCdp::replace_target`]);
/// - a socket that drops and comes back at a new epoch, with epoch-bound sends refused across
///   it (`send_at_epoch`, `send_raw_json_at_epoch`), which an unbound page open needs;
/// - methods that never answer, for cancellation tests;
/// - late news: a reply that was true when the browser sent it but arrives after other calls
///   were answered ([`FakeCdp::hold_next`]), and target events that arrive late
///   ([`FakeCdp::hold_events`]), so a test can order what Neo learns against what a host does;
/// - `Page.navigate` arriving at a page [`FakeCdp::serve`] registered, so a load changes the
///   tab's URL and title the way a real document's `<title>` would.
///
/// Any other method answers `{}`. Every call is logged with the epoch it was sent on.
#[derive(Debug)]
pub struct FakeCdp {
    state: Mutex<FakeState>,
    events: broadcast::Sender<CdpEvent>,
}

/// One CDP call the fake received, for assertions. A plain data bag.
#[derive(Clone, Debug, PartialEq)]
pub struct FakeCall {
    /// The CDP method, e.g. `Browser.getTabInfo`.
    pub method: String,
    /// Its params as sent.
    pub params: Value,
    /// The connection epoch it was sent on.
    pub epoch: u64,
}

#[derive(Debug)]
struct FakeState {
    connected: bool,
    epoch: u64,
    reachable: bool,
    connect_attempts: usize,
    /// Whether this socket asked for target events. Chromium forgets it with the socket.
    discovering: bool,
    tabs: Vec<FakeTab>,
    next_tab: i64,
    /// Numbers the targets [`FakeCdp::replace_target`] makes, so each id is new.
    replaced_targets: u64,
    calls: Vec<FakeCall>,
    hung: HashSet<String>,
    /// Canned answers that replace the fake's own for a method.
    answers: HashMap<String, Value>,
    /// URL -> the title of the document served there.
    served: HashMap<String, String>,
    /// Methods whose next call is answered only on [`FakeCdp::release_held`].
    hold_next: HashSet<String>,
    /// One gate per held reply, opened by [`FakeCdp::release_held`].
    held_replies: Vec<oneshot::Sender<()>>,
    /// Target events waiting for [`FakeCdp::release_events`]; `None` while events flow.
    held_events: Option<Vec<CdpEvent>>,
}

#[derive(Clone, Debug)]
struct FakeTab {
    tab: i64,
    /// The page target the tab shows now: `target-<tab>` until it is replaced.
    target: String,
    url: String,
    title: String,
}

impl FakeCdp {
    /// A browser with no tabs, connected at epoch 1 and accepting connections.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(FakeState {
                connected: true,
                epoch: 1,
                reachable: true,
                connect_attempts: 0,
                discovering: false,
                tabs: Vec::new(),
                next_tab: FIRST_TAB_ID,
                replaced_targets: 0,
                calls: Vec::new(),
                hung: HashSet::new(),
                answers: HashMap::new(),
                served: HashMap::new(),
                hold_next: HashSet::new(),
                held_replies: Vec::new(),
                held_events: None,
            }),
            events: broadcast::channel(EVENT_BACKLOG).0,
        })
    }

    /// One connect attempt, as `BrowserLink`'s connect loop makes them: the fake itself while it
    /// accepts connections, else the error a browser that is not up yet gives.
    pub fn connect(self: &Arc<Self>) -> Result<Arc<dyn CdpConnection>, CdpError> {
        let mut state = self.lock();
        state.connect_attempts += 1;
        if !state.reachable {
            return Err(CdpError::Discovery(
                "the fake browser is not accepting connections".to_owned(),
            ));
        }
        drop(state);
        Ok(Arc::clone(self) as Arc<dyn CdpConnection>)
    }

    /// Whether [`FakeCdp::connect`] succeeds.
    pub fn set_reachable(&self, reachable: bool) {
        self.lock().reachable = reachable;
    }

    /// How many times [`FakeCdp::connect`] was called.
    #[must_use]
    pub fn connect_attempts(&self) -> usize {
        self.lock().connect_attempts
    }

    /// Opens a tab the way the owner would, outside any op. Returns its tab id.
    pub fn open_tab(&self, url: &str, title: &str) -> TabId {
        let mut state = self.lock();
        let tab = state.add_tab(url, title);
        let events = state.created_events(tab);
        drop(state);
        self.emit(events);
        TabId(tab)
    }

    /// Closes a tab the way the owner would. `false` when the fake has no such tab.
    pub fn close_tab(&self, tab: TabId) -> bool {
        let mut state = self.lock();
        let Some(closed) = state.remove_tab(tab.0) else {
            return false;
        };
        let events = state.destroyed_events(&closed.target);
        drop(state);
        self.emit(events);
        true
    }

    /// The tab's page target is replaced while the tab stays open, as when Chromium swaps the
    /// tab's contents: the new target is created, then the old one destroyed. `false` when the
    /// fake has no such tab.
    pub fn replace_target(&self, tab: TabId) -> bool {
        let mut state = self.lock();
        state.replaced_targets += 1;
        let target = format!("target-{}.{}", tab.0, state.replaced_targets);
        let Some(entry) = state.tabs.iter_mut().find(|entry| entry.tab == tab.0) else {
            return false;
        };
        let old = mem::replace(&mut entry.target, target);
        let mut events = state.created_events(tab.0);
        events.extend(state.destroyed_events(&old));
        drop(state);
        self.emit(events);
        true
    }

    /// An event the page in `tab` sends, such as `Runtime.consoleAPICalled` or
    /// `Page.javascriptDialogOpening`, on the session `Target.attachToTarget` gives for the tab's
    /// current target. It reaches a session's listeners only once something attached to the tab.
    /// `false` when the fake has no such tab.
    pub fn page_event(&self, tab: TabId, method: &str, params: Value) -> bool {
        let state = self.lock();
        let Some(entry) = state.tabs.iter().find(|entry| entry.tab == tab.0) else {
            return false;
        };
        let session = SessionId::from(format!("session-{}", entry.target));
        drop(state);
        self.emit(vec![CdpEvent {
            method: method.to_owned(),
            params,
            session_id: Some(session),
        }]);
        true
    }

    /// Changes a tab's URL and title, as a navigation that already happened would. Core's
    /// cache keeps the old ones until something refreshes the page. `false` when the fake has
    /// no such tab.
    pub fn navigate_tab(&self, tab: TabId, url: &str, title: &str) -> bool {
        let mut state = self.lock();
        let Some(entry) = state.tabs.iter_mut().find(|entry| entry.tab == tab.0) else {
            return false;
        };
        url.clone_into(&mut entry.url);
        title.clone_into(&mut entry.title);
        true
    }

    /// The socket drops: every call fails with `NotConnected` until [`FakeCdp::reconnect`].
    pub fn drop_connection(&self) {
        let mut state = self.lock();
        state.connected = false;
        state.discovering = false;
    }

    /// A new socket: the epoch moves on and target discovery is off again. Returns the new
    /// epoch.
    pub fn reconnect(&self) -> u64 {
        let mut state = self.lock();
        state.connected = true;
        state.discovering = false;
        state.epoch += 1;
        state.epoch
    }

    /// Calls to `method` from now on are logged and never answered.
    pub fn hang(&self, method: &str) {
        self.lock().hung.insert(method.to_owned());
    }

    /// Calls to `method` from now on answer `result`, e.g. a `Page.navigate` that reports
    /// `errorText` the way Chromium does for a host that does not resolve.
    pub fn answer(&self, method: &str, result: Value) {
        self.lock().answers.insert(method.to_owned(), result);
    }

    /// The next call to `method` is answered as the browser would answer it at once, and its
    /// events go out at once, but the reply reaches the caller only on
    /// [`FakeCdp::release_held`]: a tab listing that was true when Chromium sent it and arrives
    /// after a tab opened, or a tab that exists before `Browser.createTab` returns.
    pub fn hold_next(&self, method: &str) {
        self.lock().hold_next.insert(method.to_owned());
    }

    /// Delivers every reply [`FakeCdp::hold_next`] held back.
    pub fn release_held(&self) {
        let gates = mem::take(&mut self.lock().held_replies);
        for gate in gates {
            // A caller that stopped waiting is not an error.
            let _ = gate.send(());
        }
    }

    /// Target events from now on wait, in order, until [`FakeCdp::release_events`]: a browser
    /// whose events reach Neo after the answers to calls made in the meantime.
    pub fn hold_events(&self) {
        self.lock().held_events.get_or_insert_with(Vec::new);
    }

    /// Sends every held target event, in the order the browser made them, and stops holding.
    pub fn release_events(&self) {
        let held = self.lock().held_events.take().unwrap_or_default();
        for event in held {
            // No receiver is not an error.
            let _ = self.events.send(event);
        }
    }

    /// Serves a document titled `title` at `url`: a `Page.navigate` to it, on a tab's session,
    /// changes that tab's URL and title. A navigation to any other URL changes nothing.
    pub fn serve(&self, url: &str, title: &str) {
        self.lock().served.insert(url.to_owned(), title.to_owned());
    }

    /// Every call so far, oldest first.
    #[must_use]
    pub fn calls(&self) -> Vec<FakeCall> {
        self.lock().calls.clone()
    }

    /// The open tabs, oldest first, including one whose `Browser.createTab` reply is held.
    #[must_use]
    pub fn tabs(&self) -> Vec<TabId> {
        self.lock()
            .tabs
            .iter()
            .map(|entry| TabId(entry.tab))
            .collect()
    }

    /// How many calls to `method` the fake received.
    #[must_use]
    pub fn count(&self, method: &str) -> usize {
        self.lock()
            .calls
            .iter()
            .filter(|call| call.method == method)
            .count()
    }

    /// Answers one call. `None` means it never will. `expected` is the caller's epoch for an
    /// epoch-bound send, checked under the same lock that a reconnect takes, as the real
    /// client checks it under its socket lock.
    async fn send_value(
        &self,
        expected: Option<u64>,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CdpError> {
        let (reply, gate) = {
            let mut state = self.lock();
            state.reply(expected, method, params, session)
        };
        match reply {
            Reply::Now(result, events) => {
                self.emit(events);
                if let Some(gate) = gate {
                    // Dropped only with the fake itself; the reply goes out either way.
                    let _ = gate.await;
                }
                result
            }
            Reply::Never => future::pending().await,
        }
    }

    fn emit(&self, events: Vec<CdpEvent>) {
        let mut state = self.lock();
        if let Some(held) = &mut state.held_events {
            held.extend(events);
            return;
        }
        drop(state);
        for event in events {
            // No receiver is not an error.
            let _ = self.events.send(event);
        }
    }

    fn lock(&self) -> MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

enum Reply {
    /// The answer, and the target events the call made the browser send.
    Now(Result<Value, CdpError>, Vec<CdpEvent>),
    Never,
}

impl FakeState {
    /// The reply, and the gate it waits behind when its method was held
    /// ([`FakeCdp::hold_next`]). Only a call the browser answers uses up a hold.
    fn reply(
        &mut self,
        expected: Option<u64>,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> (Reply, Option<oneshot::Receiver<()>>) {
        if !self.connected {
            return (Reply::Now(Err(CdpError::NotConnected), Vec::new()), None);
        }
        if let Some(expected) = expected
            && expected != self.epoch
        {
            let error = CdpError::ConnectionEpochChanged {
                expected,
                actual: self.epoch,
            };
            return (Reply::Now(Err(error), Vec::new()), None);
        }
        let reply = if self.hung.contains(method) {
            Reply::Never
        } else if let Some(result) = self.answers.get(method) {
            Reply::Now(Ok(result.clone()), Vec::new())
        } else {
            self.answer(method, &params, session)
        };
        self.calls.push(FakeCall {
            method: method.to_owned(),
            params,
            epoch: self.epoch,
        });
        let gate = self.hold_next.remove(method).then(|| {
            let (open, gate) = oneshot::channel();
            self.held_replies.push(open);
            gate
        });
        (reply, gate)
    }

    /// The fake's behavior per method. Anything it does not model answers `{}`.
    fn answer(&mut self, method: &str, params: &Value, session: Option<&SessionId>) -> Reply {
        match method {
            "Browser.getTabs" => {
                let tabs: Vec<Value> = (0..self.tabs.len())
                    .filter_map(|index| self.tab_json(index))
                    .collect();
                Reply::Now(Ok(json!({ "tabs": tabs })), Vec::new())
            }
            "Browser.getTabInfo" => {
                let tab = tab_param(params).and_then(|tab| self.tab_reply(tab));
                Reply::Now(tab.ok_or_else(no_tab), Vec::new())
            }
            "Browser.createTab" => {
                let url = params
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("about:blank");
                let tab = self.add_tab(url, url);
                let events = self.created_events(tab);
                Reply::Now(self.tab_reply(tab).ok_or_else(no_tab), events)
            }
            "Browser.closeTab" => self.close_by_call(params),
            "Page.navigate" => {
                self.arrive(params, session);
                Reply::Now(Ok(json!({})), Vec::new())
            }
            "Target.setDiscoverTargets" => {
                self.discovering = params
                    .get("discover")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                // Chromium announces every target that exists before it answers.
                let tabs: Vec<i64> = self.tabs.iter().map(|entry| entry.tab).collect();
                let events: Vec<CdpEvent> = tabs
                    .into_iter()
                    .flat_map(|tab| self.created_events(tab))
                    .collect();
                Reply::Now(Ok(json!({})), events)
            }
            "Target.attachToTarget" => {
                let target = params
                    .get("targetId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                Reply::Now(
                    Ok(json!({ "sessionId": format!("session-{target}") })),
                    Vec::new(),
                )
            }
            _ => Reply::Now(Ok(json!({})), Vec::new()),
        }
    }

    /// `Browser.closeTab`, which names the tab by `tabId` (core's page manager) or by
    /// `targetId` (a bound view).
    fn close_by_call(&mut self, params: &Value) -> Reply {
        let target = params.get("targetId").and_then(Value::as_str);
        let by_target = || {
            self.tabs
                .iter()
                .find(|entry| Some(entry.target.as_str()) == target)
                .map(|entry| entry.tab)
        };
        let Some(tab) = tab_param(params).or_else(by_target) else {
            return Reply::Now(Err(no_tab()), Vec::new());
        };
        let Some(closed) = self.remove_tab(tab) else {
            return Reply::Now(Err(no_tab()), Vec::new());
        };
        let events = self.destroyed_events(&closed.target);
        Reply::Now(Ok(json!({})), events)
    }

    /// A `Page.navigate` on a tab's session (`session-target-<tab>`, as `attachToTarget`
    /// answers) to a served URL: the tab now shows that document.
    fn arrive(&mut self, params: &Value, session: Option<&SessionId>) {
        let tab = session
            .and_then(|session| session.as_str().strip_prefix("session-target-"))
            .and_then(|tab| tab.parse::<i64>().ok());
        let Some(url) = params.get("url").and_then(Value::as_str) else {
            return;
        };
        let (Some(tab), Some(title)) = (tab, self.served.get(url)) else {
            return;
        };
        if let Some(entry) = self.tabs.iter_mut().find(|entry| entry.tab == tab) {
            url.clone_into(&mut entry.url);
            title.clone_into(&mut entry.title);
        }
    }

    /// `{ "tab": TabInfo }`, the reply to `getTabInfo` and `createTab`.
    fn tab_reply(&self, tab: i64) -> Option<Value> {
        let index = self.tabs.iter().position(|entry| entry.tab == tab)?;
        Some(json!({ "tab": self.tab_json(index)? }))
    }

    fn add_tab(&mut self, url: &str, title: &str) -> i64 {
        let tab = self.next_tab;
        self.next_tab += 1;
        self.tabs.push(FakeTab {
            tab,
            target: format!("target-{tab}"),
            url: url.to_owned(),
            title: title.to_owned(),
        });
        tab
    }

    /// Takes the tab out, returning it as it was.
    fn remove_tab(&mut self, tab: i64) -> Option<FakeTab> {
        let index = self.tabs.iter().position(|entry| entry.tab == tab)?;
        Some(self.tabs.remove(index))
    }

    /// A tab as Chromium's `Browser.TabInfo` describes it.
    fn tab_json(&self, index: usize) -> Option<Value> {
        let entry = self.tabs.get(index)?;
        Some(json!({
            "tabId": entry.tab,
            "targetId": entry.target,
            "url": entry.url,
            "title": entry.title,
            "isActive": false,
            "isLoading": false,
            "loadProgress": 1.0,
            "isPinned": false,
            "isHidden": false,
            "windowId": WINDOW_ID,
            "index": index,
        }))
    }

    /// `Target.targetCreated` for the tab's current page target, in Chromium's shape with the
    /// `tabId` BrowserOS adds; none while this socket has not asked for target events.
    fn created_events(&self, tab: i64) -> Vec<CdpEvent> {
        let Some(entry) = self.tabs.iter().find(|entry| entry.tab == tab) else {
            return Vec::new();
        };
        let info = json!({ "targetId": entry.target, "type": "page", "tabId": tab });
        self.target_event("Target.targetCreated", json!({ "targetInfo": info }))
    }

    /// `Target.targetDestroyed`, which names only the target; none while this socket has not
    /// asked for target events.
    fn destroyed_events(&self, target: &str) -> Vec<CdpEvent> {
        self.target_event("Target.targetDestroyed", json!({ "targetId": target }))
    }

    fn target_event(&self, method: &str, params: Value) -> Vec<CdpEvent> {
        if !(self.connected && self.discovering) {
            return Vec::new();
        }
        vec![CdpEvent {
            method: method.to_owned(),
            params,
            session_id: None,
        }]
    }
}

fn tab_param(params: &Value) -> Option<i64> {
    params.get("tabId").and_then(Value::as_i64)
}

fn no_tab() -> CdpError {
    CdpError::Protocol {
        code: -32000,
        message: "No tab with given id".to_owned(),
    }
}

impl CdpConnection for FakeCdp {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(self.send_value(None, method, params, session))
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            let params = serde_json::from_str(params_json)?;
            let value = self.send_value(None, method, params, session).await?;
            Ok(serde_json::to_string(&value)?)
        })
    }

    fn send_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(self.send_value(Some(expected_epoch), method, params, session))
    }

    fn send_raw_json_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            let params = serde_json::from_str(params_json)?;
            let value = self
                .send_value(Some(expected_epoch), method, params, session)
                .await?;
            Ok(serde_json::to_string(&value)?)
        })
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }

    fn is_connected(&self) -> bool {
        self.lock().connected
    }

    fn connection_epoch(&self) -> u64 {
        self.lock().epoch
    }
}
