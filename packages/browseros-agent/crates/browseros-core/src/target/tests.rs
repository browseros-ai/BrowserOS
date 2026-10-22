//! Stateful native adapter tests exercise the public view, including mutations and retained
//! capabilities. This is Rust admission proof; real Chromium acceptance belongs to integration.
use super::*;
use crate::{
    Browser, CdpConnection, TargetId,
    input::ClickOptions,
    pages::{PageManager, PageManagerHooks},
};
use futures_util::future::BoxFuture;
use std::{
    collections::VecDeque,
    error::Error,
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
};

type TestResult = Result<(), Box<dyn Error>>;
#[derive(Clone, Debug)]
struct Call {
    method: String,
    params: Value,
    session: Option<SessionId>,
}
struct Fixture {
    profiles: Vec<Value>,
    windows: Vec<Value>,
    tabs: Vec<Value>,
    groups: Vec<Value>,
    calls: Vec<Call>,
    missing_profiles: bool,
    replace_on_attach: VecDeque<String>,
    context_after_send: Option<String>,
    move_on_window_read: Option<(usize, i64, i64)>,
    move_group_on_tab_read: Option<(usize, i64)>,
    epoch_after_profiles: bool,
    fail_load: bool,
}
struct Native {
    state: Mutex<Fixture>,
    epoch: AtomicU64,
    events: broadcast::Sender<CdpEvent>,
    attach_hold: Mutex<Option<Arc<tokio::sync::Notify>>>,
    attach_started: tokio::sync::Notify,
    detach_done: tokio::sync::Notify,
    navigation_sent: tokio::sync::Notify,
    dispatch_hold: Mutex<Option<(String, Arc<tokio::sync::Notify>)>>,
    dispatch_started: tokio::sync::Notify,
    creation_read_hold: Mutex<Option<Arc<tokio::sync::Notify>>>,
    group_read_hold: Mutex<Option<Arc<tokio::sync::Notify>>>,
    group_read_started: tokio::sync::Notify,
}
fn profile(id: &str, loaded: bool) -> Value {
    let mut value = json!({"browserProfileId":id,"name":id,"directory":format!("directory-{id}"),"loaded":loaded});
    if loaded {
        value["browserContextId"] = json!(format!("live-{id}"));
    }
    value
}
fn window(id: i64, profile: &str, kind: &str, active: bool) -> Value {
    json!({"windowId":id,"windowType":kind,"bounds":{},"isActive":active,"isVisible":true,"tabCount":1,
        "browserContextId":"legacy-directory-not-a-context","browserProfileId":profile,"liveBrowserContextId":format!("live-{profile}")})
}
fn tab(id: i64, window: i64, profile: &str) -> Value {
    json!({"tabId":id,"targetId":format!("target-{id}"),"windowId":window,"url":format!("https://{profile}.test/{id}"),"title":profile,
        "isActive":true,"isLoading":false,"loadProgress":1.0,"isPinned":false,"isHidden":false,
        "browserContextId":"legacy-directory-not-a-context","browserProfileId":profile,"liveBrowserContextId":format!("live-{profile}")})
}
impl Native {
    fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(32);
        let mut grouped = tab(1, 11, "eng");
        grouped["groupId"] = json!("eng-group");
        Arc::new(Self {
            epoch: AtomicU64::new(1),
            events,
            attach_hold: Mutex::new(None),
            attach_started: tokio::sync::Notify::new(),
            detach_done: tokio::sync::Notify::new(),
            navigation_sent: tokio::sync::Notify::new(),
            dispatch_hold: Mutex::new(None),
            dispatch_started: tokio::sync::Notify::new(),
            creation_read_hold: Mutex::new(None),
            group_read_hold: Mutex::new(None),
            group_read_started: tokio::sync::Notify::new(),
            state: Mutex::new(Fixture {
                profiles: vec![
                    profile("eng", true),
                    profile("sales", true),
                    profile("closed", false),
                ],
                windows: vec![
                    window(11, "eng", "normal", false),
                    window(12, "eng", "normal", false),
                    window(13, "eng", "popup", true),
                    window(20, "sales", "normal", true),
                ],
                tabs: vec![
                    grouped,
                    tab(2, 12, "eng"),
                    tab(3, 13, "eng"),
                    tab(4, 20, "sales"),
                ],
                groups: vec![
                    json!({"groupId":"eng-group","windowId":11,"title":"eng","color":"blue","collapsed":false,"tabIds":[1]}),
                ],
                calls: Vec::new(),
                missing_profiles: false,
                replace_on_attach: VecDeque::new(),
                context_after_send: None,
                move_on_window_read: None,
                move_group_on_tab_read: None,
                epoch_after_profiles: false,
                fail_load: false,
            }),
        })
    }
    fn lock(&self) -> MutexGuard<'_, Fixture> {
        self.state
            .lock()
            .unwrap_or_else(|err| panic!("fixture poisoned: {err}"))
    }
    fn calls(&self, method: &str) -> Vec<Call> {
        self.lock()
            .calls
            .iter()
            .filter(|call| call.method == method)
            .cloned()
            .collect()
    }
    fn reply(
        &self,
        expected_epoch: Option<u64>,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CdpError> {
        let mut state = self.lock();
        // This lock also owns fixture reconnect publication. Pauses occur before the lock,
        // so the final epoch check and mutation recording model the real CDP sink boundary.
        if let Some(expected) = expected_epoch {
            let actual = self.epoch.load(Ordering::SeqCst);
            if expected != actual {
                return Err(CdpError::ConnectionEpochChanged { expected, actual });
            }
        }
        state.calls.push(Call {
            method: method.into(),
            params: params.clone(),
            session: session.cloned(),
        });
        // The real native resolver rejects dual selectors (-32602), even when they identify
        // the same tab. A permissive OR lookup hid invalid bound-facade close/move requests.
        if matches!(
            method,
            "Browser.getTabInfo"
                | "Browser.closeTab"
                | "Browser.activateTab"
                | "Browser.showTab"
                | "Browser.moveTab"
        ) && params.get("targetId").is_some() == params.get("tabId").is_some()
        {
            return Err(CdpError::Protocol {
                code: -32602,
                message: if params.get("targetId").is_some() {
                    "Specify either targetId or tabId, not both"
                } else {
                    "Either targetId or tabId must be specified"
                }
                .into(),
            });
        }
        let result = match method {
            "Browser.getProfiles" if state.missing_profiles => {
                return Err(CdpError::Protocol {
                    code: -32601,
                    message: "method not found".into(),
                });
            }
            "Browser.getProfiles" => json!({"profiles":state.profiles}),
            "Browser.getWindows" => json!({"windows":state.windows}),
            "Browser.getTabs" => {
                json!({"tabs":state.tabs.iter().filter(|tab|params.get("windowId").is_none_or(|id|*id == tab["windowId"])).collect::<Vec<_>>()})
            }
            "Browser.getTabGroups" => json!({"groups":state.groups}),
            "Browser.getTabInfo" => {
                json!({"tab":state.tabs.iter().find(|tab|params.get("tabId").is_some_and(|id|*id == tab["tabId"]) || params.get("targetId").is_some_and(|id|*id == tab["targetId"])).ok_or(CdpError::SessionGone)?})
            }
            "Browser.getActiveTab" => {
                json!({"tab":state.tabs.iter().find(|tab|params.get("windowId").is_none_or(|id|*id == tab["windowId"]))})
            }
            "Browser.loadProfile" if state.fail_load => {
                return Err(CdpError::Protocol {
                    code: -32000,
                    message: "profile could not be loaded".into(),
                });
            }
            "Browser.loadProfile" => {
                let saved = state
                    .profiles
                    .iter_mut()
                    .find(|profile| profile["browserProfileId"] == params["browserProfileId"])
                    .ok_or(CdpError::SessionGone)?;
                let loaded = saved["loaded"] == false;
                saved["loaded"] = json!(true);
                saved["browserContextId"] = json!(format!(
                    "live-{}",
                    params["browserProfileId"].as_str().unwrap_or_default()
                ));
                json!({"profile":saved,"loaded":loaded})
            }
            "Autofill.getCredentialContext" => json!({"context":{
                "browserProfileId":"eng","origin":"https://eng.test","documentId":"fixture-document"}}),
            "Autofill.cancelCredentialOperation" => json!({}),
            "Autofill.listCredentials" => {
                let owner = params["browserContextId"]
                    .as_str()
                    .and_then(|context| context.strip_prefix("live-"))
                    .ok_or(CdpError::SessionGone)?;
                json!({"result":{"browserProfileId":owner,"credentials":[],"total":0}})
            }
            "Browser.createWindow" => {
                let id = state
                    .windows
                    .iter()
                    .filter_map(|window| window["windowId"].as_i64())
                    .max()
                    .unwrap_or_default()
                    + 1;
                let profile = params["browserContextId"]
                    .as_str()
                    .and_then(|id| id.strip_prefix("live-"))
                    .unwrap_or("sales");
                let created = window(id, profile, "normal", false);
                state.windows.push(created.clone());
                // Like the native, a window created with a URL opens it as its first tab.
                if let Some(url) = params["url"].as_str() {
                    let tab_id = state
                        .tabs
                        .iter()
                        .filter_map(|tab| tab["tabId"].as_i64())
                        .max()
                        .unwrap_or_default()
                        + 1;
                    let mut first = tab(tab_id, id, profile);
                    first["url"] = json!(url);
                    state.tabs.push(first);
                }
                json!({"window":created})
            }
            "Browser.createTab" => {
                let window = params["windowId"].as_i64().unwrap_or(20);
                let owner = state
                    .windows
                    .iter()
                    .find(|candidate| candidate["windowId"] == window)
                    .and_then(|window| window["browserProfileId"].as_str())
                    .unwrap_or("sales");
                if params
                    .get("browserContextId")
                    .is_some_and(|context| *context != json!(format!("live-{owner}")))
                {
                    return Err(CdpError::Protocol {
                        code: -32000,
                        message: "window/context mismatch".into(),
                    });
                }
                let id = state
                    .tabs
                    .iter()
                    .filter_map(|tab| tab["tabId"].as_i64())
                    .max()
                    .unwrap_or_default()
                    + 1;
                let mut created = tab(id, window, owner);
                // The native opens a default page when no URL is given.
                created["url"] = params
                    .get("url")
                    .cloned()
                    .unwrap_or_else(|| json!("about:blank"));
                state.tabs.push(created.clone());
                json!({"tab":created})
            }
            "Browser.moveTab" => {
                let found = state
                    .tabs
                    .iter_mut()
                    .find(|tab| {
                        params.get("tabId").is_some_and(|id| *id == tab["tabId"])
                            || params
                                .get("targetId")
                                .is_some_and(|id| *id == tab["targetId"])
                    })
                    .ok_or(CdpError::SessionGone)?;
                found["windowId"] = params["windowId"].clone();
                json!({"tab":found})
            }
            "Browser.closeWindow" => {
                state
                    .windows
                    .retain(|window| window["windowId"] != params["windowId"]);
                json!({})
            }
            "Browser.closeTab" => {
                state.tabs.retain(|tab| {
                    tab["targetId"] != params["targetId"] && tab["tabId"] != params["tabId"]
                });
                json!({})
            }
            "Browser.addTabsToGroup" => {
                let index = state
                    .groups
                    .iter()
                    .position(|group| group["groupId"] == params["groupId"])
                    .ok_or(CdpError::SessionGone)?;
                let ids = params["tabIds"].as_array().ok_or(CdpError::SessionGone)?;
                let window = state.groups[index]["windowId"].clone();
                // BrowserHandler::AddTabsToGroup rejects cross-window tabs before calling
                // AddToExistingGroup. It does not relocate even same-profile tabs.
                if ids.iter().any(|id| {
                    !state
                        .tabs
                        .iter()
                        .any(|tab| tab["tabId"] == *id && tab["windowId"] == window)
                }) {
                    return Err(CdpError::Protocol {
                        code: -32602,
                        message: "Tabs must be in the same window as the group".into(),
                    });
                }
                for tab in &mut state.tabs {
                    if ids.contains(&tab["tabId"]) {
                        tab["groupId"] = params["groupId"].clone();
                    }
                }
                let members = state.groups[index]["tabIds"]
                    .as_array_mut()
                    .ok_or(CdpError::SessionGone)?;
                for id in ids {
                    if !members.contains(id) {
                        members.push(id.clone());
                    }
                }
                json!({"group":state.groups[index]})
            }
            "Browser.moveTabGroup" => {
                let index = state
                    .groups
                    .iter()
                    .position(|group| group["groupId"] == params["groupId"])
                    .ok_or(CdpError::SessionGone)?;
                let ids = state.groups[index]["tabIds"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                state.groups[index]["windowId"] = params["windowId"].clone();
                for tab in &mut state.tabs {
                    if ids.contains(&tab["tabId"]) {
                        tab["windowId"] = params["windowId"].clone();
                    }
                }
                json!({"group":state.groups[index]})
            }
            "Target.attachToTarget" => {
                if let Some(target) = state.replace_on_attach.pop_front() {
                    state.tabs[0]["targetId"] = json!(target);
                }
                json!({"sessionId":format!("session-{}",params["targetId"].as_str().unwrap_or_default())})
            }
            "Target.getTargets" => {
                let mut targets = state.tabs.iter().map(|tab|json!({"targetId":tab["targetId"],"type":"page","title":tab["title"],"url":tab["url"],"attached":false,"browserContextId":tab["liveBrowserContextId"]})).collect::<Vec<_>>();
                for id in ["eng", "sales"] {
                    targets.push(json!({"targetId":format!("worker-{id}"),"type":"service_worker","url":"chrome-extension://synthetic/worker.js","browserContextId":format!("live-{id}")}));
                }
                json!({"targetInfos":targets})
            }
            "Target.getTargetInfo" => {
                json!({"targetInfo":{"targetId":params["targetId"],"type":"service_worker","url":"chrome-extension://synthetic/worker.js","browserContextId":"live-eng"}})
            }
            "Runtime.evaluate" => {
                if params["expression"] == "document.readyState" {
                    json!({"result":{"value":"complete"}})
                } else {
                    json!({"result":{"value":session.map(SessionId::as_str).unwrap_or("root")}})
                }
            }
            "Storage.getCookies"
            | "Storage.setCookies"
            | "Storage.clearCookies"
            | "History.search"
            | "History.getRecent"
            | "History.deleteUrl"
            | "History.deleteRange"
            | "Browser.setPermission"
            | "Browser.grantPermissions"
            | "Browser.resetPermissions"
            | "Browser.setDownloadBehavior" => json!({"routedContext":params["browserContextId"]}),
            _ => json!({}),
        };
        if method == "Browser.getWindows"
            && let Some((remaining, tab_id, destination)) = state.move_on_window_read.take()
        {
            if remaining == 1 {
                if let Some(tab) = state.tabs.iter_mut().find(|tab| tab["tabId"] == tab_id) {
                    tab["windowId"] = json!(destination);
                }
            } else {
                state.move_on_window_read = Some((remaining - 1, tab_id, destination));
            }
        }
        if method == "Browser.getTabInfo"
            && let Some((remaining, destination)) = state.move_group_on_tab_read.take()
        {
            if remaining == 1 {
                state.groups[0]["windowId"] = json!(destination);
                for tab in &mut state.tabs {
                    if tab["groupId"] == "eng-group" {
                        tab["windowId"] = json!(destination);
                    }
                }
            } else {
                state.move_group_on_tab_read = Some((remaining - 1, destination));
            }
        }
        if !matches!(
            method,
            "Browser.getProfiles"
                | "Browser.getWindows"
                | "Browser.getTabs"
                | "Browser.getTabInfo"
                | "Browser.getTabGroups"
        ) && let Some(context) = state.context_after_send.take()
        {
            state.profiles[0]["browserContextId"] = json!(context);
        }
        Ok(result)
    }

    fn reconnect(&self) {
        let _state = self.lock();
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }

    async fn dispatch(
        &self,
        expected: Option<u64>,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CdpError> {
        // Group admission can read existing tabs before creation. This separate one-shot
        // boundary pauses only the first read after the native create reply has been emitted.
        let creation_hold = if method == "Browser.getTabInfo"
            && self
                .lock()
                .calls
                .iter()
                .any(|call| call.method == "Browser.createTab")
        {
            self.creation_read_hold
                .lock()
                .unwrap_or_else(|err| panic!("fixture poisoned: {err}"))
                .take()
        } else {
            None
        };
        if let Some(hold) = creation_hold {
            self.dispatch_started.notify_one();
            hold.notified().await;
        }
        let hold = self
            .dispatch_hold
            .lock()
            .unwrap_or_else(|err| panic!("fixture poisoned: {err}"))
            .as_ref()
            .filter(|(held_method, _)| held_method == method)
            .map(|(_, hold)| hold.clone());
        if let Some(hold) = hold {
            self.dispatch_started.notify_one();
            hold.notified().await;
        }
        let worker_attach = method == "Target.attachToTarget"
            && params["targetId"]
                .as_str()
                .is_some_and(|id| id.starts_with("worker-"));
        let result = self.reply(expected, method, params, session);
        if method == "Browser.getProfiles" {
            let invalidate_epoch = std::mem::take(&mut self.lock().epoch_after_profiles);
            if invalidate_epoch {
                self.reconnect();
            }
        }
        if worker_attach {
            let hold = self
                .attach_hold
                .lock()
                .unwrap_or_else(|err| panic!("fixture poisoned: {err}"))
                .clone();
            if let Some(hold) = hold {
                self.attach_started.notify_one();
                hold.notified().await;
            }
        }
        if method == "Browser.getTabGroups" {
            let hold = self
                .group_read_hold
                .lock()
                .unwrap_or_else(|err| panic!("fixture poisoned: {err}"))
                .clone();
            if let Some(hold) = hold {
                self.group_read_started.notify_one();
                hold.notified().await;
            }
        }
        if method == "Target.detachFromTarget" {
            self.detach_done.notify_one();
        }
        if method == "Page.navigate" {
            self.navigation_sent.notify_one();
        }
        result
    }
}

#[tokio::test]
async fn show_tab_validates_destination_and_never_uses_global_focus() -> TestResult {
    let native = Native::new();
    let view = view(
        native.clone(),
        Some(WindowSelection::Pinned {
            window_id: WindowId(11),
        }),
    )
    .await;
    for destination in [12, 13, 20] {
        let error = view
            .cdp(
                "Browser.showTab",
                json!({"tabId":1,"windowId":destination}),
                None,
            )
            .await;
        assert!(matches!(
            error,
            Err(CoreError::Target(TargetError::WindowOutOfScope(_)))
        ));
    }
    assert!(native.calls("Browser.showTab").is_empty());
    view.cdp("Browser.showTab", json!({"tabId":1}), None)
        .await?;
    let calls = native.calls("Browser.showTab");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].params["windowId"], 11);
    assert_eq!(calls[0].params["targetId"], "target-1");
    Ok(())
}

