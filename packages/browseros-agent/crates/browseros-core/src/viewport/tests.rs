//! Viewport behaviour through `BrowserView` against a fake browser. Its windows exist only so
//! tests can show that nothing reads or changes them. Real-Chromium acceptance lives in the live
//! harness.
use super::*;
use crate::{BrowserSession, BrowserSessionHooks, CdpConnection, SessionId};
use browseros_cdp::{CdpError, CdpEvent};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    error::Error,
    sync::{Arc, MutexGuard},
};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

type TestResult = Result<(), Box<dyn Error>>;

const CHROME_HEIGHT: i64 = 121;

#[derive(Clone, Debug, PartialEq, Eq)]
struct WindowBounds {
    width: i64,
    height: i64,
    window_state: String,
}

#[derive(Clone, Debug)]
struct Call {
    method: String,
    params: Value,
    session: Option<String>,
}

#[derive(Clone, Default)]
struct Emulated {
    metrics: Option<Value>,
    touch: bool,
    user_agent: String,
}

struct Fake {
    calls: Vec<Call>,
    tabs: Vec<Value>,
    windows: HashMap<i64, WindowBounds>,
    sessions: HashMap<String, String>,
    /// The live session per target. Overrides belong to a session and die with it.
    live: HashMap<String, String>,
    attached: u64,
    emulation: HashMap<String, Emulated>,
    ignore_metrics: bool,
    /// Page scripts override `screen` and `devicePixelRatio` in the main world.
    tampered: bool,
    /// `Emulation.clearDeviceMetricsOverride` fails, as on a page that just went away.
    fail_clear: bool,
    /// This method fails, as on a page that just went away.
    fail: Option<&'static str>,
    /// This method never answers, as when a caller's deadline cuts the call short.
    stall: Option<&'static str>,
    /// The page reports its natural screen on this many more checks before the emulated one.
    lagging_measures: u32,
    /// Calls to this method wait until the gate is opened; `parked` counts the waiting ones.
    gate: Option<(&'static str, Arc<tokio::sync::Notify>)>,
    parked: usize,
    chrome_version: String,
}

struct Native {
    state: std::sync::Mutex<Fake>,
    events: broadcast::Sender<CdpEvent>,
}

impl Native {
    fn new(tabs: &[(i64, i64)]) -> Arc<Self> {
        let mut windows = HashMap::new();
        for (_, window) in tabs {
            windows.insert(
                *window,
                WindowBounds {
                    width: 1200,
                    height: 1241,
                    window_state: "normal".to_string(),
                },
            );
        }
        let tabs = tabs
            .iter()
            .map(|(tab, window)| {
                json!({"tabId":tab,"targetId":format!("target-{tab}"),"windowId":window,
                    "url":format!("https://site.test/{tab}"),"title":"t","isActive":false,
                    "isLoading":false,"loadProgress":1.0,"isPinned":false,"isHidden":false})
            })
            .collect();
        let (events, _) = broadcast::channel(8);
        Arc::new(Self {
            state: std::sync::Mutex::new(Fake {
                calls: Vec::new(),
                tabs,
                windows,
                sessions: HashMap::new(),
                live: HashMap::new(),
                attached: 0,
                emulation: HashMap::new(),
                ignore_metrics: false,
                tampered: false,
                fail_clear: false,
                fail: None,
                stall: None,
                lagging_measures: 0,
                gate: None,
                parked: 0,
                chrome_version: "155.0.8315.26".to_string(),
            }),
            events,
        })
    }

    async fn stall(&self, method: &str) {
        let (stalled, gate) = {
            let mut fake = self.lock();
            let gate = fake
                .gate
                .as_ref()
                .filter(|(gated, _)| *gated == method)
                .map(|(_, gate)| gate.clone());
            if gate.is_some() {
                fake.parked += 1;
            }
            (fake.stall == Some(method), gate)
        };
        if stalled {
            std::future::pending::<()>().await;
        }
        if let Some(gate) = gate {
            gate.notified().await;
        }
    }

    fn lock(&self) -> MutexGuard<'_, Fake> {
        self.state
            .lock()
            .unwrap_or_else(|err| panic!("fake poisoned: {err}"))
    }

    fn calls(&self, method: &str) -> Vec<Call> {
        self.lock()
            .calls
            .iter()
            .filter(|call| call.method == method)
            .cloned()
            .collect()
    }

    fn methods(&self) -> Vec<String> {
        self.lock()
            .calls
            .iter()
            .map(|call| call.method.clone())
            .collect()
    }

    fn only(&self, method: &str) -> Result<Call, Box<dyn Error>> {
        let calls = self.calls(method);
        match calls.as_slice() {
            [call] => Ok(call.clone()),
            _ => Err(format!("expected one {method}, got {}", calls.len()).into()),
        }
    }

    fn clear_calls(&self) {
        self.lock().calls.clear();
    }

    fn window(&self, id: i64) -> Result<WindowBounds, Box<dyn Error>> {
        Ok(self
            .lock()
            .windows
            .get(&id)
            .cloned()
            .ok_or("unknown window")?)
    }

    /// What a person dragging the window edge does.
    fn resize(&self, id: i64, width: i64, height: i64) {
        if let Some(window) = self.lock().windows.get_mut(&id) {
            window.width = width;
            window.height = height;
        }
    }

    fn close_tab(&self, tab: i64) {
        self.lock().tabs.retain(|entry| entry["tabId"] != tab);
    }

    fn set_window_state(&self, id: i64, state: &str) {
        if let Some(window) = self.lock().windows.get_mut(&id) {
            window.window_state = state.to_string();
        }
    }

    fn window_of(fake: &Fake, target: &str) -> Result<i64, CdpError> {
        fake.tabs
            .iter()
            .find(|tab| tab["targetId"] == target)
            .and_then(|tab| tab["windowId"].as_i64())
            .ok_or(CdpError::SessionGone)
    }

    fn metrics(fake: &Fake, target: &str) -> Value {
        let emulated = fake
            .live
            .get(target)
            .and_then(|session| fake.emulation.get(session))
            .cloned()
            .unwrap_or_default();
        let touch_points = if emulated.touch { 5 } else { 0 };
        let mobile = emulated.user_agent.contains("Mobile");
        match emulated.metrics {
            Some(metrics) => json!({
                "width": metrics["width"], "height": metrics["height"],
                "screenWidth": metrics["screenWidth"], "screenHeight": metrics["screenHeight"],
                "deviceScaleFactor": metrics["deviceScaleFactor"], "touchPoints": touch_points,
                "mobile": mobile,
            }),
            None => {
                let window = Self::window_of(fake, target)
                    .ok()
                    .and_then(|id| fake.windows.get(&id).cloned());
                let (width, height) = window.map_or((0, 0), |window| {
                    (window.width, window.height - CHROME_HEIGHT)
                });
                json!({"width": width, "height": height, "screenWidth": 2056, "screenHeight": 1329,
                    "deviceScaleFactor": 2, "touchPoints": touch_points, "mobile": mobile})
            }
        }
    }

