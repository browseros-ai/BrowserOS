//! Protocol-level wait contracts run through the real QuickJS bridge. The fake
//! emits events inside command replies to exercise subscribe-before-command races.

use crate::{
    framework::{BrowserToolOptions, InnerCallHook, InnerCallRecord, ToolCtx},
    output_file::create_browser_output_file_access,
    tools::run::{ScriptSpec, execute_script},
};
use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{BrowserSession, BrowserSessionHooks, CdpConnection, SessionId};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

// Keep the leaf tests independent of P3's facade implementation while exercising
// P0/P4 routing, timeout propagation, serialization and on_page_created ordering.
const BOOTSTRAP: &str = r#"
globalThis.__browserosBrowser = {};
globalThis.__browserosConsole = {};
globalThis.__browserosJsonSafeString = JSON.stringify;
globalThis.__browserosSafeStringify = String;
globalThis.__browserosMakeRunFunction = code => new (Object.getPrototypeOf(async function(){}).constructor)('browser', 'console', code);
globalThis.call = (method, args) => __browserosCall(method, JSON.stringify(args), false);
"#;

struct Fake {
    events: broadcast::Sender<CdpEvent>,
    commands: broadcast::Sender<String>,
    state: Mutex<State>,
}
struct State {
    url: String,
    loader: String,
    ready: &'static str,
    document_title: &'static str,
    automatic_load: bool,
    navigate_error: Option<&'static str>,
    new_tabs: Vec<Value>,
    on_enable: Vec<CdpEvent>,
    calls: Vec<(String, Value)>,
    function_value: Value,
}
impl Fake {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            events: broadcast::channel(256).0,
            commands: broadcast::channel(256).0,
            state: Mutex::new(State {
                url: "https://example.test/start".to_owned(),
                loader: "old".to_owned(),
                ready: "loading",
                document_title: "Wait test",
                automatic_load: true,
                navigate_error: None,
                new_tabs: Vec::new(),
                on_enable: Vec::new(),
                calls: Vec::new(),
                function_value: json!({"matched":false}),
            }),
        })
    }
    fn with_state<T>(&self, update: impl FnOnce(&mut State) -> T) -> T {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        update(&mut state)
    }
    fn emit(&self, method: &str, params: Value) {
        let _ = self.events.send(event(method, params));
    }
    fn load(&self, name: &str, loader: &str, frame: &str) {
        self.emit(
            "Page.lifecycleEvent",
            json!({"frameId":frame,"loaderId":loader,"name":name}),
        );
    }
    fn navigate(&self, url: &str) {
        self.with_state(|s| {
            s.url = url.to_owned();
            s.loader = "new".to_owned();
        });
        self.emit(
            "Page.frameNavigated",
            json!({"frame":{"id":"main","loaderId":"new","url":url}}),
        );
    }
    fn ctx(self: &Arc<Self>, hook: Option<Arc<dyn InnerCallHook>>) -> ToolCtx {
        ToolCtx::new(BrowserToolOptions {
            session: BrowserSession::new(self.clone(), BrowserSessionHooks::default()),
            defaults: Default::default(),
            cancel: CancellationToken::new(),
            output_files: create_browser_output_file_access(),
            inner_call_hook: hook,
            preloaded_helpers: Vec::new(),
        })
    }
}
fn event(method: &str, params: Value) -> CdpEvent {
    CdpEvent {
        method: method.to_owned(),
        params,
        session_id: Some(SessionId::from("session-1")),
    }
}
fn tab(id: i64, url: &str) -> Value {
    json!({"tabId":id,"targetId":format!("target-{id}"),"url":url,"title":"Wait test","isActive":false,"isLoading":false,"loadProgress":1.0,"isPinned":false,"isHidden":false,"windowId":1,"index":id})
}
impl CdpConnection for Fake {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            self.with_state(|s| s.calls.push((method.to_owned(), params.clone())));
            let result = match method {
                "Browser.getTabs" => self.with_state(|s| { let mut tabs = vec![tab(1, &s.url)]; tabs.extend(s.new_tabs.clone()); json!({"tabs":tabs}) }),
                "Browser.getTabInfo" => self.with_state(|s| json!({"tab":tab(1,&s.url)})),
                "Target.attachToTarget" => json!({"sessionId":"session-1"}),
                "Page.getFrameTree" => self.with_state(|s| json!({"frameTree":{"frame":{"id":"main","loaderId":s.loader,"url":s.url}}})),
                "Page.navigate" | "Page.reload" | "Page.navigateToHistoryEntry" => {
                    if let Some(err) = self.with_state(|s| s.navigate_error) { json!({"errorText":err}) }
                    else {
                        let url = params["url"].as_str().unwrap_or("https://example.test/reloaded");
                        self.navigate(url);
                        self.emit("Network.responseReceived", json!({"frameId":"main","loaderId":"new","type":"Document","response":{"status":200}}));
                        if self.with_state(|s| s.automatic_load) { self.load("load", "new", "main"); }
                        json!({"frameId":"main","loaderId":"new"})
                    }
                }
                "Page.getNavigationHistory" => json!({"currentIndex":0,"entries":[{"id":1,"url":"https://example.test/start"}]}),
                "Network.getResponseBody" => json!({"body":"{\"answer\":42}","base64Encoded":false}),
                "Runtime.evaluate" => self.with_state(|s| if params["expression"] == "document.readyState" { json!({"result":{"value":s.ready}}) } else if params["expression"] == "document.title" { json!({"result":{"value":s.document_title}}) } else { json!({"result":{"value":s.function_value}}) }),
                "Network.enable" | "Page.setDownloadBehavior" | "Page.setLifecycleEventsEnabled" | "Target.setDiscoverTargets" => {
                    for evt in self.with_state(|s| std::mem::take(&mut s.on_enable)) { let _ = self.events.send(evt); }
                    json!({})
                }
                "Page.enable" | "DOM.enable" | "Runtime.enable" | "Accessibility.enable" | "Target.setAutoAttach" | "Runtime.runIfWaitingForDebugger" | "Page.handleJavaScriptDialog" | "Emulation.setFocusEmulationEnabled" | "Runtime.releaseObject" => json!({}),
                _ => return Err(CdpError::Protocol { code: -1, message: format!("unexpected command {method}") }),
            };
            let _ = self.commands.send(method.to_owned());
            Ok(result)
        })
    }
    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            let params = serde_json::from_str(params).map_err(|err| CdpError::Protocol {
                code: -1,
                message: err.to_string(),
            })?;
            self.send(method, params, session)
                .await
                .map(|v| v.to_string())
        })
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
async fn script(ctx: &ToolCtx, code: &str, timeout_ms: u64) -> anyhow::Result<Value> {
    let result = execute_script(
        ScriptSpec {
            bootstrap_js: BOOTSTRAP,
            code: code.to_owned(),
            timeout_ms,
            helpers: false,
        },
        ctx,
    )
    .await
    .map_err(|err| anyhow::anyhow!("{err:?}"))?
    .into_tool_result();
    result
        .structured_content
        .ok_or_else(|| anyhow::anyhow!("missing script output"))
}
async fn wait_command(
    commands: &mut broadcast::Receiver<String>,
    method: &str,
) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if commands.recv().await? == method {
                return Ok::<_, broadcast::error::RecvError>(());
            }
        }
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn goto_subscribes_before_navigate_and_returns_status() -> anyhow::Result<()> {
    let fake = Fake::new();
    let value = script(
        &fake.ctx(None),
        "return await call('page.goto',[1,'https://example.test/done']);",
        2000,
    )
    .await?;
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(
        value["value"],
        json!({"url":"https://example.test/done","title":"Wait test","status":200})
    );
    Ok(())
}