#[tokio::test]
async fn tab_drag_during_scope_lookups_is_rejected_before_close_dispatch() -> TestResult {
    let native = Native::new();
    let view = view(
        native.clone(),
        Some(WindowSelection::Pinned {
            window_id: WindowId(11),
        }),
    )
    .await;
    // Drag after the page was admitted, while the final profile/window lookup is in flight.
    // A scope-only recheck misses this: the pin still exists, but the tab is no longer inside it.
    native.lock().move_on_window_read = Some((3, 1, 12));
    let error = view.cdp("Browser.closeTab", json!({"tabId":1}), None).await;
    assert!(matches!(
        error,
        Err(CoreError::Target(TargetError::PageOutOfScope(_)))
    ));
    assert!(native.calls("Browser.closeTab").is_empty());
    assert!(
        native
            .lock()
            .tabs
            .iter()
            .any(|tab| tab["tabId"] == 1 && tab["windowId"] == 12)
    );
    Ok(())
}
impl CdpConnection for Native {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(self.dispatch(None, method, params, session))
    }
    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            let value = self
                .send(method, serde_json::from_str(params)?, session)
                .await?;
            Ok(serde_json::to_string(&value)?)
        })
    }
    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }
    fn send_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(self.dispatch(Some(expected_epoch), method, params, session))
    }
    fn send_raw_json_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            let value = self
                .dispatch(
                    Some(expected_epoch),
                    method,
                    serde_json::from_str(params)?,
                    session,
                )
                .await?;
            Ok(serde_json::to_string(&value)?)
        })
    }
    fn is_connected(&self) -> bool {
        true
    }
    fn connection_epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }
}
fn binding(window: Option<WindowSelection>) -> TargetBinding {
    TargetBinding {
        intent: TargetIntent {
            browser_profile_id: BrowserProfileId("eng".into()),
            directory: "directory-eng".into(),
            name: "eng".into(),
            window,
        },
        revision: 1,
    }
}
async fn view(native: Arc<Native>, window: Option<WindowSelection>) -> BrowserView {
    BrowserView::capture(
        BrowserSession::new_with_native(
            native,
            crate::BrowserSessionHooks::default(),
            current_native(),
        ),
        Some(binding(window)),
        None,
    )
    .await
}
async fn page_id(view: &BrowserView, tab: i64) -> Result<PageId, CoreError> {
    view.session
        .pages
        .list()
        .await?
        .into_iter()
        .find(|page| page.tab_id.0 == tab)
        .map(|page| page.page_id)
        .ok_or_else(|| CoreError::Message("fixture page missing".into()))
}
fn code(error: CoreError, expected: &str) {
    assert!(error.to_string().starts_with(expected), "{error}");
}
fn native_version(raw: &str) -> NativeCapabilities {
    NativeCapabilities::from_browseros_version(Some(raw))
}
/// The first native release with profile targeting; fixtures that model the #2908 bundle use it.
fn current_native() -> NativeCapabilities {
    native_version("0.51.1.3")
}
async fn view_on(
    native: Arc<Native>,
    caps: NativeCapabilities,
    window: Option<WindowSelection>,
) -> BrowserView {
    BrowserView::capture(
        BrowserSession::new_with_native(native, crate::BrowserSessionHooks::default(), caps),
        Some(binding(window)),
        None,
    )
    .await
}