    fn reply(
        &self,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CdpError> {
        let mut fake = self.lock();
        fake.calls.push(Call {
            method: method.to_string(),
            params: params.clone(),
            session: session.map(|session| session.as_str().to_string()),
        });
        if fake.fail == Some(method) {
            return Err(CdpError::Protocol {
                code: -32000,
                message: "Target closed".into(),
            });
        }
        let session_key = session.map(|session| session.as_str().to_string());
        let target = session_key
            .as_ref()
            .and_then(|session| fake.sessions.get(session).cloned());
        Ok(match method {
            "Browser.getTabs" => json!({"tabs": fake.tabs}),
            "Browser.getTabInfo" => json!({"tab": fake.tabs.iter()
                .find(|tab| tab["tabId"] == params["tabId"]).ok_or(CdpError::SessionGone)?}),
            "Target.attachToTarget" => {
                fake.attached += 1;
                let id = format!("session-{}", fake.attached);
                let target = params["targetId"].as_str().unwrap_or_default().to_string();
                fake.sessions.insert(id.clone(), target.clone());
                fake.live.insert(target, id.clone());
                json!({"sessionId": id})
            }
            "Browser.getVersion" => {
                json!({"protocolVersion":"1.3","product":format!("Chrome/{}", fake.chrome_version),
                "revision":"@0","jsVersion":"15.5",
                "userAgent":"Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36"})
            }
            "Emulation.setDeviceMetricsOverride" => {
                if !fake.ignore_metrics {
                    let session = session_key.ok_or(CdpError::SessionGone)?;
                    fake.emulation.entry(session).or_default().metrics = Some(params);
                }
                json!({})
            }
            "Emulation.clearDeviceMetricsOverride" if fake.fail_clear => {
                return Err(CdpError::Protocol {
                    code: -32000,
                    message: "Target closed".into(),
                });
            }
            "Emulation.clearDeviceMetricsOverride" => {
                let session = session_key.ok_or(CdpError::SessionGone)?;
                fake.emulation.entry(session).or_default().metrics = None;
                json!({})
            }
            "Emulation.setTouchEmulationEnabled" => {
                let session = session_key.ok_or(CdpError::SessionGone)?;
                fake.emulation.entry(session).or_default().touch = params["enabled"] == true;
                json!({})
            }
            "Emulation.setUserAgentOverride" => {
                let session = session_key.ok_or(CdpError::SessionGone)?;
                fake.emulation.entry(session).or_default().user_agent =
                    params["userAgent"].as_str().unwrap_or_default().to_string();
                json!({})
            }
            "Page.getFrameTree" => {
                json!({"frameTree": {"frame": {"id": format!("frame-{}", target.unwrap_or_default())}}})
            }
            "Page.createIsolatedWorld" => json!({"executionContextId": 77}),
            "Runtime.evaluate" => match target {
                Some(target)
                    if params["expression"]
                        .as_str()
                        .is_some_and(|e| e.contains("screenWidth")) =>
                {
                    let mut value = Self::metrics(&fake, &target);
                    if fake.lagging_measures > 0 {
                        fake.lagging_measures -= 1;
                        value["screenWidth"] = json!(2056);
                    }
                    if fake.tampered && params.get("contextId").is_none() {
                        value["screenWidth"] = json!(123);
                        value["deviceScaleFactor"] = json!(7);
                    }
                    json!({"result": {"type": "object", "value": value}})
                }
                _ => json!({"result": {"type": "string", "value": "complete"}}),
            },
            _ => json!({}),
        })
    }
}

impl CdpConnection for Native {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            self.stall(method).await;
            self.reply(method, params, session)
        })
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            self.stall(method).await;
            let value = self.reply(method, serde_json::from_str(params_json)?, session)?;
            serde_json::to_string(&value).map_err(CdpError::from)
        })
    }

    fn send_at_epoch<'a>(
        &'a self,
        _expected: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            self.stall(method).await;
            self.reply(method, params, session)
        })
    }

    fn send_raw_json_at_epoch<'a>(
        &'a self,
        _expected: u64,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            self.stall(method).await;
            let value = self.reply(method, serde_json::from_str(params_json)?, session)?;
            serde_json::to_string(&value).map_err(CdpError::from)
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

struct Fixture {
    native: Arc<Native>,
    session: Arc<BrowserSession>,
    viewports: Arc<Viewports>,
    view: BrowserView,
}

impl Fixture {
    /// Tabs as (tab id, window id).
    fn new(tabs: &[(i64, i64)]) -> Self {
        let native = Native::new(tabs);
        let session = BrowserSession::new(native.clone(), BrowserSessionHooks::default());
        let viewports = Arc::new(Viewports::default());
        let view = BrowserView::unbound(session.clone(), None).with_viewports(viewports.clone());
        Self {
            native,
            session,
            viewports,
            view,
        }
    }

    async fn page(&self, tab: i64) -> Result<PageId, Box<dyn Error>> {
        let listing = self.view.list_pages().await?;
        Ok(listing
            .pages
            .into_iter()
            .find(|page| page.tab_id.0 == tab)
            .ok_or("tab not listed")?
            .page_id)
    }

    async fn set(
        &self,
        page: &PageId,
        mode: ViewportMode,
        zoom: Option<f64>,
    ) -> Result<ViewportReport, Box<dyn Error>> {
        Ok(self
            .view
            .set_viewport(
                page.clone(),
                ViewportRequest::new(mode, zoom)?,
                &CancellationToken::new(),
            )
            .await?)
    }
}

/// Every way to read or change a window. setViewport and page operations send none of them.
fn window_methods(methods: &[String]) -> Vec<String> {
    methods
        .iter()
        .filter(|method| {
            matches!(
                method.as_str(),
                "Browser.getWindowForTarget"
                    | "Browser.getWindowBounds"
                    | "Browser.setContentsSize"
                    | "Browser.setWindowBounds"
                    | "Emulation.getScreenInfos"
            )
        })
        .cloned()
        .collect()
}

fn emulation_methods(methods: &[String]) -> Vec<String> {
    methods
        .iter()
        .filter(|method| {
            method.starts_with("Emulation.set") || method.starts_with("Emulation.clear")
        })
        .cloned()
        .collect()
}