#[tokio::test]
async fn goto_ignores_old_loaders_child_frames_and_domcontentloaded() -> anyhow::Result<()> {
    let fake = Fake::new();
    fake.with_state(|s| s.automatic_load = false);
    let ctx = fake.ctx(None);
    let mut commands = fake.commands.subscribe();
    let run = script(
        &ctx,
        "return await call('page.goto',[1,'https://example.test/done',{timeout:1000}]);",
        2000,
    );
    tokio::pin!(run);
    let driver = async {
        wait_command(&mut commands, "Page.navigate").await?;
        fake.load("load", "old", "main");
        fake.load("load", "new", "child");
        fake.load("DOMContentLoaded", "new", "main");
        Ok::<_, anyhow::Error>(())
    };
    tokio::select! { result = &mut run => panic!("completed before load: {result:?}"), result = driver => result? }
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut run)
            .await
            .is_err()
    );
    fake.load("load", "new", "main");
    let value = run.await?;
    assert_eq!(value["ok"], true, "{value}");
    Ok(())
}

#[tokio::test]
async fn goto_reports_network_errors() -> anyhow::Result<()> {
    let fake = Fake::new();
    fake.with_state(|s| s.navigate_error = Some("net::ERR_NAME_NOT_RESOLVED"));
    let value = script(
        &fake.ctx(None),
        "return await call('page.goto',[1,'https://missing.invalid']);",
        2000,
    )
    .await?;
    assert_eq!(value["ok"], false);
    assert!(
        value["error"]
            .as_str()
            .unwrap_or_default()
            .contains("page.goto: net::ERR_NAME_NOT_RESOLVED"),
        "{value}"
    );
    Ok(())
}