#[tokio::test]
async fn normal_window_projection_preserves_global_inventory_and_raw_sidecar() -> TestResult {
    let native = Native::new();
    let scoped = view(native, None).await;
    let listing = scoped.list_pages().await?;
    assert_eq!(listing.pages.len(), 2);
    assert_eq!(listing.hidden, 2);
    assert_eq!(listing.live_page_ids.len(), 4);
    assert_eq!(
        scoped.session.pages.known_page_ids().await,
        listing.live_page_ids
    );
    assert!(
        listing
            .pages
            .iter()
            .all(
                |page| page.browser_profile_id.as_ref() == Some(&BrowserProfileId("eng".into()))
                    && page.live_browser_context_id.as_ref()
                        == Some(&BrowserContextId("live-eng".into()))
            )
    );
    assert_eq!(
        scoped
            .list_windows()
            .await?
            .iter()
            .map(|window| window.window_id)
            .collect::<Vec<_>>(),
        vec![11, 12]
    );
    Ok(())
}
#[tokio::test]
async fn focus_and_working_window_are_placement_while_pin_is_restriction() -> TestResult {
    let native = Native::new();
    let scoped = view(native.clone(), None).await;
    assert_eq!(
        scoped.scope()?.and_then(|scope| scope.placement_window),
        Some(WindowId(12))
    );
    assert_eq!(
        scoped.active_page().await?.and_then(|page| page.window_id),
        Some(WindowId(12))
    );
    let working = view(
        native.clone(),
        Some(WindowSelection::Working {
            window_id: WindowId(11),
        }),
    )
    .await;
    assert_eq!(working.list_pages().await?.pages.len(), 2);
    let created = working
        .new_page("https://created.test", NewPageOptions::default())
        .await?;
    assert_eq!(created.window_id, Some(WindowId(11)));
    let sent = native.calls("Browser.createTab");
    assert_eq!(sent[0].params["browserContextId"], "live-eng");
    assert_eq!(sent[0].params["background"], true);
    let pinned = view(
        native.clone(),
        Some(WindowSelection::Pinned {
            window_id: WindowId(11),
        }),
    )
    .await;
    assert!(
        pinned
            .list_pages()
            .await?
            .pages
            .iter()
            .all(|page| page.window_id == Some(WindowId(11)))
    );
    code(
        pinned
            .create_window()
            .await
            .err()
            .ok_or("creation unexpectedly admitted")?,
        "pinned_window_restricts",
    );
    Ok(())
}
#[tokio::test]
async fn legacy_or_missing_metadata_never_admits_and_popup_ids_fail_before_mutation() -> TestResult
{
    let native = Native::new();
    native.lock().windows[1]
        .as_object_mut()
        .ok_or("window missing")?
        .remove("liveBrowserContextId");
    native.lock().tabs[0]
        .as_object_mut()
        .ok_or("tab missing")?
        .remove("browserProfileId");
    let scoped = view(native.clone(), None).await;
    assert!(scoped.list_pages().await?.pages.is_empty());
    let popup = page_id(&scoped, 3).await?;
    code(
        scoped
            .close_page(popup)
            .await
            .err()
            .ok_or("popup admitted")?,
        "page_out_of_scope",
    );
    code(
        scoped
            .close_window(WindowId(20))
            .await
            .err()
            .ok_or("foreign window admitted")?,
        "window_out_of_scope",
    );
    assert!(
        native.calls("Browser.closeTab").is_empty()
            && native.calls("Browser.closeWindow").is_empty()
    );
    Ok(())
}
#[tokio::test]
async fn profile_root_methods_route_only_genuine_context_and_page_raw_is_explicit() -> TestResult {
    let native = Native::new();
    let scoped = view(native.clone(), None).await;
    for method in [
        "History.search",
        "History.getRecent",
        "History.deleteUrl",
        "History.deleteRange",
        "Storage.getCookies",
        "Storage.setCookies",
        "Storage.clearCookies",
        "Browser.setPermission",
        "Browser.grantPermissions",
        "Browser.resetPermissions",
        "Browser.setDownloadBehavior",
    ] {
        let result = scoped.cdp(method, json!({}), None).await?;
        assert_eq!(result["routedContext"], "live-eng");
    }
    code(
        scoped
            .cdp(
                "Storage.getCookies",
                json!({"browserContextId":"live-sales"}),
                None,
            )
            .await
            .err()
            .ok_or("conflict admitted")?,
        "target_scope_unsupported",
    );
    let page = page_id(&scoped, 1).await?;
    let session = scoped.page_session(page.clone()).await?;
    assert!(
        session
            .session
            .send_value("Runtime.evaluate", json!({"expression":"1"}))
            .await?["result"]["value"]
            .as_str()
            .is_some_and(|value| value.contains("target-1"))
    );
    for method in [
        "History.getRecent",
        "Storage.getCookies",
        "Browser.close",
        "Target.attachToTarget",
        "Runtime.unknownFutureOperation",
        "Emulation.setGeolocationOverride",
    ] {
        code(
            session
                .session
                .send_value(method, json!({}))
                .await
                .err()
                .ok_or("global/unknown page method admitted")?,
            "target_scope_unsupported",
        );
    }
    code(
        scoped
            .cdp_json_for_page(
                page,
                "Runtime.evaluate",
                "{\"expression\":\"1\",\"browserContextId\":\"live-sales\"}",
            )
            .await
            .err()
            .ok_or("page context override admitted")?,
        "target_scope_unsupported",
    );
    code(
        scoped
            .cdp(
                "Runtime.evaluate",
                json!({}),
                Some(&SessionId::from("unknown")),
            )
            .await
            .err()
            .ok_or("foreign session admitted")?,
        "target_scope_unsupported",
    );
    Ok(())
}
#[tokio::test]
async fn raw_discovery_projects_actual_page_ownership_and_excludes_workers() -> TestResult {
    let scoped = view(
        Native::new(),
        Some(WindowSelection::Pinned {
            window_id: WindowId(11),
        }),
    )
    .await;
    let targets = scoped.cdp("Target.getTargets", json!({}), None).await?;
    assert_eq!(
        targets["targetInfos"]
            .as_array()
            .ok_or("targets missing")?
            .len(),
        1
    );
    assert_eq!(targets["targetInfos"][0]["targetId"], "target-1");
    assert_eq!(
        scoped.cdp("Browser.getTabs", json!({}), None).await?["tabs"]
            .as_array()
            .ok_or("tabs missing")?
            .len(),
        1
    );
    Ok(())
}
#[tokio::test]
async fn group_move_validates_destination_and_members_before_dispatch() -> TestResult {
    let native = Native::new();
    let scoped = view(native.clone(), None).await;
    code(
        scoped
            .cdp(
                "Browser.moveTabGroup",
                json!({"groupId":"eng-group","windowId":20}),
                None,
            )
            .await
            .err()
            .ok_or("foreign destination admitted")?,
        "window_out_of_scope",
    );
    assert!(native.calls("Browser.moveTabGroup").is_empty());
    let result = scoped
        .cdp(
            "Browser.moveTabGroup",
            json!({"groupId":"eng-group","windowId":12}),
            None,
        )
        .await?;
    assert_eq!(result["group"]["windowId"], 12);
    assert_eq!(native.calls("Browser.moveTabGroup").len(), 1);
    code(
        scoped
            .new_page(
                "about:blank",
                NewPageOptions {
                    window_id: Some(WindowId(11)),
                    tab_group_id: Some("eng-group".into()),
                    ..NewPageOptions::default()
                },
            )
            .await
            .err()
            .ok_or("wrong-window group admitted")?,
        "group_out_of_scope",
    );
    assert!(native.calls("Browser.createTab").is_empty());
    Ok(())
}
#[tokio::test]
async fn retained_sessions_observer_input_and_navigation_fail_after_move_or_replacement()
-> TestResult {
    let native = Native::new();
    let scoped = view(
        native.clone(),
        Some(WindowSelection::Pinned {
            window_id: WindowId(11),
        }),
    )
    .await;
    let page = page_id(&scoped, 1).await?;
    let session = scoped.page_session(page.clone()).await?;
    let observer = scoped.observe(page.clone()).await?;
    let input = scoped.input(page.clone()).await?;
    let nav = scoped.nav(page).await?;
    native.lock().tabs[0]["windowId"] = json!(12);
    code(
        session
            .session
            .send_value("Runtime.evaluate", json!({}))
            .await
            .err()
            .ok_or("retained session admitted")?,
        "page_out_of_scope",
    );
    code(
        observer
            .snapshot()
            .await
            .err()
            .ok_or("retained observer admitted")?,
        "page_out_of_scope",
    );
    code(
        input
            .click_at(1.0, 2.0, ClickOptions::default())
            .await
            .err()
            .ok_or("retained input admitted")?,
        "page_out_of_scope",
    );
    code(
        nav.reload().await.err().ok_or("retained nav admitted")?,
        "page_out_of_scope",
    );
    assert!(
        native.calls("Runtime.evaluate").is_empty()
            && native.calls("Input.dispatchMouseEvent").is_empty()
            && native.calls("Page.reload").is_empty()
    );
    native.lock().tabs[0]["windowId"] = json!(11);
    native.lock().tabs[0]["targetId"] = json!("replacement");
    code(
        session
            .session
            .send_value("Runtime.evaluate", json!({}))
            .await
            .err()
            .ok_or("replacement adopted")?,
        "page_target_changed",
    );
    Ok(())
}
#[tokio::test]
async fn context_replacement_and_epoch_changes_never_refresh_a_capture() -> TestResult {
    let native = Native::new();
    let scoped = view(native.clone(), None).await;
    let page = page_id(&scoped, 1).await?;
    let retained = scoped.page_session(page).await?;
    native.lock().profiles[0]["browserContextId"] = json!("replacement-context");
    code(
        retained
            .session
            .send_value("Runtime.evaluate", json!({}))
            .await
            .err()
            .ok_or("new context adopted")?,
        "profile_gone",
    );
    native.lock().profiles[0]["browserContextId"] = json!("live-eng");
    native.epoch.store(2, Ordering::SeqCst);
    code(
        scoped.list_pages().await.err().ok_or("new epoch adopted")?,
        "profile_gone",
    );
    assert!(native.calls("Runtime.evaluate").is_empty());
    Ok(())
}
#[tokio::test]
async fn closed_strict_pin_fails_but_management_status_and_selection_recover() -> TestResult {
    let native = Native::new();
    let scoped = view(
        native.clone(),
        Some(WindowSelection::Pinned {
            window_id: WindowId(11),
        }),
    )
    .await;
    scoped.close_window(WindowId(11)).await?;
    code(
        scoped
            .list_pages()
            .await
            .err()
            .ok_or("closed pin admitted")?,
        "pinned_window_closed",
    );
    assert!(
        scoped
            .status(&binding(Some(WindowSelection::Pinned {
                window_id: WindowId(11)
            })))
            .await
            .error
            .is_some_and(|error| error.starts_with("pinned_window_closed"))
    );
    assert_eq!(scoped.list_profiles().await?.len(), 3);
    let intent = scoped
        .resolve_intent(&ProfileSelector::Name("eng".into()), None)
        .await?;
    assert_eq!(intent.browser_profile_id, BrowserProfileId("eng".into()));
    Ok(())
}
#[tokio::test]
async fn closed_working_window_does_not_redirect_an_existing_capture() -> TestResult {
    let native = Native::new();
    let selected = Some(WindowSelection::Working {
        window_id: WindowId(11),
    });
    let scoped = view(native.clone(), selected).await;
    native
        .lock()
        .windows
        .retain(|window| window["windowId"] != 11);
    code(
        scoped
            .new_page("about:blank", NewPageOptions::default())
            .await
            .err()
            .ok_or("placement redirected")?,
        "window_out_of_scope",
    );
    assert_eq!(
        view(native, selected)
            .await
            .scope()?
            .and_then(|scope| scope.placement_window),
        Some(WindowId(12))
    );
    Ok(())
}
#[tokio::test]
async fn unloaded_capture_can_load_but_only_a_new_capture_can_execute() -> TestResult {
    let native = Native::new();
    native.lock().profiles[0] = profile("eng", false);
    native
        .lock()
        .windows
        .retain(|window| window["browserProfileId"] != "eng");
    let scoped = view(native.clone(), None).await;
    code(
        scoped
            .cdp("Storage.getCookies", json!({}), None)
            .await
            .err()
            .ok_or("unloaded context admitted")?,
        "profile_not_loaded",
    );
    assert!(
        scoped
            .load_profile(&BrowserProfileId("eng".into()))
            .await?
            .loaded
    );
    code(
        scoped
            .cdp("Storage.getCookies", json!({}), None)
            .await
            .err()
            .ok_or("in-run load admitted cookie read")?,
        "profile_not_loaded",
    );
    let captured = scoped.scope()?.ok_or("bound scope missing")?;
    assert!(!captured.profile.loaded);
    assert!(captured.context_id.is_none());
    assert!(native.calls("Storage.getCookies").is_empty());
    code(
        scoped
            .create_window()
            .await
            .err()
            .ok_or("old unloaded capture refreshed")?,
        "profile_not_loaded",
    );
    let loaded = view(native.clone(), None).await;
    assert_eq!(
        loaded.cdp("Storage.getCookies", json!({}), None).await?["routedContext"],
        "live-eng"
    );
    code(
        loaded
            .new_page("about:blank", NewPageOptions::default())
            .await
            .err()
            .ok_or("window implicitly created")?,
        "no_profile_window",
    );
    assert!(native.calls("Browser.createWindow").is_empty());
    let window = loaded.create_window().await?;
    assert!(!window.is_active);
    code(
        loaded
            .new_page("about:blank", NewPageOptions::default())
            .await
            .err()
            .ok_or("placement refreshed mid-call")?,
        "no_profile_window",
    );
    assert_eq!(
        view(native, None)
            .await
            .new_page("about:blank", NewPageOptions::default())
            .await?
            .window_id,
        Some(WindowId(window.window_id))
    );
    Ok(())
}