#[test]
fn tests_that_zoom_is_a_mobile_percentage_between_25_and_400() -> TestResult {
    assert_eq!(
        ViewportRequest::new(ViewportMode::Mobile, None)?.zoom(),
        Some(100.0)
    );
    for zoom in [25.0, 62.5, 400.0] {
        assert_eq!(
            ViewportRequest::new(ViewportMode::Mobile, Some(zoom))?.zoom(),
            Some(zoom)
        );
    }
    for zoom in [24.9, 400.1, 0.0, -50.0, f64::NAN, f64::INFINITY] {
        assert!(
            ViewportRequest::new(ViewportMode::Mobile, Some(zoom)).is_err(),
            "{zoom}"
        );
    }
    for mode in [ViewportMode::Desktop, ViewportMode::Default] {
        assert!(ViewportRequest::new(mode, Some(100.0)).is_err(), "{mode:?}");
        assert_eq!(ViewportRequest::new(mode, None)?.zoom(), None);
    }
    Ok(())
}

#[tokio::test]
async fn tests_that_mobile_emulates_the_device_and_leaves_the_window() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    let before = fixture.native.window(10)?;
    let report = fixture.set(&page, ViewportMode::Mobile, None).await?;

    assert_eq!(report.mode, ViewportMode::Mobile);
    assert_eq!(report.zoom, Some(100.0));
    let viewport = &report.viewport;
    assert_eq!(
        (
            viewport.width,
            viewport.height,
            viewport.device_scale_factor
        ),
        (400.0, 906.0, 2.0)
    );
    assert_eq!(viewport.mobile, Some(true));
    assert!(viewport.touch_points > 0);
    let metrics = fixture
        .native
        .only("Emulation.setDeviceMetricsOverride")?
        .params;
    assert_eq!(
        (
            &metrics["width"],
            &metrics["height"],
            &metrics["deviceScaleFactor"]
        ),
        (&json!(400), &json!(906), &json!(2.0))
    );
    assert_eq!(
        (&metrics["mobile"], &metrics["scale"]),
        (&json!(true), &json!(1.0))
    );
    // Hidden tabs keep their window-sized view, so zoomed pointer input stays inside it.
    assert_eq!(metrics["dontSetVisibleSize"], true);
    assert_eq!(
        fixture
            .native
            .only("Emulation.setTouchEmulationEnabled")?
            .params["enabled"],
        true
    );
    let identity = fixture
        .native
        .only("Emulation.setUserAgentOverride")?
        .params;
    let user_agent = identity["userAgent"].as_str().unwrap_or_default();
    assert!(
        user_agent.contains("Android") && user_agent.contains("Chrome/155.0.0.0 Mobile"),
        "{user_agent}"
    );
    assert_eq!(identity["userAgentMetadata"]["mobile"], true);
    assert_eq!(identity["userAgentMetadata"]["platform"], "Android");

    // The page renders at the device size inside whatever window it has.
    let methods = fixture.native.methods();
    assert!(window_methods(&methods).is_empty(), "{methods:?}");
    assert_eq!(fixture.native.window(10)?, before);
    assert_eq!(
        fixture.native.only("Emulation.setVisibleSize")?.params,
        json!({"width": 400, "height": 906})
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_zoom_scales_the_preview_and_never_the_page() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    let report = fixture
        .set(&page, ViewportMode::Mobile, Some(150.0))
        .await?;

    assert_eq!(report.zoom, Some(150.0));
    assert_eq!(
        (report.viewport.width, report.viewport.height),
        (400.0, 906.0)
    );
    assert_eq!(
        fixture
            .native
            .only("Emulation.setDeviceMetricsOverride")?
            .params["scale"],
        1.5
    );
    // The view holds the whole zoomed preview, even where the window is smaller.
    assert_eq!(
        fixture.native.only("Emulation.setVisibleSize")?.params,
        json!({"width": 600, "height": 1359})
    );
    assert!(window_methods(&fixture.native.methods()).is_empty());
    Ok(())
}

#[tokio::test]
async fn tests_that_desktop_uses_the_native_identity_without_touch() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    let report = fixture.set(&page, ViewportMode::Desktop, None).await?;

    assert_eq!((report.mode, report.zoom), (ViewportMode::Desktop, None));
    assert_eq!(
        (
            report.viewport.width,
            report.viewport.height,
            report.viewport.device_scale_factor
        ),
        (1920.0, 1080.0, 1.0)
    );
    let metrics = fixture
        .native
        .only("Emulation.setDeviceMetricsOverride")?
        .params;
    assert_eq!(
        (&metrics["width"], &metrics["height"], &metrics["mobile"]),
        (&json!(1920), &json!(1080), &json!(false))
    );
    assert_eq!(
        fixture
            .native
            .only("Emulation.setTouchEmulationEnabled")?
            .params["enabled"],
        false
    );
    assert_eq!(
        fixture
            .native
            .only("Emulation.setUserAgentOverride")?
            .params,
        json!({"userAgent": ""})
    );
    assert!(window_methods(&fixture.native.methods()).is_empty());
    Ok(())
}

#[tokio::test]
async fn tests_that_mobile_without_zoom_resets_a_previous_zoom() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    fixture
        .set(&page, ViewportMode::Mobile, Some(200.0))
        .await?;
    let report = fixture.set(&page, ViewportMode::Mobile, None).await?;

    assert_eq!(report.zoom, Some(100.0));
    let scales = fixture
        .native
        .calls("Emulation.setDeviceMetricsOverride")
        .into_iter()
        .map(|call| call.params["scale"].clone())
        .collect::<Vec<_>>();
    assert_eq!(scales, vec![json!(2.0), json!(1.0)]);
    Ok(())
}

#[tokio::test]
async fn tests_that_default_clears_only_our_overrides_and_leaves_the_window() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    let before = fixture.native.window(10)?;
    fixture.set(&page, ViewportMode::Mobile, None).await?;
    fixture.native.clear_calls();

    let report = fixture.set(&page, ViewportMode::Default, None).await?;
    assert_eq!((report.mode, report.zoom), (ViewportMode::Default, None));
    assert!(window_methods(&fixture.native.methods()).is_empty());
    assert_eq!(fixture.native.window(10)?, before);
    assert_eq!(
        emulation_methods(&fixture.native.methods()),
        vec![
            "Emulation.clearDeviceMetricsOverride",
            "Emulation.setTouchEmulationEnabled",
            "Emulation.setUserAgentOverride"
        ]
    );
    let clears = fixture.native.calls("Emulation.clearDeviceMetricsOverride");
    assert_eq!(clears[0].session.as_deref(), Some("session-1"));
    // Natural again: the page follows its window.
    assert_eq!(
        (report.viewport.width, report.viewport.height),
        (1200.0, 1120.0)
    );

    fixture.native.clear_calls();
    fixture.set(&page, ViewportMode::Default, None).await?;
    assert!(emulation_methods(&fixture.native.methods()).is_empty());
    Ok(())
}