#[tokio::test]
async fn wait_for_url_matches_glob_after_navigation() -> anyhow::Result<()> {
    let fake = Fake::new();
    let ctx = fake.ctx(None);
    let mut commands = fake.commands.subscribe();
    let run = script(
        &ctx,
        "return await call('page.waitForURL',[1,'**/done',{timeout:1000}]);",
        2000,
    );
    let driver = async {
        wait_command(&mut commands, "Page.setLifecycleEventsEnabled").await?;
        fake.navigate("https://example.test/deep/done");
        fake.load("load", "new", "main");
        Ok::<_, anyhow::Error>(())
    };
    let (value, driver) = tokio::join!(run, driver);
    driver?;
    let value = value?;
    assert_eq!(value["ok"], true, "{value}");
    Ok(())
}

#[derive(Default)]
struct Hook(Mutex<Vec<String>>);
impl InnerCallHook for Hook {
    fn authorize<'a>(&'a self, _page: Option<u32>) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
    fn record<'a>(&'a self, record: InnerCallRecord<'a>) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(record.method.to_owned());
        })
    }
    fn on_page_created<'a>(&'a self, page: u32) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(format!("created:{page}"));
        })
    }
}
#[tokio::test]
async fn popup_sets_created_page_before_audit_and_ignores_unrelated_targets() -> anyhow::Result<()>
{
    let fake = Fake::new();
    let hook = Arc::new(Hook::default());
    let ctx = fake.ctx(Some(hook.clone()));
    let mut commands = fake.commands.subscribe();
    let run = script(
        &ctx,
        "return await call('page.waitForEvent',[1,'popup',null,{timeout:1000}]);",
        2000,
    );
    let driver = async {
        wait_command(&mut commands, "Target.setDiscoverTargets").await?;
        fake.with_state(|s| {
            s.new_tabs.extend([
                tab(2, "https://unrelated.test"),
                tab(3, "https://popup.test"),
            ])
        });
        fake.emit(
            "Target.targetCreated",
            json!({"targetInfo":{"type":"page","targetId":"target-2","openerId":"other"}}),
        );
        fake.emit(
            "Target.targetCreated",
            json!({"targetInfo":{"type":"page","targetId":"target-3","openerId":"target-1"}}),
        );
        Ok::<_, anyhow::Error>(())
    };
    let (value, driver) = tokio::join!(run, driver);
    driver?;
    let value = value?;
    assert_eq!(value["value"], 3, "{value}");
    assert_eq!(
        *hook.0.lock().unwrap_or_else(|p| p.into_inner()),
        vec!["created:3", "page.waitForEvent"]
    );
    Ok(())
}