#[tokio::test]
async fn unloaded_capture_prioritizes_missing_identity_and_epoch_over_not_loaded() -> TestResult {
    for invalidation in [
        "missing",
        "directory",
        "epoch_before",
        "epoch_during_catalog",
    ] {
        let native = Native::new();
        native.lock().profiles[0] = profile("eng", false);
        let scoped = view(native.clone(), None).await;
        scoped.load_profile(&BrowserProfileId("eng".into())).await?;
        match invalidation {
            "missing" => native
                .lock()
                .profiles
                .retain(|profile| profile["browserProfileId"] != "eng"),
            "directory" => native.lock().profiles[0]["directory"] = json!("reused-directory"),
            "epoch_before" => native.epoch.store(2, Ordering::SeqCst),
            // The catalog still returns this saved identity, but the awaited reply belongs
            // to a new connection. That invalidation must outrank the unloaded capture.
            "epoch_during_catalog" => native.lock().epoch_after_profiles = true,
            _ => unreachable!(),
        }
        let result = scoped.cdp("Storage.getCookies", json!({}), None).await;
        assert!(
            matches!(result, Err(CoreError::Target(TargetError::ProfileGone(id))) if id == BrowserProfileId("eng".into())),
            "{invalidation}"
        );
        assert!(
            native.calls("Storage.getCookies").is_empty(),
            "{invalidation}"
        );
        assert!(
            scoped
                .scope()?
                .ok_or("bound scope missing")?
                .context_id
                .is_none()
        );
    }
    Ok(())
}
#[tokio::test]
async fn facade_rebind_only_changes_later_calls_and_old_view_keeps_genuine_context() -> TestResult {
    let native = Native::new();
    let browser = Browser::new_with_native(native.clone(), current_native());
    browser.set_target(Some(binding(None))).await;
    let old = browser.view().await;
    let mut next = binding(None);
    next.intent.browser_profile_id = BrowserProfileId("sales".into());
    next.intent.directory = "directory-sales".into();
    next.intent.name = "sales".into();
    next.revision = 2;
    browser.set_target(Some(next)).await;
    assert_eq!(
        old.cdp("Storage.getCookies", json!({}), None).await?["routedContext"],
        "live-eng"
    );
    assert_eq!(
        browser
            .view()
            .await
            .cdp("Storage.getCookies", json!({}), None)
            .await?["routedContext"],
        "live-sales"
    );
    assert_eq!(browser.list_pages().await?.len(), 1);
    let page = browser
        .new_page("https://sales.test/new", NewPageOptions::default())
        .await?;
    assert_eq!(
        browser.get_page_session(page).await?.target_id,
        TargetId("target-5".into())
    );
    Ok(())
}
#[tokio::test]
async fn extension_adapter_returns_values_without_other_context_or_session_escape() -> TestResult {
    let native = Native::new();
    let scoped = view(native.clone(), None).await;
    assert_eq!(
        scoped.evaluate_extension_workers("1").await?,
        vec![json!("session-worker-eng")]
    );
    let attached = native.calls("Target.attachToTarget");
    assert_eq!(attached.len(), 1);
    assert_eq!(attached[0].params["targetId"], "worker-eng");
    assert_eq!(native.calls("Target.detachFromTarget").len(), 1);
    assert!(native.calls("Runtime.evaluate").iter().all(|call| {
        call.session
            .as_ref()
            .is_some_and(|id| id.as_str() == "session-worker-eng")
    }));
    Ok(())
}
#[tokio::test]
async fn method_specific_target_creation_translates_placement_and_rejects_unsupported_modes()
-> TestResult {
    let native = Native::new();
    let scoped = view(native.clone(), None).await;
    for params in [
        json!({"url":"about:blank","hidden":true}),
        json!({"url":"about:blank","forTab":true}),
        json!({"url":"about:blank","width":800}),
    ] {
        code(
            scoped
                .cdp("Target.createTarget", params, None)
                .await
                .err()
                .ok_or("unsupported mode admitted")?,
            "target_scope_unsupported",
        );
    }
    assert!(native.calls("Browser.createTab").is_empty());
    let created = scoped
        .cdp(
            "Target.createTarget",
            json!({"url":"https://raw.test"}),
            None,
        )
        .await?;
    assert_eq!(created["targetId"], "target-5");
    assert!(native.calls("Target.createTarget").is_empty());
    assert_eq!(native.calls("Browser.createTab")[0].params["windowId"], 12);
    Ok(())
}
#[tokio::test]
async fn admission_retries_one_target_replacement_and_rejects_a_second() -> TestResult {
    struct Permit;
    impl PageAdmission for Permit {
        fn admit(&self, _: &PageInfo) -> Result<(), CoreError> {
            Ok(())
        }
    }
    let native = Native::new();
    let manager = PageManager::new(native.clone(), PageManagerHooks::default());
    let page = manager.list().await?[0].page_id.clone();
    native
        .lock()
        .replace_on_attach
        .push_back("replacement-once".into());
    assert_eq!(
        manager
            .get_session_admitted(page.clone(), &Permit)
            .await?
            .target_id
            .as_str(),
        "replacement-once"
    );
    native.lock().replace_on_attach =
        VecDeque::from(["replacement-twice".into(), "replacement-third".into()]);
    // Drop the old attachment so both subsequent target changes occur while admission attaches.
    manager
        .detach_session(&SessionId::from("session-replacement-once"))
        .await;
    code(
        manager
            .get_session_admitted(page, &Permit)
            .await
            .err()
            .ok_or("second replacement admitted")?,
        "page_target_changed",
    );
    Ok(())
}
#[tokio::test]
async fn missing_profile_method_on_a_capable_identity_fails_closed_while_unbound_raw_is_unchanged()
-> TestResult {
    // A version mismatch (identity says capable, native lacks the method) keeps today's
    // native_missing backstop; it is not a substitute for the version gate.
    let native = Native::new();
    native.lock().missing_profiles = true;
    let scoped = view(native.clone(), None).await;
    assert_eq!(
        scoped.admission_error(),
        Some(&TargetError::UnsupportedNative)
    );
    let unbound = BrowserView::unbound(scoped.session.clone(), None);
    assert_eq!(
        unbound
            .cdp("Unknown.futureMethod", json!({"unchanged":true}), None)
            .await?,
        json!({})
    );
    assert_eq!(
        native.calls("Unknown.futureMethod")[0].params,
        json!({"unchanged":true})
    );
    Ok(())
}
#[tokio::test]
async fn post_dispatch_context_replacement_does_not_publish_a_redirected_result() -> TestResult {
    let native = Native::new();
    let scoped = view(native.clone(), None).await;
    native.lock().context_after_send = Some("replacement-after-send".into());
    code(
        scoped
            .cdp("Storage.getCookies", json!({}), None)
            .await
            .err()
            .ok_or("changed context result published")?,
        "profile_gone",
    );
    assert_eq!(
        native.calls("Storage.getCookies")[0].params["browserContextId"],
        "live-eng"
    );
    Ok(())
}
#[test]
fn exact_selector_ambiguity_and_saved_directory_guard_are_distinct() -> TestResult {
    let first: BrowserProfileInfo = decode(profile("eng", true))?;
    let mut second = first.clone();
    second.browser_profile_id = BrowserProfileId("other".into());
    second.directory = "other-directory".into();
    assert!(
        matches!(crate::resolve_profile(&[first.clone(),second],&ProfileSelector::Name("eng".into())),Err(TargetError::ProfileAmbiguous {candidates,..}) if candidates.len() == 2)
    );
    assert!(matches!(
        crate::resolve_profile(
            std::slice::from_ref(&first),
            &ProfileSelector::Name("ENG".into())
        ),
        Err(TargetError::ProfileNotFound(_))
    ));
    assert!(matches!(
        crate::resolve_profile(&[first], &ProfileSelector::Name("directory-eng".into())),
        Err(TargetError::ProfileNotFound(_))
    ));
    Ok(())
}

#[tokio::test]
async fn cancellation_while_native_attach_is_in_flight_still_detaches_without_evaluation()
-> TestResult {
    let native = Native::new();
    let scoped = view(native.clone(), None).await;
    let release = Arc::new(tokio::sync::Notify::new());
    *native
        .attach_hold
        .lock()
        .unwrap_or_else(|err| panic!("fixture poisoned: {err}")) = Some(release.clone());
    let caller = tokio::spawn(async move { scoped.evaluate_extension_workers("1").await });
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        native.attach_started.notified(),
    )
    .await?;
    caller.abort();
    assert!(caller.await.is_err());
    release.notify_one();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        native.detach_done.notified(),
    )
    .await?;
    assert_eq!(native.calls("Target.attachToTarget").len(), 1);
    assert_eq!(native.calls("Target.detachFromTarget").len(), 1);
    assert!(native.calls("Runtime.evaluate").is_empty());
    Ok(())
}

#[tokio::test]
async fn signal_projection_denies_foreign_reads_and_ownership_mutation() -> TestResult {
    let native = Native::new();
    let scoped = view(native.clone(), None).await;
    let own = page_id(&scoped, 1).await?;
    let foreign = page_id(&scoped, 4).await?;
    let own_session = scoped.page_session(own.clone()).await?;
    let foreign_session = scoped.session.pages.get_session(foreign.clone()).await?;
    scoped
        .signals()
        .attach_page(own.clone(), foreign_session.session_id.clone());
    let event = CdpEvent {
        method: "Page.javascriptDialogOpening".into(),
        params: json!({"type":"confirm","message":"foreign-secret","url":"https://sales.test"}),
        session_id: Some(foreign_session.session_id),
    };
    native.events.send(event)?;
    tokio::task::yield_now().await;
    assert_eq!(
        scoped
            .session
            .page_signals
            .pending_dialog(&foreign)
            .map(|dialog| dialog.message),
        Some("foreign-secret".into())
    );
    assert!(scoped.signals().pending_dialog(&foreign).is_none());
    assert!(scoped.signals().pending_dialog(&own).is_none());
    assert!(scoped.signals().console_entries(&foreign).is_empty());
    native.events.send(CdpEvent {
        method: "Page.javascriptDialogOpening".into(),
        params: json!({"type":"confirm","message":"own-message"}),
        session_id: Some(own_session.session_id),
    })?;
    tokio::task::yield_now().await;
    scoped.signals().forget_page(&own);
    assert_eq!(
        scoped
            .signals()
            .pending_dialog(&own)
            .map(|dialog| dialog.message),
        Some("own-message".into())
    );
    Ok(())
}

#[tokio::test]
async fn event_projection_omits_foreign_page_events_and_closes_on_stale_context() -> TestResult {
    let native = Native::new();
    let scoped = view(native.clone(), None).await;
    let own = scoped.page_session(page_id(&scoped, 1).await?).await?;
    let foreign = scoped
        .session
        .pages
        .get_session(page_id(&scoped, 4).await?)
        .await?;
    let mut events = scoped.cdp_events();
    native.events.send(CdpEvent {
        method: "Runtime.consoleAPICalled".into(),
        params: json!({"type":"log","args":[]}),
        session_id: Some(foreign.session_id),
    })?;
    native.events.send(CdpEvent {
        method: "Runtime.consoleAPICalled".into(),
        params: json!({"type":"log","args":[]}),
        session_id: Some(own.session_id.clone()),
    })?;
    let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv()).await??;
    assert_eq!(event.session_id, Some(own.session_id.clone()));
    native.epoch.store(2, Ordering::SeqCst);
    native.events.send(CdpEvent {
        method: "Runtime.consoleAPICalled".into(),
        params: json!({"type":"log","args":[]}),
        session_id: Some(own.session_id),
    })?;
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(1), events.recv()).await?,
        Err(broadcast::error::RecvError::Closed)
    ));
    Ok(())
}

#[tokio::test]
async fn navigation_load_wait_rechecks_scope_before_dialog_readback() -> TestResult {
    let native = Native::new();
    let scoped = view(
        native.clone(),
        Some(WindowSelection::Pinned {
            window_id: WindowId(11),
        }),
    )
    .await;
    let page = page_id(&scoped, 1).await?;
    let session = scoped.page_session(page.clone()).await?;
    let nav = scoped.nav(page.clone()).await?;
    let call = tokio::spawn(async move { nav.goto("https://eng.test/next").await });
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        native.navigation_sent.notified(),
    )
    .await?;
    native.lock().tabs[0]["windowId"] = json!(12);
    native.events.send(CdpEvent {
        method: "Page.javascriptDialogOpening".into(),
        params: json!({"type":"confirm","message":"moved-tab-secret"}),
        session_id: Some(session.session_id),
    })?;
    // The next load-wait iteration must report scope loss, rather than expose this dialog
    // from the shared event cache after the tab has left the strict pin.
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), call).await??;
    assert!(matches!(
        result,
        Err(CoreError::Target(TargetError::PageOutOfScope(_)))
    ));
    assert_eq!(
        scoped
            .session
            .page_signals
            .pending_dialog(&page)
            .map(|dialog| dialog.message),
        Some("moved-tab-secret".into())
    );
    Ok(())
}