#[tokio::test]
async fn tests_that_a_new_tab_is_default_and_untouched() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    let report = fixture.set(&page, ViewportMode::Default, None).await?;

    assert_eq!(report.mode, ViewportMode::Default);
    let methods = fixture.native.methods();
    assert!(emulation_methods(&methods).is_empty(), "{methods:?}");
    assert!(window_methods(&methods).is_empty(), "{methods:?}");
    assert_eq!(fixture.native.window(10)?.width, 1200);
    Ok(())
}

#[tokio::test]
async fn tests_that_a_fullscreen_or_maximized_window_stays_as_it_is() -> TestResult {
    for state in ["fullscreen", "maximized"] {
        let fixture = Fixture::new(&[(1, 10)]);
        fixture.native.set_window_state(10, state);
        let page = fixture.page(1).await?;
        let before = fixture.native.window(10)?;

        // A phone-sized owner view: the caller made the window fullscreen at the phone's size.
        let phone = ViewportRequest::new(ViewportMode::Mobile, None)?.with_size(Size {
            width: 390,
            height: 844,
        })?;
        fixture
            .view
            .set_viewport(page.clone(), phone, &CancellationToken::new())
            .await?;
        for _ in 0..3 {
            drop(fixture.view.prepare_viewport(page.clone()).await?);
        }
        fixture.set(&page, ViewportMode::Desktop, None).await?;
        drop(fixture.view.prepare_viewport(page.clone()).await?);
        fixture.set(&page, ViewportMode::Default, None).await?;

        let methods = fixture.native.methods();
        assert!(window_methods(&methods).is_empty(), "{state}: {methods:?}");
        assert_eq!(fixture.native.window(10)?, before, "{state}");
    }
    Ok(())
}

#[tokio::test]
async fn tests_that_a_page_ignoring_emulation_is_reported_as_a_failure() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    fixture.native.lock().ignore_metrics = true;
    let page = fixture.page(1).await?;
    let error = fixture
        .view
        .set_viewport(
            page,
            ViewportRequest::new(ViewportMode::Mobile, None)?,
            &CancellationToken::new(),
        )
        .await
        .err()
        .ok_or("a page that keeps its natural size must not report mobile")?;
    assert!(error.to_string().contains("mobile"), "{error}");
    Ok(())
}

#[tokio::test]
async fn tests_that_a_raw_override_on_a_fixed_tab_is_undone_by_its_next_operation() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    fixture
        .set(&page, ViewportMode::Mobile, Some(150.0))
        .await?;
    let session = fixture.view.page_session(page.clone()).await?.session_id;

    // A caller's raw calls share the page session with the mode: page.cdp, then browser.cdp
    // addressed to the page's session.
    for raw in ["page", "session"] {
        if raw == "page" {
            fixture
                .view
                .cdp_json_for_page(page.clone(), "Emulation.clearDeviceMetricsOverride", "{}")
                .await?;
        } else {
            fixture
                .view
                .cdp(
                    "Emulation.setDeviceMetricsOverride",
                    json!({"width": 777, "height": 555, "deviceScaleFactor": 1, "mobile": false}),
                    Some(&session),
                )
                .await?;
        }
        fixture.native.clear_calls();
        drop(fixture.view.prepare_viewport(page.clone()).await?);
        let methods = emulation_methods(&fixture.native.methods());
        for method in [
            "Emulation.setDeviceMetricsOverride",
            "Emulation.setVisibleSize",
        ] {
            assert!(
                methods.iter().any(|sent| sent == method),
                "{raw}: {methods:?}"
            );
        }
    }

    // A raw call that leaves the mode's overrides alone puts nothing back but the view pin.
    fixture
        .view
        .cdp_json_for_page(
            page.clone(),
            "Emulation.setFocusEmulationEnabled",
            r#"{"enabled": true}"#,
        )
        .await?;
    fixture.native.clear_calls();
    drop(fixture.view.prepare_viewport(page).await?);
    assert_eq!(
        emulation_methods(&fixture.native.methods()),
        vec!["Emulation.setVisibleSize"]
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_a_default_change_failing_midway_leaves_the_mode_to_be_restored() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    fixture
        .set(&page, ViewportMode::Mobile, Some(150.0))
        .await?;
    // The metrics clear lands, then the identity reset fails.
    fixture.native.lock().fail = Some("Emulation.setUserAgentOverride");
    assert!(
        fixture
            .set(&page, ViewportMode::Default, None)
            .await
            .is_err()
    );
    fixture.native.lock().fail = None;
    fixture.native.clear_calls();

    // The tab is still mobile at 150 %, so its next operation puts the whole mode back.
    drop(fixture.view.prepare_viewport(page.clone()).await?);
    let methods = emulation_methods(&fixture.native.methods());
    for method in [
        "Emulation.setDeviceMetricsOverride",
        "Emulation.setVisibleSize",
    ] {
        assert!(methods.iter().any(|sent| sent == method), "{methods:?}");
    }
    assert!(fixture.viewports.pointer_scale(&page) == 1.5);
    Ok(())
}