#[tokio::test]
async fn dialog_reads_opening_event_and_accepts_prompt() -> anyhow::Result<()> {
    let fake = Fake::new();
    let ctx = fake.ctx(None);
    let mut commands = fake.commands.subscribe();
    let run = script(
        &ctx,
        "const dialog = await call('page.waitForEvent',[1,'dialog',null,{timeout:1000}]); await call('page.dialog',[1,true,'Ada']); return dialog;",
        2000,
    );
    let driver = async {
        wait_command(&mut commands, "Target.setAutoAttach").await?;
        fake.emit(
            "Page.javascriptDialogOpening",
            json!({"type":"prompt","message":"Your name?","defaultPrompt":"Guest"}),
        );
        Ok::<_, anyhow::Error>(())
    };
    let (value, driver) = tokio::join!(run, driver);
    driver?;
    let value = value?;
    assert_eq!(
        value["value"],
        json!({"type":"prompt","message":"Your name?","defaultValue":"Guest"}),
        "{value}"
    );
    assert!(fake.with_state(|s| {
        s.calls.iter().any(|(m, p)| {
            m == "Page.handleJavaScriptDialog" && p == &json!({"accept":true,"promptText":"Ada"})
        })
    }));
    Ok(())
}

#[tokio::test]
async fn download_correlates_progress_and_restores_behavior() -> anyhow::Result<()> {
    let fake = Fake::new();
    fake.with_state(|s| {
        s.on_enable = vec![
            event(
                "Page.downloadWillBegin",
                json!({"guid":"wanted","suggestedFilename":"report.csv"}),
            ),
            event(
                "Page.downloadProgress",
                json!({"guid":"other","state":"canceled"}),
            ),
            event(
                "Page.downloadProgress",
                json!({"guid":"wanted","state":"completed"}),
            ),
        ]
    });
    let ctx = fake.ctx(None);
    let value = script(
        &ctx,
        "return await call('page.waitForEvent',[1,'download',null,{timeout:1000}]);",
        2000,
    )
    .await?;
    assert_eq!(value["value"]["suggestedFilename"], "report.csv", "{value}");
    let path = value["value"]["path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing path: {value}"))?;
    assert!(
        ctx.output_files
            .lock()
            .await
            .contains(std::path::Path::new(path))
    );
    tokio::task::yield_now().await;
    assert!(fake.with_state(|s| {
        s.calls
            .iter()
            .any(|(m, p)| m == "Page.setDownloadBehavior" && p["behavior"] == "default")
    }));
    if let Some(parent) = std::path::Path::new(path).parent() {
        tokio::fs::remove_dir(parent).await?;
    }
    Ok(())
}

#[tokio::test]
async fn operation_timeout_has_playwright_shape() -> anyhow::Result<()> {
    let value = script(
        &Fake::new().ctx(None),
        "return await call('page.waitForURL',[1,'/done',{timeout:25}]);",
        2000,
    )
    .await?;
    assert_eq!(value["ok"], false);
    assert!(value["error"].as_str().unwrap_or_default().contains("TimeoutError: page.waitForURL: Timeout 25ms exceeded.\n=========================== logs ===========================\nwaiting for navigation to \"/done\" until \"load\""),"{value}");
    Ok(())
}

#[tokio::test]
async fn sixty_second_operation_is_clamped_to_run_budget() -> anyhow::Result<()> {
    let before = tokio::time::Instant::now();
    let value = script(
        &Fake::new().ctx(None),
        "return await call('page.waitForURL',[1,'**/never',{timeout:60000}]);",
        60,
    )
    .await?;
    assert_eq!(value["ok"], false, "{value}");
    assert!(before.elapsed() < Duration::from_secs(1));
    assert!(
        value["error"].as_str().unwrap_or_default().contains("60ms"),
        "{value}"
    );
    Ok(())
}