#[tokio::test]
async fn group_drag_during_final_tab_read_is_rejected_before_close_dispatch() -> TestResult {
    let native = Native::new();
    let scoped = view(
        native.clone(),
        Some(WindowSelection::Pinned {
            window_id: WindowId(11),
        }),
    )
    .await;
    // The third read is the final tab check. Its reply describes the old owner, while the
    // group is dragged during that handoff; only a later group inventory sees the new window.
    native.lock().move_group_on_tab_read = Some((3, 12));
    let result = scoped
        .cdp(
            "Browser.closeTabGroup",
            json!({"groupId":"eng-group"}),
            None,
        )
        .await;
    assert!(matches!(
        result,
        Err(CoreError::Target(TargetError::GroupOutOfScope(_)))
    ));
    assert!(native.calls("Browser.closeTabGroup").is_empty());
    assert_eq!(native.lock().groups[0]["windowId"], 12);
    Ok(())
}

#[tokio::test]
async fn reconnect_after_admission_never_dispatches_root_page_raw_or_extension_work() -> TestResult
{
    // Pause inside the transport, after core's last admission. The replacement retains all
    // native IDs, so only forwarding the original epoch to atomic publication can reject it.
    for (case, method) in [
        ("root", "Browser.closeWindow"),
        ("page", "Runtime.evaluate"),
        ("raw", "Runtime.evaluate"),
        ("extension", "Runtime.evaluate"),
        ("extension_cleanup", "Target.detachFromTarget"),
        ("page_attach", "Target.attachToTarget"),
        ("page_setup", "Page.enable"),
        ("oopif_attach", "Target.attachToTarget"),
        ("oopif_setup", "DOM.enable"),
    ] {
        let native = Native::new();
        let scoped = view(native.clone(), None).await;
        let page = page_id(&scoped, 1).await?;
        let retained = if matches!(case, "page" | "raw" | "oopif_attach" | "oopif_setup") {
            Some(scoped.page_session(page.clone()).await?)
        } else {
            None
        };
        let before = native.calls(method).len();
        let release = Arc::new(tokio::sync::Notify::new());
        *native
            .dispatch_hold
            .lock()
            .unwrap_or_else(|err| panic!("fixture poisoned: {err}")) =
            Some((method.into(), release.clone()));
        let caller = tokio::spawn(async move {
            match case {
                "root" => scoped.close_window(WindowId(11)).await,
                "page" => retained
                    .ok_or_else(|| CoreError::Message("missing retained session".into()))?
                    .session
                    .send_value("Runtime.evaluate", json!({"expression":"1"}))
                    .await
                    .map(|_| ()),
                "raw" => scoped
                    .cdp_json_for_page(page, "Runtime.evaluate", r#"{"expression":"1"}"#)
                    .await
                    .map(|_| ()),
                "extension" | "extension_cleanup" => {
                    scoped.evaluate_extension_workers("1").await.map(|_| ())
                }
                "page_attach" | "page_setup" => scoped.page_session(page).await.map(|_| ()),
                "oopif_attach" | "oopif_setup" => scoped
                    .session
                    .frames()
                    .resolve_frame_target_with_epoch(
                        page,
                        Some(crate::FrameId("child-target".into())),
                        None,
                        true,
                        scoped.epoch_guard()?,
                    )
                    .await
                    .map(|_| ()),
                _ => Err(CoreError::Message("unknown regression case".into())),
            }
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            native.dispatch_started.notified(),
        )
        .await?;
        native.reconnect();
        release.notify_one();
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), caller).await??;
        assert!(
            matches!(result, Err(CoreError::Target(TargetError::ProfileGone(_)))),
            "{case}: {result:?}"
        );
        assert_eq!(
            native.calls(method).len(),
            before,
            "{case} published to the replacement"
        );
        assert!(
            native
                .lock()
                .windows
                .iter()
                .any(|window| window["windowId"] == 11)
        );
        if matches!(case, "extension" | "extension_cleanup") {
            assert!(
                native.calls("Target.detachFromTarget").is_empty(),
                "old worker session leaked onto replacement"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn created_page_constraint_survives_group_lookup_and_guards_every_group_write() -> TestResult
{
    for bound in [true, false] {
        let native = Native::new();
        let base = if bound {
            view(native.clone(), None).await
        } else {
            BrowserView::unbound(
                BrowserSession::new_with_native(
                    native.clone(),
                    crate::BrowserSessionHooks::default(),
                    current_native(),
                ),
                None,
            )
        };
        let page = base.page(page_id(&base, 1).await?).await?;
        let derived = base.with_expected_created_page(&page)?;
        let release = Arc::new(tokio::sync::Notify::new());
        *native
            .group_read_hold
            .lock()
            .unwrap_or_else(|err| panic!("fixture poisoned: {err}")) = Some(release.clone());
        let delayed = derived.clone();
        let lookup =
            tokio::spawn(async move { delayed.cdp("Browser.getTabGroups", json!({}), None).await });
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            native.group_read_started.notified(),
        )
        .await?;
        native.lock().tabs[0]["targetId"] = json!("replacement-with-same-tab-and-window");
        release.notify_one();
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), lookup).await??;
        assert!(
            matches!(
                result,
                Err(CoreError::Target(TargetError::PageTargetChanged(_)))
            ),
            "bound={bound}: {result:?}"
        );
        // Deliberately try every follow-up even after lookup failed. Title and expansion have
        // no page argument, so an entry-only guard or PageId-only final add cannot protect them.
        for (method, params) in [
            (
                "Browser.updateTabGroup",
                json!({"groupId":"eng-group","title":"pending title"}),
            ),
            (
                "Browser.updateTabGroup",
                json!({"groupId":"eng-group","collapsed":false}),
            ),
            (
                "Browser.createTabGroup",
                json!({"tabIds":[1],"title":"new group"}),
            ),
            (
                "Browser.addTabsToGroup",
                json!({"groupId":"eng-group","tabIds":[1]}),
            ),
            ("Browser.removeTabsFromGroup", json!({"tabIds":[1]})),
            ("Browser.closeTabGroup", json!({"groupId":"eng-group"})),
            (
                "Browser.moveTabGroup",
                json!({"groupId":"eng-group","windowId":12}),
            ),
        ] {
            let result = derived.clone().cdp(method, params, None).await;
            assert!(
                matches!(
                    result,
                    Err(CoreError::Target(TargetError::PageTargetChanged(_)))
                ),
                "bound={bound}, {method}: {result:?}"
            );
            assert!(
                native.calls(method).is_empty(),
                "bound={bound}, {method} mutated replacement"
            );
        }
        let replacement = base.page(page.page_id.clone()).await?;
        assert_eq!(replacement.tab_id, page.tab_id);
        assert_eq!(replacement.window_id, page.window_id);
        assert!(
            derived.with_expected_created_page(&replacement).is_err(),
            "derived view adopted a new expectation"
        );
    }
    Ok(())
}

#[tokio::test]
async fn unbound_created_page_retains_creation_epoch_before_delayed_callback() -> TestResult {
    let native = Native::new();
    let base = BrowserView::unbound(
        BrowserSession::new_with_native(
            native.clone(),
            crate::BrowserSessionHooks::default(),
            current_native(),
        ),
        None,
    );
    let created = base.page(page_id(&base, 1).await?).await?;
    let delayed = base.with_expected_created_page(&created)?;
    native.reconnect();
    let result = delayed
        .cdp(
            "Browser.updateTabGroup",
            json!({"groupId":"eng-group","title":"delayed"}),
            None,
        )
        .await;
    assert!(matches!(
        result,
        Err(CoreError::Target(TargetError::PageTargetChanged(_)))
    ));
    assert!(native.calls("Browser.updateTabGroup").is_empty());
    assert!(
        base.with_expected_created_page(&created).is_err(),
        "late derivation adopted the replacement epoch"
    );
    // Ordinary unbound calls still follow the live browser; only the creation clone is frozen.
    base.cdp(
        "Browser.updateTabGroup",
        json!({"groupId":"eng-group","title":"current"}),
        None,
    )
    .await?;
    assert_eq!(native.calls("Browser.updateTabGroup").len(), 1);
    Ok(())
}

#[tokio::test]
async fn created_page_constraint_allows_live_work_but_never_adopts_changed_tab_window_or_context()
-> TestResult {
    for changed in [
        "tabId",
        "windowId",
        "browserProfileId",
        "liveBrowserContextId",
    ] {
        let native = Native::new();
        let base = BrowserView::unbound(
            BrowserSession::new_with_native(
                native.clone(),
                crate::BrowserSessionHooks::default(),
                current_native(),
            ),
            None,
        );
        let created = base.page(page_id(&base, 1).await?).await?;
        let derived = base.with_expected_created_page(&created)?;
        derived
            .cdp(
                "Browser.updateTabGroup",
                json!({"groupId":"eng-group","title":"live title"}),
                None,
            )
            .await?;
        assert_eq!(native.calls("Browser.updateTabGroup").len(), 1);
        native.lock().tabs[0][changed] = match changed {
            "tabId" => json!(101),
            "windowId" => json!(12),
            "browserProfileId" => json!("sales"),
            _ => json!("live-sales"),
        };
        let result = derived
            .cdp(
                "Browser.updateTabGroup",
                json!({"groupId":"eng-group","collapsed":false}),
                None,
            )
            .await;
        assert!(
            matches!(
                result,
                Err(CoreError::Target(TargetError::PageTargetChanged(_)))
            ),
            "{changed}: {result:?}"
        );
        assert_eq!(
            native.calls("Browser.updateTabGroup").len(),
            1,
            "{changed} authorized a new incarnation"
        );
    }
    Ok(())
}

struct LegacyTransport(Arc<Native>);
impl CdpConnection for LegacyTransport {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        self.0.send(method, params, session)
    }
    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        self.0.send_raw_json(method, params, session)
    }
    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.0.events()
    }
    fn is_connected(&self) -> bool {
        self.0.is_connected()
    }
    fn connection_epoch(&self) -> u64 {
        self.0.connection_epoch()
    }
}