#[tokio::test]
async fn tests_that_a_failed_pin_is_retried_by_the_next_operation() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    fixture.native.lock().fail = Some("Emulation.setVisibleSize");
    assert!(
        fixture
            .set(&page, ViewportMode::Mobile, Some(150.0))
            .await
            .is_err()
    );
    fixture.native.lock().fail = None;
    fixture.native.clear_calls();
    drop(fixture.view.prepare_viewport(page.clone()).await?);
    assert_eq!(
        fixture.native.only("Emulation.setVisibleSize")?.params,
        json!({"width": 600, "height": 1359})
    );

    // A re-application whose pin fails is retried in turn.
    fixture
        .view
        .cdp_json_for_page(page.clone(), "Emulation.clearDeviceMetricsOverride", "{}")
        .await?;
    fixture.native.lock().fail = Some("Emulation.setVisibleSize");
    assert!(fixture.view.prepare_viewport(page.clone()).await.is_err());
    fixture.native.lock().fail = None;
    fixture.native.clear_calls();
    drop(fixture.view.prepare_viewport(page).await?);
    assert_eq!(
        fixture.native.only("Emulation.setVisibleSize")?.params,
        json!({"width": 600, "height": 1359})
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_a_change_cut_short_while_pinning_is_finished_by_the_next_operation()
-> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    fixture.native.lock().stall = Some("Emulation.setVisibleSize");
    let change = fixture.set(&page, ViewportMode::Mobile, Some(150.0));
    // A run deadline drops the call while the pin is in flight.
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(300), change)
            .await
            .is_err()
    );
    fixture.native.lock().stall = None;
    fixture.native.clear_calls();

    drop(fixture.view.prepare_viewport(page).await?);
    assert_eq!(
        fixture.native.only("Emulation.setVisibleSize")?.params,
        json!({"width": 600, "height": 1359})
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_a_raw_override_during_a_change_is_not_lost() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    let session = fixture.view.page_session(page.clone()).await?.session_id;
    // The page reports the mode only on its fourth check, so the change is still waiting for
    // it when the caller's raw override lands.
    fixture.native.lock().lagging_measures = 3;
    let view = fixture.view.clone();
    let target = page.clone();
    let request = ViewportRequest::new(ViewportMode::Mobile, Some(150.0))?;
    let change = tokio::spawn(async move {
        view.set_viewport(target, request, &CancellationToken::new())
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    });
    while !fixture
        .native
        .methods()
        .iter()
        .any(|method| method == "Emulation.setDeviceMetricsOverride")
    {
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    // Same screen and device scale, so the change still sees its mode; only the zoom differs.
    fixture
        .view
        .cdp(
            "Emulation.setDeviceMetricsOverride",
            json!({"width": 400, "height": 906, "screenWidth": 400, "screenHeight": 906,
                "deviceScaleFactor": 2, "mobile": true, "scale": 1}),
            Some(&session),
        )
        .await?;
    change.await??;
    fixture.native.clear_calls();

    drop(fixture.view.prepare_viewport(page).await?);
    assert_eq!(
        fixture
            .native
            .only("Emulation.setDeviceMetricsOverride")?
            .params["scale"],
        json!(1.5)
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_default_clears_what_a_failed_change_left_on_the_page() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    // The change sends its overrides, then fails because the page never reports the mode.
    fixture.native.lock().ignore_metrics = true;
    assert!(
        fixture
            .set(&page, ViewportMode::Mobile, Some(150.0))
            .await
            .is_err()
    );
    fixture.native.lock().ignore_metrics = false;
    fixture.native.clear_calls();

    fixture.set(&page, ViewportMode::Default, None).await?;
    let methods = emulation_methods(&fixture.native.methods());
    for method in [
        "Emulation.clearDeviceMetricsOverride",
        "Emulation.setTouchEmulationEnabled",
        "Emulation.setUserAgentOverride",
    ] {
        assert!(methods.iter().any(|sent| sent == method), "{methods:?}");
    }
    Ok(())
}

#[tokio::test]
async fn tests_that_views_without_viewports_refuse_and_prepare_nothing() -> TestResult {
    let native = Native::new(&[(1, 10)]);
    let session = BrowserSession::new(native.clone(), BrowserSessionHooks::default());
    let view = BrowserView::unbound(session, None);
    let page = PageId(1);
    let refused = view
        .set_viewport(
            page.clone(),
            ViewportRequest::new(ViewportMode::Mobile, None)?,
            &CancellationToken::new(),
        )
        .await
        .err()
        .ok_or("a view without viewports must refuse")?;
    assert!(refused.to_string().contains("hosted"), "{refused}");
    drop(view.prepare_viewport(page).await?);
    assert!(native.methods().is_empty());
    assert!(!view.offers_viewport());
    Ok(())
}

impl Fixture {
    fn emit(&self, method: &str, params: Value, session: &str) {
        let _ = self.native.events.send(CdpEvent {
            method: method.to_string(),
            params,
            session_id: Some(SessionId::from(session)),
        });
    }
}

/// What `future` returns, if it finishes within a moment in which an unblocked call would.
async fn finishes<F: std::future::Future>(future: F) -> Option<F::Output> {
    tokio::time::timeout(std::time::Duration::from_millis(300), future)
        .await
        .ok()
}

/// Whether `future` is still waiting after a moment in which an unblocked call would finish.
async fn still_waiting<F: std::future::Future>(future: &mut std::pin::Pin<Box<F>>) -> bool {
    tokio::time::timeout(std::time::Duration::from_millis(300), future.as_mut())
        .await
        .is_err()
}

#[tokio::test]
async fn tests_that_prepare_leaves_default_tabs_alone() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    fixture.native.clear_calls();

    drop(fixture.view.prepare_viewport(page).await?);
    assert!(
        fixture.native.methods().is_empty(),
        "{:?}",
        fixture.native.methods()
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_operations_only_put_the_view_back_while_the_mode_holds() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    fixture
        .set(&page, ViewportMode::Mobile, Some(150.0))
        .await?;
    // A person resizes the window, then makes it fullscreen.
    fixture.native.resize(10, 700, 500);
    fixture.native.set_window_state(10, "fullscreen");
    let left = fixture.native.window(10)?;
    fixture.native.clear_calls();

    // Chromium sizes the view to its window again on every resize; each operation pins it back
    // to the preview and sends nothing else.
    for _ in 0..3 {
        drop(fixture.view.prepare_viewport(page.clone()).await?);
    }
    let methods = fixture.native.methods();
    assert_eq!(methods, vec!["Emulation.setVisibleSize"; 3], "{methods:?}");
    for pin in fixture.native.calls("Emulation.setVisibleSize") {
        assert_eq!(pin.params, json!({"width": 600, "height": 1359}));
    }
    assert_eq!(fixture.native.window(10)?, left);
    Ok(())
}

#[tokio::test]
async fn tests_that_prepare_reapplies_emulation_on_a_new_session() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    fixture.set(&page, ViewportMode::Mobile, Some(50.0)).await?;
    fixture
        .session
        .pages
        .detach_session(&SessionId::from("session-1"))
        .await;
    fixture.native.clear_calls();

    drop(fixture.view.prepare_viewport(page).await?);
    let metrics = fixture.native.only("Emulation.setDeviceMetricsOverride")?;
    assert_eq!(metrics.session.as_deref(), Some("session-2"));
    assert_eq!(metrics.params["scale"], 0.5);
    assert!(window_methods(&fixture.native.methods()).is_empty());
    Ok(())
}

#[tokio::test]
async fn tests_that_a_failed_restore_fails_the_operation() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    fixture.set(&page, ViewportMode::Mobile, None).await?;
    fixture
        .session
        .pages
        .detach_session(&SessionId::from("session-1"))
        .await;
    fixture.native.lock().ignore_metrics = true;

    let error = fixture
        .view
        .prepare_viewport(page)
        .await
        .err()
        .ok_or("an operation must not run on a page that lost its mobile viewport")?;
    assert!(error.to_string().contains("mobile"), "{error}");
    Ok(())
}