#[tokio::test]
async fn response_filter_accepts_js_predicate_and_preserves_body_request_id() -> anyhow::Result<()>
{
    let fake = Fake::new();
    fake.with_state(|s| s.on_enable = vec![
        event("Network.responseReceived",json!({"requestId":"bad","response":{"url":"https://example.test/api","status":404,"headers":{}}})),
        event("Network.responseReceived",json!({"requestId":"good","response":{"url":"https://example.test/api","status":200,"headers":{"content-type":"application/json"}}})),
    ]);
    let value = script(&fake.ctx(None),"return await call('page.waitForEvent',[1,'response','r => r.url().endsWith(\"/api\") && r.status() === 200',{timeout:1000}]);",2000).await?;
    assert_eq!(value["value"]["requestId"], "good", "{value}");
    assert_eq!(value["value"]["status"], 200);
    Ok(())
}

#[tokio::test]
async fn request_matches_regex_flags_and_ignores_other_sessions() -> anyhow::Result<()> {
    let fake = Fake::new();
    let mut unrelated = event(
        "Network.requestWillBeSent",
        json!({"requestId":"other","request":{"url":"https://example.test/API","headers":{}}}),
    );
    unrelated.session_id = Some(SessionId::from("session-2"));
    fake.with_state(|s|s.on_enable=vec![unrelated,event("Network.requestWillBeSent",json!({"requestId":"ours","request":{"url":"https://example.test/API","method":"GET","headers":{}}}))]);
    let value = script(
        &fake.ctx(None),
        "return await call('page.waitForEvent',[1,'request','/api$/i',{timeout:1000}]);",
        2000,
    )
    .await?;
    assert_eq!(value["value"]["requestId"], "ours", "{value}");
    Ok(())
}

#[tokio::test]
async fn function_wait_uses_main_world_and_returns_truthy_json() -> anyhow::Result<()> {
    let fake = Fake::new();
    fake.with_state(|s| s.function_value = json!({"matched":true,"value":{"ready":true}}));
    let value = script(&fake.ctx(None),"return await call('page.waitForFunction',[1,'arg => window.ready && arg',{ready:true},{polling:5,timeout:1000}]);",2000).await?;
    assert_eq!(value["value"], json!({"ready":true}), "{value}");
    assert!(
        fake.with_state(|s| s.calls.iter().any(|(m, p)| m == "Runtime.evaluate"
            && p.get("contextId").is_none()
            && p["awaitPromise"] == true))
    );
    Ok(())
}

#[tokio::test]
async fn go_back_without_history_returns_null() -> anyhow::Result<()> {
    let value = script(
        &Fake::new().ctx(None),
        "return await call('page.goBack',[1]);",
        2000,
    )
    .await?;
    assert_eq!(value["ok"], true, "{value}");
    assert!(value["value"].is_null());
    Ok(())
}

#[tokio::test]
async fn existing_load_state_and_commit_do_not_need_future_events() -> anyhow::Result<()> {
    let fake = Fake::new();
    fake.with_state(|s| s.ready = "complete");
    let value = script(&fake.ctx(None),"await call('page.waitForLoadState',[1,'load']); await call('page.goto',[1,'https://example.test/commit',{waitUntil:'commit'}]); return true;",2000).await?;
    assert_eq!(value["value"], true, "{value}");
    Ok(())
}

#[tokio::test]
async fn response_body_is_fetched_only_on_demand() -> anyhow::Result<()> {
    let fake = Fake::new();
    fake.with_state(|s| s.on_enable = vec![event("Network.responseReceived", json!({"requestId":"r1","response":{"url":"https://example.test/api","status":200,"headers":{}}}))]);
    let ctx = fake.ctx(None);
    let metadata = script(
        &ctx,
        "return await call('page.waitForEvent',[1,'response',null,{timeout:1000}]);",
        2000,
    )
    .await?;
    assert_eq!(metadata["value"]["requestId"], "r1", "{metadata}");
    assert!(!fake.with_state(|s| s.calls.iter().any(|(m, _)| m == "Network.getResponseBody")));
    let body = script(&ctx,"return await call('page.waitForEvent',[1,'response',{requestId:'r1'},{body:true,timeout:1000}]);",2000).await?;
    assert_eq!(
        body["value"],
        json!({"body":"{\"answer\":42}","base64Encoded":false}),
        "{body}"
    );
    Ok(())
}

