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
}
struct Native {
    state: Mutex<Fixture>,
    epoch: AtomicU64,
    events: broadcast::Sender<CdpEvent>,
    attach_hold: Mutex<Option<Arc<tokio::sync::Notify>>>,
    attach_started: tokio::sync::Notify,
    detach_done: tokio::sync::Notify,
    navigation_sent: tokio::sync::Notify,
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
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CdpError> {
        let mut state = self.lock();
        state.calls.push(Call {
            method: method.into(),
            params: params.clone(),
            session: session.cloned(),
        });
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
                created["url"] = params["url"].clone();
                state.tabs.push(created.clone());
                json!({"tab":created})
            }
            "Browser.moveTab" => {
                let found = state
                    .tabs
                    .iter_mut()
                    .find(|tab| tab["tabId"] == params["tabId"])
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
        Box::pin(async move {
            let worker_attach = method == "Target.attachToTarget"
                && params["targetId"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("worker-"));
            let result = self.reply(method, params, session);
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
            if method == "Target.detachFromTarget" {
                self.detach_done.notify_one();
            }
            if method == "Page.navigate" {
                self.navigation_sent.notify_one();
            }
            result
        })
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
        BrowserSession::new(native, crate::BrowserSessionHooks::default()),
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
            .create_window()
            .await
            .err()
            .ok_or("old unloaded capture refreshed")?,
        "profile_gone",
    );
    let loaded = view(native.clone(), None).await;
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
async fn facade_rebind_only_changes_later_calls_and_old_view_keeps_genuine_context() -> TestResult {
    let native = Native::new();
    let browser = Browser::new(native.clone());
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
async fn old_native_is_explicitly_unsupported_while_unbound_raw_is_unchanged() -> TestResult {
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