#[tokio::test]
async fn tests_that_tabs_sharing_a_window_do_not_wait_for_each_other() -> TestResult {
    let fixture = Fixture::new(&[(1, 10), (2, 10), (3, 10)]);
    let busy = fixture.page(1).await?;
    let fixed = fixture.page(2).await?;
    let natural = fixture.page(3).await?;
    fixture.set(&busy, ViewportMode::Mobile, None).await?;
    fixture.set(&fixed, ViewportMode::Desktop, None).await?;

    // A fixed tab's operation holds only its own tab: the others run, and succeed, meanwhile.
    let lease = fixture.view.prepare_viewport(busy).await?;
    for page in [&fixed, &natural] {
        let other = finishes(fixture.view.prepare_viewport(page.clone()))
            .await
            .ok_or("an operation on another tab waited")??;
        drop(other);
    }
    for (page, mode) in [
        (fixed, ViewportMode::Mobile),
        (natural, ViewportMode::Desktop),
    ] {
        let request = ViewportRequest::new(mode, None)?;
        finishes(
            fixture
                .view
                .set_viewport(page, request, &CancellationToken::new()),
        )
        .await
        .ok_or("a mode change on another tab waited")??;
    }
    drop(lease);
    Ok(())
}

#[tokio::test]
async fn tests_that_a_fixed_tab_serves_one_operation_at_a_time() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    fixture.set(&page, ViewportMode::Mobile, None).await?;

    let lease = fixture.view.prepare_viewport(page.clone()).await?;
    let view = fixture.view.clone();
    let target = page.clone();
    let mut operation = Box::pin(async move { view.prepare_viewport(target).await });
    assert!(still_waiting(&mut operation).await);
    // A capture in flight would restore the emulation it saw when it began.
    let view = fixture.view.clone();
    let target = page.clone();
    let mut capture = Box::pin(async move { view.hold_viewport(target).await });
    assert!(still_waiting(&mut capture).await);
    drop(lease);
    drop(operation.await?);
    drop(capture.await);
    Ok(())
}

#[tokio::test]
async fn tests_that_prepare_leaves_a_dialog_blocked_page_to_dialog_actions() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    fixture.set(&page, ViewportMode::Mobile, None).await?;
    fixture.emit(
        "Page.javascriptDialogOpening",
        json!({"type": "confirm", "message": "Leave?", "url": "https://site.test/1"}),
        "session-1",
    );
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while fixture.view.signals().pending_dialog(&page).is_none() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await?;
    fixture
        .session
        .pages
        .detach_session(&SessionId::from("session-1"))
        .await;
    fixture.native.clear_calls();

    drop(fixture.view.prepare_viewport(page).await?);
    let methods = fixture.native.methods();
    assert!(emulation_methods(&methods).is_empty(), "{methods:?}");
    assert!(
        !methods.iter().any(|method| method == "Runtime.evaluate"),
        "{methods:?}"
    );
    Ok(())
}

fn pressed_at(native: &Native) -> Result<(Value, Value), Box<dyn Error>> {
    let pressed = native
        .calls("Input.dispatchMouseEvent")
        .into_iter()
        .find(|call| call.params["type"] == "mousePressed")
        .ok_or("no mousePressed")?;
    Ok((pressed.params["x"].clone(), pressed.params["y"].clone()))
}

#[tokio::test]
async fn tests_that_main_frame_pointer_input_follows_the_mobile_zoom() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    fixture
        .set(&page, ViewportMode::Mobile, Some(150.0))
        .await?;
    fixture.native.clear_calls();

    let input = fixture.view.input(page).await?;
    input
        .click_at(100.0, 200.0, crate::input::ClickOptions::default())
        .await?;
    assert_eq!(pressed_at(&fixture.native)?, (json!(150.0), json!(300.0)));
    Ok(())
}

#[tokio::test]
async fn tests_that_desktop_and_default_pointer_input_is_not_scaled() -> TestResult {
    let fixture = Fixture::new(&[(1, 10), (2, 10)]);
    let desktop = fixture.page(1).await?;
    let natural = fixture.page(2).await?;
    fixture.set(&desktop, ViewportMode::Desktop, None).await?;

    for page in [desktop, natural] {
        fixture.native.clear_calls();
        let input = fixture.view.input(page).await?;
        input
            .click_at(100.0, 200.0, crate::input::ClickOptions::default())
            .await?;
        assert_eq!(pressed_at(&fixture.native)?, (json!(100.0), json!(200.0)));
    }
    Ok(())
}

#[tokio::test]
async fn tests_that_page_scripts_cannot_fake_or_fail_the_check() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    fixture.native.lock().tampered = true;
    let page = fixture.page(1).await?;
    let report = fixture.set(&page, ViewportMode::Mobile, None).await?;

    assert_eq!(
        (
            report.viewport.screen_width,
            report.viewport.device_scale_factor
        ),
        (400.0, 2.0)
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_a_failed_switch_to_default_keeps_the_mode() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    fixture
        .set(&page, ViewportMode::Mobile, Some(200.0))
        .await?;
    fixture.native.lock().fail_clear = true;
    assert!(
        fixture
            .set(&page, ViewportMode::Default, None)
            .await
            .is_err()
    );

    // Still mobile at 200 %: pointer input keeps its scale, and a retry clears for real.
    fixture.native.clear_calls();
    let input = fixture.view.input(page.clone()).await?;
    input
        .click_at(10.0, 10.0, crate::input::ClickOptions::default())
        .await?;
    assert_eq!(pressed_at(&fixture.native)?, (json!(20.0), json!(20.0)));
    fixture.native.lock().fail_clear = false;
    fixture.native.clear_calls();
    fixture.set(&page, ViewportMode::Default, None).await?;
    assert_eq!(
        fixture
            .native
            .calls("Emulation.clearDeviceMetricsOverride")
            .len(),
        1
    );
    Ok(())
}