#[tokio::test]
async fn networkidle_waits_for_idle_lifecycle_not_just_load() -> anyhow::Result<()> {
    let fake = Fake::new();
    let ctx = fake.ctx(None);
    let mut commands = fake.commands.subscribe();
    let run = script(
        &ctx,
        "return await call('page.goto',[1,'https://example.test/idle',{waitUntil:'networkidle',timeout:1000}]);",
        2000,
    );
    tokio::pin!(run);
    let driver = wait_command(&mut commands, "Page.navigate");
    tokio::select! { result = &mut run => panic!("completed before idle: {result:?}"), result = driver => result? }
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut run)
            .await
            .is_err()
    );
    fake.load("networkIdle", "new", "main");
    let value = run.await?;
    assert_eq!(value["ok"], true, "{value}");
    Ok(())
}

#[tokio::test]
async fn page_event_uses_tab_diff_without_an_opener() -> anyhow::Result<()> {
    let fake = Fake::new();
    let ctx = fake.ctx(None);
    let mut commands = fake.commands.subscribe();
    let run = script(
        &ctx,
        "return await call('page.waitForEvent',[1,'page',null,{timeout:1000}]);",
        2000,
    );
    let driver = async {
        wait_command(&mut commands, "Target.setDiscoverTargets").await?;
        fake.with_state(|s| s.new_tabs.push(tab(2, "https://new.test")));
        Ok::<_, anyhow::Error>(())
    };
    let (value, driver) = tokio::join!(run, driver);
    driver?;
    let value = value?;
    assert_eq!(value["value"], 2, "{value}");
    Ok(())
}

#[tokio::test]
async fn frame_navigation_returns_child_frame_summary() -> anyhow::Result<()> {
    let fake = Fake::new();
    let ctx = fake.ctx(None);
    let mut commands = fake.commands.subscribe();
    let run = script(
        &ctx,
        "return await call('page.waitForEvent',[1,'framenavigated',null,{timeout:1000}]);",
        2000,
    );
    let driver = async {
        wait_command(&mut commands, "Target.setAutoAttach").await?;
        fake.emit("Page.frameNavigated",json!({"frame":{"id":"child","parentId":"main","name":"embedded","url":"https://frame.test"}}));
        Ok::<_, anyhow::Error>(())
    };
    let (value, driver) = tokio::join!(run, driver);
    driver?;
    let value = value?;
    assert_eq!(
        value["value"],
        json!({"frameId":"child","parentFrameId":"main","name":"embedded","url":"https://frame.test"}),
        "{value}"
    );
    Ok(())
}

#[tokio::test]
async fn download_timeout_restores_browser_behavior() -> anyhow::Result<()> {
    let fake = Fake::new();
    let value = script(
        &fake.ctx(None),
        "return await call('page.waitForEvent',[1,'download',null,{timeout:25}]);",
        2000,
    )
    .await?;
    assert_eq!(value["ok"], false, "{value}");
    tokio::task::yield_now().await;
    let calls = fake.with_state(|s| s.calls.clone());
    assert!(
        calls
            .iter()
            .any(|(m, p)| m == "Page.setDownloadBehavior" && p["behavior"] == "default")
    );
    for (method, params) in calls {
        if method == "Page.setDownloadBehavior"
            && let Some(path) = params["downloadPath"].as_str()
        {
            tokio::fs::remove_dir(path).await?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn goto_returns_document_title_when_browser_tab_metadata_lags() -> anyhow::Result<()> {
    let fake = Fake::new();
    fake.with_state(|s| s.document_title = "Fresh document title");
    let value = script(
        &fake.ctx(None),
        "return await call('page.goto',[1,'https://example.test/title']);",
        2000,
    )
    .await?;
    assert_eq!(value["value"]["title"], "Fresh document title", "{value}");
    Ok(())
}