#[tokio::test]
async fn transports_without_atomic_epoch_support_fail_without_value_or_raw_dispatch() -> TestResult
{
    let native = Native::new();
    let legacy: Arc<dyn CdpConnection> = Arc::new(LegacyTransport(native.clone()));
    let guarded = crate::ProtocolSession::root(legacy).with_epoch_guard(Some(
        crate::connection::EpochGuard {
            expected: 1,
            invalidated: TargetError::ProfileGone(BrowserProfileId("eng".into())),
        },
    ));
    for result in [
        guarded
            .send_value("Browser.closeWindow", json!({"windowId":11}))
            .await
            .map(|_| ()),
        guarded
            .send_raw_json("Browser.closeWindow", r#"{"windowId":11}"#)
            .await
            .map(|_| ()),
    ] {
        let error = result.err().ok_or("unsafe fallback dispatched")?;
        assert!(
            error.to_string().starts_with("target_scope_unsupported:"),
            "{error}"
        );
    }
    assert!(native.calls("Browser.closeWindow").is_empty());
    assert!(
        native
            .lock()
            .windows
            .iter()
            .any(|window| window["windowId"] == 11)
    );
    Ok(())
}

async fn creation_receipt_before_handoff(bound: bool) -> TestResult {
    let native = Native::new();
    let base = if bound {
        view(
            native.clone(),
            Some(WindowSelection::Working {
                window_id: WindowId(11),
            }),
        )
        .await
    } else {
        BrowserView::unbound(
            BrowserSession::new_with_native(
                native.clone(),
                crate::BrowserSessionHooks::default(),
                current_native(),
            ),
            Some(WindowId(11)),
        )
    };
    // The hook's view clone may predate creation. Receipt ownership must be shared by those
    // clones, while registry reconciliation remains global and can rebind the reusable ID.
    let delayed_view = base.clone();
    let created = base
        .new_page("https://creation-receipt.test", NewPageOptions::default())
        .await?;
    let created_id = created.page_id.clone();
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let callback_started = started.clone();
    let callback_release = release.clone();
    let delayed = tokio::spawn(async move {
        // Models the ownership/response await BEFORE Claw reads get_info and derives its
        // capability. Keeping another early registry read would not protect this interval.
        callback_started.notify_one();
        callback_release.notified().await;
        let supplied = delayed_view
            .session
            .pages
            .get_info(created_id.clone())
            .await
            .ok_or(CoreError::UnknownPage(created_id))?;
        let derived = delayed_view.with_expected_created_page(&supplied)?;
        derived
            .cdp(
                "Browser.updateTabGroup",
                json!({"groupId":"eng-group","title":"pending title"}),
                None,
            )
            .await?;
        derived
            .cdp(
                "Browser.addTabsToGroup",
                json!({"groupId":"eng-group","tabIds":[supplied.tab_id.0]}),
                None,
            )
            .await?;
        Ok::<_, CoreError>(())
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), started.notified()).await?;
    {
        let mut state = native.lock();
        let tab = state
            .tabs
            .iter_mut()
            .find(|tab| tab["tabId"] == created.tab_id.0)
            .ok_or("created native tab missing")?;
        tab["targetId"] = json!("replacement-before-created-page-capture");
    }
    base.session.pages.list().await?;
    let rebound = base
        .session
        .pages
        .get_info(created.page_id.clone())
        .await
        .ok_or("reconciled page missing")?;
    assert_eq!(rebound.page_id, created.page_id);
    assert_eq!(rebound.tab_id, created.tab_id);
    assert_eq!(rebound.window_id, created.window_id);
    assert_ne!(rebound.target_id, created.target_id);
    release.notify_one();
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), delayed).await??;
    assert!(
        matches!(
            result,
            Err(CoreError::Target(TargetError::PageTargetChanged(_)))
        ),
        "bound={bound}: {result:?}; title writes={}",
        native.calls("Browser.updateTabGroup").len()
    );
    assert!(native.calls("Browser.updateTabGroup").is_empty());
    assert!(native.calls("Browser.addTabsToGroup").is_empty());

    // Rejection must not overwrite the receipt with the supplied replacement snapshot.
    {
        let mut state = native.lock();
        let tab = state
            .tabs
            .iter_mut()
            .find(|tab| tab["tabId"] == created.tab_id.0)
            .ok_or("created native tab missing")?;
        tab["targetId"] = json!(created.target_id.as_str());
    }
    base.session.pages.list().await?;
    let original = base
        .session
        .pages
        .get_info(created.page_id.clone())
        .await
        .ok_or("original page missing")?;
    assert!(
        base.clone().with_expected_created_page(&original).is_ok(),
        "receipt was overwritten after rejection"
    );
    Ok(())
}

#[tokio::test]
async fn bound_creation_receipt_rejects_reconciliation_before_handoff_capture() -> TestResult {
    creation_receipt_before_handoff(true).await
}

#[tokio::test]
async fn unbound_creation_receipt_rejects_reconciliation_before_handoff_capture() -> TestResult {
    creation_receipt_before_handoff(false).await
}

async fn creation_origin_during_readiness(bound: bool, reconnect: bool) -> TestResult {
    let native = Native::new();
    let base = if bound {
        view(
            native.clone(),
            Some(WindowSelection::Working {
                window_id: WindowId(11),
            }),
        )
        .await
    } else {
        BrowserView::unbound(
            BrowserSession::new_with_native(
                native.clone(),
                crate::BrowserSessionHooks::default(),
                current_native(),
            ),
            Some(WindowId(11)),
        )
    };
    let release = Arc::new(tokio::sync::Notify::new());
    *native
        .creation_read_hold
        .lock()
        .map_err(|_| "hold poisoned")? = Some(release.clone());
    let creating_view = base.clone();
    let creation = tokio::spawn(async move {
        creating_view
            .new_page(
                "https://creation-origin.test",
                NewPageOptions {
                    tab_group_id: Some("eng-group".into()),
                    ..NewPageOptions::default()
                },
            )
            .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        native.dispatch_started.notified(),
    )
    .await?;
    assert_eq!(native.calls("Browser.createTab").len(), 1);
    // Native creation has already returned target-5. Pause the first readiness dispatch,
    // before the implementation can derive an expectation from a mutable registry/readback.
    base.session.pages.list().await?;
    let original = base
        .session
        .pages
        .list()
        .await?
        .into_iter()
        .find(|page| page.tab_id == TabId(5))
        .ok_or("original creation missing")?;
    assert_eq!(original.target_id, TargetId("target-5".into()));
    if reconnect {
        native.reconnect();
    } else {
        let mut state = native.lock();
        let tab = state
            .tabs
            .iter_mut()
            .find(|tab| tab["tabId"] == 5)
            .ok_or("native created tab missing")?;
        tab["targetId"] = json!("replacement-before-creation-receipt");
    }
    base.session.pages.list().await?;
    if !reconnect {
        let replacement = base
            .session
            .pages
            .get_info(original.page_id.clone())
            .await
            .ok_or("reconciled creation missing")?;
        assert_eq!(replacement.page_id, original.page_id);
        assert_eq!(replacement.tab_id, original.tab_id);
        assert_eq!(replacement.window_id, original.window_id);
        assert_ne!(replacement.target_id, original.target_id);
    }
    release.notify_one();
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), creation).await??;
    let expected_error = if reconnect && bound {
        matches!(result, Err(CoreError::Target(TargetError::ProfileGone(_))))
    } else {
        matches!(
            result,
            Err(CoreError::Target(TargetError::PageTargetChanged(_)))
        )
    };
    assert!(
        expected_error,
        "bound={bound}, reconnect={reconnect}: {result:?}; default-group writes={}",
        native.calls("Browser.addTabsToGroup").len()
    );
    assert!(native.calls("Browser.addTabsToGroup").is_empty());
    assert!(native.calls("Browser.updateTabGroup").is_empty());
    assert!(native.calls("Browser.createTabGroup").is_empty());
    Ok(())
}

#[tokio::test]
async fn bound_creation_origin_rejects_replacement_during_first_readiness_read() -> TestResult {
    creation_origin_during_readiness(true, false).await
}

#[tokio::test]
async fn unbound_creation_origin_rejects_replacement_during_first_readiness_read() -> TestResult {
    creation_origin_during_readiness(false, false).await
}

#[tokio::test]
async fn bound_creation_origin_retains_epoch_during_readiness() -> TestResult {
    creation_origin_during_readiness(true, true).await
}

#[tokio::test]
async fn unbound_creation_origin_retains_epoch_during_readiness() -> TestResult {
    creation_origin_during_readiness(false, true).await
}

#[tokio::test]
async fn unbound_creation_origin_uses_creation_epoch_after_an_earlier_reconnect() -> TestResult {
    let native = Native::new();
    let base = BrowserView::unbound(
        BrowserSession::new_with_native(
            native.clone(),
            crate::BrowserSessionHooks::default(),
            current_native(),
        ),
        Some(WindowId(11)),
    );
    // An ordinary unbound view follows the live connection at each new call. Only the receipt
    // for the newly created page is frozen; using the view's old epoch rejects valid creation.
    native.reconnect();
    let created = base
        .new_page("https://creation-epoch.test", NewPageOptions::default())
        .await?;
    let derived = base.clone().with_expected_created_page(&created)?;
    derived
        .cdp(
            "Browser.updateTabGroup",
            json!({"groupId":"eng-group","title":"creation epoch"}),
            None,
        )
        .await?;
    assert_eq!(native.calls("Browser.updateTabGroup").len(), 1);
    native.reconnect();
    let result = derived
        .cdp(
            "Browser.updateTabGroup",
            json!({"groupId":"eng-group","collapsed":false}),
            None,
        )
        .await;
    assert!(matches!(
        result,
        Err(CoreError::Target(TargetError::PageTargetChanged(_)))
    ));
    assert_eq!(native.calls("Browser.updateTabGroup").len(), 1);
    Ok(())
}

#[tokio::test]
async fn bound_facade_close_uses_exclusive_native_target_selector() -> TestResult {
    let native = Native::new();
    let browser = Browser::new_with_native(native.clone(), current_native());
    browser.set_target(Some(binding(None))).await;
    let closing = browser
        .list_pages()
        .await?
        .into_iter()
        .find(|page| page.tab_id == TabId(1))
        .ok_or("bound facade page missing")?;
    browser.close_page(closing.page_id).await?;
    let calls = native.calls("Browser.closeTab");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].params, json!({"targetId":"target-1"}));
    assert!(!native.lock().tabs.iter().any(|tab| tab["tabId"] == 1));
    // Another profile's tab remains present; selector normalization must preserve admission.
    assert!(native.lock().tabs.iter().any(|tab| tab["tabId"] == 4));
    Ok(())
}

#[tokio::test]
async fn bound_tab_selector_translation_uses_one_admitted_target() -> TestResult {
    for method in [
        "Browser.getTabInfo",
        "Browser.activateTab",
        "Browser.showTab",
        "Browser.moveTab",
        "Browser.closeTab",
    ] {
        let native = Native::new();
        let scoped = view(native.clone(), None).await;
        let mut params = json!({"tabId":1});
        if method == "Browser.moveTab" {
            params["windowId"] = json!(12);
        }
        scoped.cdp(method, params, None).await?;
        let calls = native.calls(method);
        // Tab-info admission uses tabId internally, but the final caller request is canonical.
        let call = calls.last().ok_or("no native selector dispatch")?;
        assert_eq!(call.params["targetId"], "target-1", "{method}");
        assert!(call.params.get("tabId").is_none(), "{method}: {call:?}");
    }
    Ok(())
}

#[tokio::test]
async fn bound_conflicting_tab_selectors_fail_before_native_mutation() -> TestResult {
    let native = Native::new();
    let scoped = view(native.clone(), None).await;
    let result = scoped
        .cdp(
            "Browser.closeTab",
            json!({"tabId":1,"targetId":"target-2"}),
            None,
        )
        .await;
    assert!(matches!(
        result,
        Err(CoreError::Target(TargetError::CdpUnsupported { .. }))
    ));
    assert!(native.calls("Browser.closeTab").is_empty());
    Ok(())
}

#[tokio::test]
async fn unbound_creation_origin_preserves_best_effort_cross_window_group_rejection() -> TestResult
{
    let native = Native::new();
    let base = BrowserView::unbound(
        BrowserSession::new_with_native(
            native.clone(),
            crate::BrowserSessionHooks::default(),
            current_native(),
        ),
        Some(WindowId(12)),
    );
    let created = base
        .new_page(
            "https://cross-window-group.test",
            NewPageOptions {
                tab_group_id: Some("eng-group".into()),
                ..NewPageOptions::default()
            },
        )
        .await?;
    assert_eq!(created.target_id, TargetId("target-5".into()));
    assert_eq!(created.tab_id, TabId(5));
    assert_eq!(created.window_id, Some(WindowId(12)));
    assert_eq!(
        created.browser_profile_id,
        Some(BrowserProfileId("eng".into()))
    );
    assert!(created.group_id.is_none());
    assert_eq!(native.calls("Browser.addTabsToGroup").len(), 1);
    assert_eq!(native.lock().groups[0]["windowId"], 11);
    assert_eq!(native.lock().groups[0]["tabIds"], json!([1]));
    let actual = native
        .lock()
        .tabs
        .iter()
        .find(|tab| tab["tabId"] == 5)
        .cloned()
        .ok_or("new native tab missing")?;
    assert_eq!(actual["windowId"], 12);
    assert!(actual.get("groupId").is_none());
    assert!(base.clone().with_expected_created_page(&created).is_ok());
    Ok(())
}

#[tokio::test]
async fn bound_creation_origin_rejects_cross_window_default_group_before_create() -> TestResult {
    let native = Native::new();
    let scoped = view(native.clone(), None).await;
    let result = scoped
        .new_page(
            "https://cross-window-group.test",
            NewPageOptions {
                window_id: Some(WindowId(12)),
                tab_group_id: Some("eng-group".into()),
                ..NewPageOptions::default()
            },
        )
        .await;
    assert!(matches!(
        result,
        Err(CoreError::Target(TargetError::GroupOutOfScope(_)))
    ));
    assert!(native.calls("Browser.createTab").is_empty());
    assert!(native.calls("Browser.addTabsToGroup").is_empty());
    Ok(())
}