#[test]
fn tests_that_sizes_are_css_pixels_for_fixed_modes_only() -> TestResult {
    let size = |width, height| Size { width, height };
    for mode in [ViewportMode::Mobile, ViewportMode::Desktop] {
        for ok in [size(100, 100), size(390, 844), size(4096, 4096)] {
            assert_eq!(
                ViewportRequest::new(mode, None)?.with_size(ok)?.size(),
                Some(ok)
            );
        }
        for bad in [
            size(99, 500),
            size(500, 99),
            size(4097, 500),
            size(500, 4097),
        ] {
            assert!(
                ViewportRequest::new(mode, None)?.with_size(bad).is_err(),
                "{bad:?}"
            );
        }
    }
    assert!(
        ViewportRequest::new(ViewportMode::Default, None)?
            .with_size(size(390, 844))
            .is_err()
    );
    assert_eq!(
        ViewportRequest::new(ViewportMode::Mobile, None)?.size(),
        None
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_a_custom_size_replaces_the_device_size() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    let phone = Size {
        width: 390,
        height: 844,
    };
    let request = ViewportRequest::new(ViewportMode::Mobile, Some(150.0))?.with_size(phone)?;
    let report = fixture
        .view
        .set_viewport(page.clone(), request, &CancellationToken::new())
        .await?;

    assert_eq!(report.size, Some(phone));
    assert_eq!(
        (
            report.viewport.width,
            report.viewport.height,
            report.viewport.device_scale_factor
        ),
        (390.0, 844.0, 2.0)
    );
    let metrics = fixture
        .native
        .only("Emulation.setDeviceMetricsOverride")?
        .params;
    assert_eq!(
        (
            &metrics["width"],
            &metrics["screenHeight"],
            &metrics["scale"]
        ),
        (&json!(390), &json!(844), &json!(1.5))
    );
    assert_eq!(
        fixture.native.only("Emulation.setVisibleSize")?.params,
        json!({"width": 585, "height": 1266})
    );
    fixture.native.clear_calls();

    let desktop = ViewportRequest::new(ViewportMode::Desktop, None)?.with_size(Size {
        width: 1440,
        height: 900,
    })?;
    let report = fixture
        .view
        .set_viewport(page, desktop, &CancellationToken::new())
        .await?;
    assert_eq!(
        (
            report.viewport.width,
            report.viewport.height,
            report.viewport.device_scale_factor
        ),
        (1440.0, 900.0, 1.0)
    );
    assert_eq!(
        fixture.native.only("Emulation.setVisibleSize")?.params,
        json!({"width": 1440, "height": 900})
    );
    assert!(window_methods(&fixture.native.methods()).is_empty());
    Ok(())
}

#[tokio::test]
async fn tests_that_a_call_without_size_returns_to_the_mode_size() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    let phone = ViewportRequest::new(ViewportMode::Mobile, None)?.with_size(Size {
        width: 390,
        height: 844,
    })?;
    fixture
        .view
        .set_viewport(page.clone(), phone, &CancellationToken::new())
        .await?;
    let report = fixture.set(&page, ViewportMode::Mobile, None).await?;

    assert_eq!(
        report.size,
        Some(Size {
            width: 400,
            height: 906
        })
    );
    assert_eq!(
        (report.viewport.width, report.viewport.height),
        (400.0, 906.0)
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_a_held_page_makes_viewport_changes_wait() -> TestResult {
    let fixture = Fixture::new(&[(1, 10), (2, 10)]);
    let fixed = fixture.page(1).await?;
    let neighbour = fixture.page(2).await?;
    fixture.set(&fixed, ViewportMode::Mobile, None).await?;
    fixture.native.clear_calls();

    // A capture in flight holds the page and changes nothing itself; a default tab beside it
    // is not held up.
    let held = fixture.view.hold_viewport(fixed.clone()).await;
    assert!(fixture.native.methods().is_empty());
    let view = fixture.view.clone();
    let mut beside = Box::pin(async move { view.prepare_viewport(neighbour).await });
    assert!(!still_waiting(&mut beside).await);
    let view = fixture.view.clone();
    let page = fixed.clone();
    let mut change = Box::pin(async move {
        view.set_viewport(
            page,
            ViewportRequest::new(ViewportMode::Desktop, None)?,
            &CancellationToken::new(),
        )
        .await
        .map_err(|error| error.to_string())
    });
    assert!(still_waiting(&mut change).await);
    let view = fixture.view.clone();
    let page = fixed.clone();
    let mut operation = Box::pin(async move { view.prepare_viewport(page).await });
    assert!(still_waiting(&mut operation).await);
    drop(held);
    change.await?;
    drop(operation.await?);
    Ok(())
}

#[tokio::test]
async fn tests_that_an_operation_queued_behind_a_mode_change_holds_its_tab_alone() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    let capture = fixture.view.hold_viewport(page.clone()).await;

    // Queued in this order: the tab becomes mobile, then an operation on it, which still saw it
    // default when it started waiting.
    let view = fixture.view.clone();
    let target = page.clone();
    let mut to_mobile = Box::pin(async move {
        view.set_viewport(
            target,
            ViewportRequest::new(ViewportMode::Mobile, None)?,
            &CancellationToken::new(),
        )
        .await
        .map(drop)
        .map_err(|error| error.to_string())
    });
    assert!(still_waiting(&mut to_mobile).await);
    let view = fixture.view.clone();
    let target = page.clone();
    let mut operation = Box::pin(async move { view.prepare_viewport(target).await });
    assert!(still_waiting(&mut operation).await);
    drop(capture);
    to_mobile.await?;
    let lease = operation.await?;

    // It runs as a fixed tab's operation: a capture waits until it is done.
    let view = fixture.view.clone();
    let target = page.clone();
    let mut next = Box::pin(async move { view.hold_viewport(target).await });
    assert!(still_waiting(&mut next).await);
    drop(lease);
    drop(next.await);
    Ok(())
}

#[tokio::test]
async fn tests_that_a_capture_on_a_default_tab_holds_off_its_first_mode_change() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;

    let capture = fixture.view.hold_viewport(page.clone()).await;
    let view = fixture.view.clone();
    let target = page.clone();
    let mut change = Box::pin(async move {
        view.set_viewport(
            target,
            ViewportRequest::new(ViewportMode::Mobile, None)?,
            &CancellationToken::new(),
        )
        .await
        .map_err(|error| error.to_string())
    });
    assert!(still_waiting(&mut change).await);
    assert!(emulation_methods(&fixture.native.methods()).is_empty());
    drop(capture);
    change.await?;
    Ok(())
}

#[tokio::test]
async fn tests_that_a_capture_after_a_switch_to_default_holds_off_the_next_change() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    fixture.set(&page, ViewportMode::Mobile, None).await?;
    fixture.set(&page, ViewportMode::Default, None).await?;
    fixture.native.clear_calls();

    let capture = fixture.view.hold_viewport(page.clone()).await;
    let view = fixture.view.clone();
    let target = page.clone();
    let mut change = Box::pin(async move {
        view.set_viewport(
            target,
            ViewportRequest::new(ViewportMode::Desktop, None)?,
            &CancellationToken::new(),
        )
        .await
        .map_err(|error| error.to_string())
    });
    assert!(still_waiting(&mut change).await);
    assert!(emulation_methods(&fixture.native.methods()).is_empty());
    drop(capture);
    change.await?;
    Ok(())
}