#[tokio::test]
async fn unbound_creation_origin_refreshes_successful_same_window_group_membership() -> TestResult {
    let native = Native::new();
    let base = BrowserView::unbound(
        BrowserSession::new_with_native(
            native.clone(),
            crate::BrowserSessionHooks::default(),
            current_native(),
        ),
        Some(WindowId(11)),
    );
    let created = base
        .new_page(
            "https://same-window-group.test",
            NewPageOptions {
                tab_group_id: Some("eng-group".into()),
                ..NewPageOptions::default()
            },
        )
        .await?;
    assert_eq!(created.target_id, TargetId("target-5".into()));
    assert_eq!(created.window_id, Some(WindowId(11)));
    assert_eq!(created.group_id.as_deref(), Some("eng-group"));
    assert_eq!(native.calls("Browser.addTabsToGroup").len(), 1);
    assert_eq!(native.lock().groups[0]["tabIds"], json!([1, 5]));
    assert!(base.clone().with_expected_created_page(&created).is_ok());
    Ok(())
}

#[tokio::test]
async fn native_without_profile_targeting_runs_a_saved_binding_unbound() -> TestResult {
    for caps in [native_version("0.51.1.2"), NativeCapabilities::default()] {
        let native = Native::new();
        let pin = WindowSelection::Pinned {
            window_id: WindowId(11),
        };
        let view = view_on(native.clone(), caps, Some(pin)).await;
        assert_eq!(view.admission_error(), None, "{caps}");
        assert!(view.binding().is_none());
        assert!(matches!(view.scope(), Ok(None)));
        assert_eq!(view.unapplied_binding(), Some(&binding(Some(pin))));

        // Ordinary work keeps the browser-wide routing older natives always had.
        let listing = view.list_pages().await?;
        assert_eq!(listing.pages.len(), 4);
        assert_eq!(listing.hidden, 0);
        let opened = view
            .new_page("https://new.test/", NewPageOptions::default())
            .await?;
        assert!(opened.window_id.is_some());
        let created = native.calls("Browser.createTab");
        assert_eq!(created.len(), 1);
        assert!(created[0].params.get("browserContextId").is_none());
        assert_eq!(
            view.cdp("Unknown.futureMethod", json!({"unchanged":true}), None)
                .await?,
            json!({})
        );

        // Gated operations are refused before they are sent.
        code(
            view.list_profiles().await.err().ok_or("profiles listed")?,
            "native_feature_unavailable",
        );
        code(
            view.intent_for_window(WindowSelection::Working {
                window_id: WindowId(11),
            })
            .await
            .err()
            .ok_or("window selected")?,
            "native_feature_unavailable",
        );
        for (method, params) in [
            ("Browser.getProfiles", json!({})),
            ("Browser.loadProfile", json!({"browserProfileId":"eng"})),
            ("History.getRecent", json!({"browserContextId":"live-eng"})),
            (
                "Browser.createTab",
                json!({"url":"about:blank","browserContextId":"live-eng"}),
            ),
            (
                "Browser.createWindow",
                json!({"browserContextId":"live-eng"}),
            ),
        ] {
            code(
                view.cdp(method, params, None)
                    .await
                    .err()
                    .ok_or("gated raw CDP sent")?,
                "native_feature_unavailable",
            );
        }
        let page = page_id(&view, 1).await?;
        code(
            view.cdp_json_for_page(page, "Browser.loadProfile", r#"{"browserProfileId":"eng"}"#)
                .await
                .err()
                .ok_or("gated page CDP sent")?,
            "native_feature_unavailable",
        );
        assert!(native.calls("Browser.getProfiles").is_empty());
        assert!(native.calls("Browser.loadProfile").is_empty());
        assert!(native.calls("History.getRecent").is_empty());
        assert_eq!(native.calls("Browser.createTab").len(), 1);
        assert!(native.calls("Browser.createWindow").is_empty());
    }
    Ok(())
}

#[tokio::test]
async fn stock_context_parameters_stay_allowed_on_an_older_native() -> TestResult {
    let native = Native::new();
    let view = view_on(native.clone(), native_version("0.51.1.2"), None).await;
    view.cdp(
        "Storage.getCookies",
        json!({"browserContextId":"otr"}),
        None,
    )
    .await?;
    view.cdp("History.getRecent", json!({"maxResults":3}), None)
        .await?;
    assert_eq!(
        native.calls("Storage.getCookies")[0].params,
        json!({"browserContextId":"otr"})
    );
    assert_eq!(
        native.calls("History.getRecent")[0].params,
        json!({"maxResults":3})
    );
    Ok(())
}

#[tokio::test]
async fn capable_native_retries_a_retained_binding_with_live_checks() -> TestResult {
    // Unchanged native identities: the retained binding captures targeted, and every capture
    // re-reads the live catalog instead of trusting the version gate or a cached catalog.
    let native = Native::new();
    let first = view_on(native.clone(), current_native(), None).await;
    assert!(first.scope()?.is_some());
    assert!(first.unapplied_binding().is_none());
    let reads = native.calls("Browser.getProfiles").len();
    assert!(reads > 0);
    let second = view_on(native.clone(), current_native(), None).await;
    assert!(second.scope()?.is_some());
    assert!(native.calls("Browser.getProfiles").len() > reads);

    // A restarted native minted new process-lifetime handles: the same retained binding
    // fails closed for explicit reselection instead of binding a different identity.
    let restarted = Native::new();
    restarted.lock().profiles = vec![profile("eng-after-restart", true)];
    let stale = view_on(restarted, current_native(), None).await;
    assert_eq!(
        stale.admission_error(),
        Some(&TargetError::ProfileGone(BrowserProfileId("eng".into())))
    );
    assert!(stale.unapplied_binding().is_none());
    Ok(())
}

#[tokio::test]
async fn typed_tab_creation_with_a_context_needs_profile_targeting() -> TestResult {
    // The typed API reaches the same gated createTab routing as raw CDP, so it is refused the
    // same way; creation without a context keeps working on the older native.
    let native = Native::new();
    let view = BrowserView::unbound(
        BrowserSession::new_with_native(
            native.clone(),
            crate::BrowserSessionHooks::default(),
            native_version("0.51.1.2"),
        ),
        None,
    );
    let contextual = NewPageOptions {
        browser_context_id: Some(BrowserContextId("live-eng".into())),
        ..NewPageOptions::default()
    };
    code(
        view.new_page("https://new.test/", contextual)
            .await
            .err()
            .ok_or("context creation sent")?,
        "native_feature_unavailable",
    );
    assert!(native.calls("Browser.createTab").is_empty());
    view.new_page("https://new.test/", NewPageOptions::default())
        .await?;
    assert_eq!(native.calls("Browser.createTab").len(), 1);
    Ok(())
}

/// A capable native whose original profile (directory "Default") was renamed "Work", while
/// another profile is display-named "Default" and a third profile owns the last-active window.
fn default_native(default_loaded: bool, default_window: bool) -> Arc<Native> {
    let native = Native::new();
    {
        let mut state = native.lock();
        state.profiles[0]["name"] = json!("Default");
        let mut original = json!({"browserProfileId":"original","name":"Work","directory":"Default","loaded":default_loaded});
        if default_loaded {
            original["browserContextId"] = json!("live-original");
        }
        state.profiles.push(original);
        if default_window {
            state.windows.push(window(30, "original", "normal", false));
            state.tabs.push(tab(31, 30, "original"));
        }
    }
    native
}
async fn default_view(native: Arc<Native>) -> BrowserView {
    BrowserView::capture(
        BrowserSession::new_with_native(
            native,
            crate::BrowserSessionHooks::default(),
            current_native(),
        ),
        None,
        None,
    )
    .await
}

#[tokio::test]
async fn no_target_uses_the_original_default_profile_not_focus_or_display_name() -> TestResult {
    let native = default_native(true, true);
    let view = default_view(native.clone()).await;
    assert_eq!(view.admission_error(), None);
    assert_eq!(view.target_source(), Some(TargetSource::Default));
    let effective = view.binding().ok_or("effective default binding missing")?;
    assert_eq!(effective.intent.directory, "Default");
    assert_eq!(effective.intent.browser_profile_id.as_str(), "original");
    assert_eq!(effective.revision, 0);

    // Discovery and active-page lookup see only Default's normal windows.
    let listing = view.list_pages().await?;
    assert_eq!(
        listing
            .pages
            .iter()
            .map(|page| page.tab_id.0)
            .collect::<Vec<_>>(),
        vec![31]
    );
    assert_eq!(view.list_windows().await?.len(), 1);

    // Creation lands in Default's window and context, never the focused sales window.
    let opened = view
        .new_page("https://new.test/", NewPageOptions::default())
        .await?;
    assert_eq!(opened.window_id, Some(WindowId(30)));
    let created = native.calls("Browser.createTab");
    assert_eq!(created[0].params["windowId"], json!(30));
    let window = view.create_window().await?;
    assert_eq!(
        native.calls("Browser.createWindow")[0].params["browserContextId"],
        json!("live-original")
    );
    assert!(window.window_id != 20);
    view.cdp(
        "Browser.createTab",
        json!({"url":"https://raw.test/"}),
        None,
    )
    .await?;
    assert_eq!(
        native.calls("Browser.createTab")[1].params["browserContextId"],
        json!("live-original")
    );
    view.cdp("History.getRecent", json!({"maxResults":2}), None)
        .await?;
    assert_eq!(
        native.calls("History.getRecent")[0].params["browserContextId"],
        json!("live-original")
    );
    // A page of another profile still needs explicit selection.
    let sales = page_id(&BrowserView::unbound(view.session.clone(), None), 4).await?;
    code(
        view.page(sales).await.err().ok_or("sales page admitted")?,
        "page_out_of_scope",
    );
    assert!(native.calls("Browser.loadProfile").is_empty());
    Ok(())
}

#[tokio::test]
async fn default_without_a_window_opens_an_inactive_normal_window_there() -> TestResult {
    let native = default_native(true, false);
    let view = default_view(native.clone()).await;
    assert_eq!(view.admission_error(), None);
    assert!(view.list_pages().await?.pages.is_empty());
    assert!(
        native.calls("Browser.createWindow").is_empty(),
        "listing opened a window"
    );

    let opened = view
        .new_page("https://first.test/", NewPageOptions::default())
        .await?;
    // First an inactive normal Default window, then the page created in it, so the page is
    // identified by its own createTab reply rather than a later lookup of the window.
    let windows = native.calls("Browser.createWindow");
    assert_eq!(windows.len(), 1);
    assert_eq!(
        windows[0].params["browserContextId"],
        json!("live-original")
    );
    assert_eq!(windows[0].params["windowType"], json!("normal"));
    assert!(windows[0].params.get("url").is_none());
    let created_window = native
        .lock()
        .windows
        .last()
        .cloned()
        .ok_or("window missing")?;
    assert_eq!(created_window["isActive"], json!(false));
    let tabs = native.calls("Browser.createTab");
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].params["windowId"], created_window["windowId"]);
    assert_eq!(tabs[0].params["url"], json!("https://first.test/"));
    assert_eq!(opened.url, "https://first.test/");
    assert_eq!(
        opened.window_id.map(|id| id.0),
        created_window["windowId"].as_i64()
    );
    assert_eq!(
        opened.browser_profile_id.as_ref().map(|id| id.as_str()),
        Some("original")
    );

    // Raw creation without a window takes the same path and keeps its own arguments
    // (no url, pinned); an explicit target does not.
    let raw_native = default_native(true, false);
    default_view(raw_native.clone())
        .await
        .cdp("Browser.createTab", json!({"pinned":true}), None)
        .await?;
    assert_eq!(raw_native.calls("Browser.createWindow").len(), 1);
    let raw_tabs = raw_native.calls("Browser.createTab");
    assert_eq!(raw_tabs.len(), 1);
    assert_eq!(raw_tabs[0].params["pinned"], json!(true));
    assert!(raw_tabs[0].params.get("url").is_none());
    assert_eq!(
        raw_tabs[0].params["browserContextId"],
        json!("live-original")
    );
    let explicit_native = default_native(true, false);
    let mut original = binding(None);
    original.intent.browser_profile_id = BrowserProfileId("original".into());
    original.intent.directory = "Default".into();
    let explicit = BrowserView::capture(
        BrowserSession::new_with_native(
            explicit_native.clone(),
            crate::BrowserSessionHooks::default(),
            current_native(),
        ),
        Some(original),
        None,
    )
    .await;
    code(
        explicit
            .new_page("https://explicit.test/", NewPageOptions::default())
            .await
            .err()
            .ok_or("explicit target created a window")?,
        "no_profile_window",
    );
    assert!(explicit_native.calls("Browser.createWindow").is_empty());
    Ok(())
}

#[tokio::test]
async fn unloaded_default_is_loaded_existing_only_before_admission() -> TestResult {
    let native = default_native(false, false);
    let view = default_view(native.clone()).await;
    assert_eq!(view.admission_error(), None);
    let loads = native.calls("Browser.loadProfile");
    assert_eq!(loads.len(), 1);
    assert_eq!(loads[0].params, json!({"browserProfileId":"original"}));
    let scope = view.scope()?.ok_or("default scope missing")?;
    assert_eq!(
        scope.context_id.as_ref().map(|id| id.as_str()),
        Some("live-original")
    );
    assert!(native.calls("Browser.createWindow").is_empty());
    assert!(native.calls("Browser.createTab").is_empty());
    Ok(())
}

#[tokio::test]
async fn unusable_default_fails_admission_without_using_another_profile() -> TestResult {
    let missing = Native::new();
    let failed_load = default_native(false, true);
    failed_load.lock().fail_load = true;
    for native in [missing, failed_load] {
        let view = default_view(native.clone()).await;
        assert!(
            matches!(
                view.admission_error(),
                Some(TargetError::DefaultProfileUnavailable(_))
            ),
            "{:?}",
            view.admission_error()
        );
        assert!(view.binding().is_none());
        assert_eq!(view.target_source(), Some(TargetSource::Default));
        code(
            view.new_page("https://nowhere.test/", NewPageOptions::default())
                .await
                .err()
                .ok_or("page created without a usable default")?,
            "default_profile_unavailable",
        );
        code(
            view.cdp(
                "Browser.createTab",
                json!({"url":"https://nowhere.test/"}),
                None,
            )
            .await
            .err()
            .ok_or("raw creation without a usable default")?,
            "default_profile_unavailable",
        );
        assert!(native.calls("Browser.createTab").is_empty());
        assert!(native.calls("Browser.createWindow").is_empty());
        // Management stays usable for explicit recovery.
        assert!(!view.list_profiles().await?.is_empty());
    }
    Ok(())
}

#[tokio::test]
async fn explicit_target_wins_over_the_default() -> TestResult {
    let native = default_native(true, true);
    let session = BrowserSession::new_with_native(
        native.clone(),
        crate::BrowserSessionHooks::default(),
        current_native(),
    );
    let explicit = BrowserView::capture(session.clone(), Some(binding(None)), None).await;
    assert_eq!(explicit.target_source(), Some(TargetSource::Explicit));
    let scope = explicit.scope()?.ok_or("explicit scope missing")?;
    assert_eq!(scope.profile.browser_profile_id.as_str(), "eng");
    assert!(
        explicit.list_pages().await?.pages.iter().all(|page| page
            .browser_profile_id
            .as_ref()
            .map(|id| id.as_str())
            == Some("eng"))
    );
    // With no saved binding the next capture is Default again.
    let cleared = BrowserView::capture(session, None, None).await;
    assert_eq!(
        cleared
            .scope()?
            .ok_or("default scope missing")?
            .profile
            .browser_profile_id
            .as_str(),
        "original"
    );
    Ok(())
}

#[tokio::test]
async fn default_handles_are_fresh_after_a_native_restart() -> TestResult {
    // The restarted native minted a new process-lifetime handle for the same directory.
    let restarted = default_native(true, true);
    {
        let mut state = restarted.lock();
        let original = state
            .profiles
            .iter_mut()
            .find(|profile| profile["directory"] == "Default")
            .ok_or("default missing")?;
        original["browserProfileId"] = json!("original-2");
    }
    let view = default_view(restarted.clone()).await;
    assert_eq!(
        view.binding()
            .map(|binding| binding.intent.browser_profile_id.as_str()),
        Some("original-2")
    );
    // An explicit binding saved with the old handle stays stale and needs reselection.
    let mut stale = binding(None);
    stale.intent.browser_profile_id = BrowserProfileId("original".into());
    stale.intent.directory = "Default".into();
    let explicit = BrowserView::capture(
        BrowserSession::new_with_native(
            restarted,
            crate::BrowserSessionHooks::default(),
            current_native(),
        ),
        Some(stale),
        None,
    )
    .await;
    assert_eq!(
        explicit.admission_error(),
        Some(&TargetError::ProfileGone(BrowserProfileId(
            "original".into()
        )))
    );
    Ok(())
}

#[tokio::test]
async fn admitted_default_view_fails_closed_after_a_reconnect() -> TestResult {
    let native = default_native(true, true);
    let view = default_view(native.clone()).await;
    assert_eq!(view.list_pages().await?.pages.len(), 1);
    native.epoch.store(2, Ordering::SeqCst);
    code(
        view.list_pages()
            .await
            .err()
            .ok_or("stale default view admitted")?,
        "default_profile_unavailable",
    );
    code(
        view.new_page("https://late.test/", NewPageOptions::default())
            .await
            .err()
            .ok_or("stale default view created a page")?,
        "default_profile_unavailable",
    );
    assert!(native.calls("Browser.createTab").is_empty());
    Ok(())
}

#[tokio::test]
async fn natives_without_profile_targeting_keep_unbound_routing_with_no_target() -> TestResult {
    for caps in [native_version("0.51.1.2"), NativeCapabilities::default()] {
        let native = default_native(false, true);
        let view = BrowserView::capture(
            BrowserSession::new_with_native(
                native.clone(),
                crate::BrowserSessionHooks::default(),
                caps,
            ),
            None,
            None,
        )
        .await;
        assert_eq!(view.target_source(), None, "{caps}");
        assert!(view.binding().is_none());
        assert_eq!(view.list_pages().await?.pages.len(), 5);
        view.new_page("https://legacy.test/", NewPageOptions::default())
            .await?;
        assert!(native.calls("Browser.getProfiles").is_empty());
        assert!(native.calls("Browser.loadProfile").is_empty());
        assert!(
            native
                .calls("Browser.createTab")
                .iter()
                .all(|call| call.params.get("browserContextId").is_none())
        );
    }
    Ok(())
}

#[tokio::test]
async fn management_capture_never_loads_the_default() -> TestResult {
    let native = default_native(false, false);
    let view = BrowserView::capture_for_management(
        BrowserSession::new_with_native(
            native.clone(),
            crate::BrowserSessionHooks::default(),
            current_native(),
        ),
        None,
        None,
    )
    .await;
    assert_eq!(view.target_source(), Some(TargetSource::Default));
    assert!(
        view.list_profiles()
            .await?
            .iter()
            .any(|profile| profile.directory == "Default")
    );
    let intent = view
        .resolve_intent(&ProfileSelector::Name("sales".into()), None)
        .await?;
    assert_eq!(intent.browser_profile_id.as_str(), "sales");
    assert!(native.calls("Browser.loadProfile").is_empty());
    Ok(())
}

#[tokio::test]
async fn facade_never_falls_back_to_global_routing_without_a_default() -> TestResult {
    // No profile has directory "Default": the facade must not treat the failed default as an
    // unbound view and hand out another profile's window or session.
    let native = Native::new();
    let browser = Browser::new_with_native(native.clone(), current_native());
    code(
        browser
            .get_active_page_for_window(WindowId(20))
            .await
            .err()
            .ok_or("unrestricted session returned")?,
        "default_profile_unavailable",
    );
    code(
        browser
            .new_page("https://nowhere.test/", NewPageOptions::default())
            .await
            .err()
            .ok_or("page created without a default")?,
        "default_profile_unavailable",
    );
    assert!(native.calls("Browser.createTab").is_empty());
    assert!(native.calls("Target.attachToTarget").is_empty());
    Ok(())
}

#[tokio::test]
async fn failed_default_view_exposes_no_global_events() -> TestResult {
    let native = Native::new();
    let view = default_view(native.clone()).await;
    assert!(view.admission_error().is_some());
    let mut events = view.cdp_events();
    let _ = native.events.send(CdpEvent {
        method: "Target.targetCreated".into(),
        params: json!({"targetInfo":{"targetId":"target-4","type":"page"}}),
        session_id: None,
    });
    let received = tokio::time::timeout(std::time::Duration::from_millis(200), events.recv()).await;
    assert!(
        !matches!(received, Ok(Ok(_))),
        "a view without a usable default relayed a global event"
    );
    Ok(())
}

#[tokio::test]
async fn one_admitted_default_view_opens_at_most_one_window() -> TestResult {
    // Default has no window. Several pages opened through the same admitted view, in order
    // or concurrently (Promise.all in a run), share the one window that view created.
    let native = default_native(true, false);
    let view = default_view(native.clone()).await;
    let first = view
        .new_page("https://one.test/", NewPageOptions::default())
        .await?;
    let second = view
        .new_page("https://two.test/", NewPageOptions::default())
        .await?;
    let (third, fourth) = tokio::join!(
        view.new_page("https://three.test/", NewPageOptions::default()),
        view.cdp(
            "Browser.createTab",
            json!({"url":"https://four.test/"}),
            None
        ),
    );
    let third = third?;
    fourth?;
    assert_eq!(native.calls("Browser.createWindow").len(), 1);
    assert_eq!(first.window_id, second.window_id);
    assert_eq!(first.window_id, third.window_id);
    let tabs = native.calls("Browser.createTab");
    assert_eq!(tabs.len(), 4);
    assert!(
        tabs.iter()
            .all(|call| call.params["windowId"].as_i64() == first.window_id.map(|id| id.0))
    );
    Ok(())
}

#[tokio::test]
async fn credential_inventory_uses_selected_live_profile_without_opening_a_page() -> TestResult {
    use crate::credentials::CredentialListOptions;
    let native = Native::new();
    let scoped = view_on(native.clone(), native_version("0.51.2.1"), None).await;
    native.lock().tabs.clear();
    native.lock().windows.clear();
    let inventory = scoped
        .list_credentials(None, CredentialListOptions::default())
        .await?;
    assert_eq!(inventory.browser_profile_id.as_str(), "eng");
    assert!(inventory.credentials.is_empty());
    let calls = native.calls("Autofill.listCredentials");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].params["browserContextId"], "live-eng");
    assert!(calls[0].params.get("documentId").is_none());
    assert!(calls[0].session.is_none());
    assert!(native.calls("Browser.createTab").is_empty());
    assert!(native.calls("Browser.createWindow").is_empty());
    assert!(native.calls("Target.attachToTarget").is_empty());
    Ok(())
}

#[tokio::test]
async fn older_native_cannot_dispatch_credentials_but_keeps_profile_support() -> TestResult {
    let native = Native::new();
    let scoped = view(native.clone(), None).await;
    assert_eq!(scoped.list_profiles().await?.len(), 3);
    let failure = scoped
        .list_credentials(None, crate::credentials::CredentialListOptions::default())
        .await
        .err()
        .ok_or("unexpected credential support")?;
    assert_eq!(failure.code, "not_supported");
    assert!(native.calls("Autofill.listCredentials").is_empty());
    Ok(())
}

#[tokio::test]
async fn credential_cancellation_reaches_original_session_after_tab_leaves_its_pin() -> TestResult {
    use crate::credentials::CredentialSource;
    let native = Native::new();
    let scoped = view_on(
        native.clone(),
        native_version("0.51.2.1"),
        Some(WindowSelection::Pinned {
            window_id: WindowId(11),
        }),
    )
    .await;
    let page = page_id(&scoped, 1).await?;
    let hold = Arc::new(tokio::sync::Notify::new());
    *native
        .dispatch_hold
        .lock()
        .unwrap_or_else(|error| panic!("fixture poisoned: {error}")) =
        Some(("Autofill.fillCredential".into(), hold));
    let caller = tokio::spawn(async move {
        scoped
            .fill_credential(
                page,
                CredentialSource::Provided {
                    username: "synthetic-user".into(),
                    password: "synthetic-password".into(),
                },
            )
            .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        native.dispatch_started.notified(),
    )
    .await?;
    native.lock().tabs[0]["windowId"] = json!(12);
    caller.abort();
    assert!(caller.await.is_err());
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while native
            .calls("Autofill.cancelCredentialOperation")
            .is_empty()
        {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    let cleanup = native.calls("Autofill.cancelCredentialOperation");
    let context = native.calls("Autofill.getCredentialContext");
    assert_eq!(cleanup[0].session, context[0].session);
    assert!(
        cleanup[0].params["operationId"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
    Ok(())
}