#[tokio::test]
async fn tests_that_a_change_cancelled_while_it_waits_for_its_tab_changes_nothing() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    // A capture in flight holds the tab.
    let held = fixture.view.hold_viewport(page.clone()).await;
    fixture.native.clear_calls();

    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let view = fixture.view.clone();
    let target = page.clone();
    let mut change = Box::pin(async move {
        view.set_viewport(
            target,
            ViewportRequest::new(ViewportMode::Desktop, None)?,
            &token,
        )
        .await
        .map_err(|error| error.to_string())
    });
    assert!(still_waiting(&mut change).await);
    cancel.cancel();
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), change).await;
    assert!(
        matches!(&outcome, Ok(Err(error)) if error.contains("cancelled")),
        "{outcome:?}"
    );
    drop(held);

    let methods = fixture.native.methods();
    assert!(emulation_methods(&methods).is_empty(), "{methods:?}");
    assert!(window_methods(&methods).is_empty(), "{methods:?}");
    assert!(fixture.viewports.pointer_scale(&page) == 1.0);
    Ok(())
}

#[tokio::test]
async fn tests_that_mobile_identity_matches_google_chrome_on_android() -> TestResult {
    // Real Chrome 131 sends "Google Chrome";v="131", "Chromium";v="131", "Not_A Brand";v="24".
    for (version, brands) in [
        (
            "131.0.6778.86",
            [
                ("Google Chrome", "131"),
                ("Chromium", "131"),
                ("Not_A Brand", "24"),
            ],
        ),
        (
            "155.0.8315.26",
            [
                ("Google Chrome", "155"),
                ("Chromium", "155"),
                ("Not(A:Brand", "24"),
            ],
        ),
    ] {
        let fixture = Fixture::new(&[(1, 10)]);
        fixture.native.lock().chrome_version = version.to_string();
        let page = fixture.page(1).await?;
        fixture.set(&page, ViewportMode::Mobile, None).await?;

        let identity = fixture
            .native
            .only("Emulation.setUserAgentOverride")?
            .params;
        let major = version.split('.').next().unwrap_or_default();
        assert_eq!(
            identity["userAgent"],
            format!(
                "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{major}.0.0.0 Mobile Safari/537.36"
            )
        );
        // navigator.platform as Chrome on an Android phone reports it (Chromium's reduced value,
        // navigator_base.cc), not the host's.
        assert_eq!(identity["platform"], "Linux armv81");
        let metadata = &identity["userAgentMetadata"];
        let majors: Vec<Value> = brands
            .iter()
            .map(|(brand, major)| json!({"brand": brand, "version": major}))
            .collect();
        assert_eq!(metadata["brands"], json!(majors), "{version}");
        let fulls: Vec<Value> = brands
            .iter()
            .map(|(brand, major)| {
                let full = if brand.starts_with("Not") {
                    format!("{major}.0.0.0")
                } else {
                    version.to_string()
                };
                json!({"brand": brand, "version": full})
            })
            .collect();
        assert_eq!(metadata["fullVersionList"], json!(fulls), "{version}");
        assert_eq!(metadata["fullVersion"], version);
        assert_eq!(
            (
                &metadata["platform"],
                &metadata["mobile"],
                &metadata["formFactors"]
            ),
            (&json!("Android"), &json!(true), &json!(["Mobile"]))
        );
    }
    Ok(())
}

#[tokio::test]
async fn tests_that_a_dialog_blocked_page_never_waits_for_its_tab() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    fixture.set(&page, ViewportMode::Mobile, None).await?;
    fixture.emit(
        "Page.javascriptDialogOpening",
        json!({"type": "confirm", "message": "Leave?", "url": "https://site.test/1"}),
        "session-1",
    );
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while fixture.view.signals().pending_dialog(&page).is_none() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await?;

    // Something else holds the tab, such as a capture that the dialog stalls.
    let held = fixture.view.hold_viewport(page.clone()).await;
    let quick = std::time::Duration::from_millis(300);
    let lease = tokio::time::timeout(quick, fixture.view.prepare_viewport(page.clone())).await??;
    drop(lease);
    let refused = tokio::time::timeout(
        quick,
        fixture.view.set_viewport(
            page,
            ViewportRequest::new(ViewportMode::Desktop, None)?,
            &CancellationToken::new(),
        ),
    )
    .await?
    .err()
    .ok_or("a mode change on a dialog-blocked page must be refused")?;
    assert!(refused.to_string().contains("dialog"), "{refused}");
    drop(held);
    Ok(())
}

#[tokio::test]
async fn tests_that_closed_tabs_are_forgotten() -> TestResult {
    let fixture = Fixture::new(&[(1, 10), (2, 10), (3, 10)]);
    for tab in [1, 2] {
        let page = fixture.page(tab).await?;
        fixture.set(&page, ViewportMode::Mobile, None).await?;
    }
    assert_eq!(fixture.viewports.tracked(), (2, 2));
    fixture.native.close_tab(1);
    fixture.native.close_tab(2);
    fixture.view.list_pages().await?;

    // Whatever is added next first drops what closed, so a long-lived server stays bounded.
    let page = fixture.page(3).await?;
    fixture.set(&page, ViewportMode::Desktop, None).await?;
    assert_eq!(fixture.viewports.tracked(), (1, 1));
    Ok(())
}

#[tokio::test]
async fn tests_that_fixed_modes_pin_the_tab_view_to_the_preview() -> TestResult {
    let fixture = Fixture::new(&[(1, 10)]);
    let page = fixture.page(1).await?;
    fixture
        .set(&page, ViewportMode::Mobile, Some(150.0))
        .await?;
    // Whatever size an earlier caller or capture left the tab's view at, it now matches the
    // preview, so zoomed input aimed anywhere on the preview lands inside it.
    let methods = emulation_methods(&fixture.native.methods());
    let metrics = methods
        .iter()
        .position(|method| method == "Emulation.setDeviceMetricsOverride");
    let pinned = methods
        .iter()
        .position(|method| method == "Emulation.setVisibleSize");
    assert!(metrics < pinned && pinned.is_some(), "{methods:?}");
    assert_eq!(
        fixture.native.only("Emulation.setVisibleSize")?.params,
        json!({"width": 600, "height": 1359})
    );

    fixture.native.clear_calls();
    fixture.set(&page, ViewportMode::Desktop, None).await?;
    assert_eq!(
        fixture.native.only("Emulation.setVisibleSize")?.params,
        json!({"width": 1920, "height": 1080})
    );
    fixture.native.clear_calls();
    fixture.set(&page, ViewportMode::Default, None).await?;
    assert!(fixture.native.calls("Emulation.setVisibleSize").is_empty());
    Ok(())
}
