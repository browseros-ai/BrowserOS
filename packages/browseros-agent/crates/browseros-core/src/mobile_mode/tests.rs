//! Mobile mode against a recording native: command order, rollback, the page record, re-apply
//! on attach and input scaling. Chromium's own behavior is proven by the live browser checks.
use super::*;
use crate::{
    BrowserSession, BrowserSessionHooks, BrowserView,
    input::{
        Point,
        mouse::{MouseButton, dispatch_click, dispatch_drag, dispatch_hover, dispatch_scroll},
    },
    pages::{NewPageOptions, PageManagerHooks},
};
use browseros_cdp::{CdpError, CdpEvent};
use futures_util::future::BoxFuture;
use std::{
    collections::HashMap,
    error::Error,
    sync::{
        Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, broadcast};

type TestResult = Result<(), Box<dyn Error>>;

const UA: &str = "Emulation.setUserAgentOverride";
const TOUCH: &str = "Emulation.setTouchEmulationEnabled";
const METRICS: &str = "Emulation.setDeviceMetricsOverride";
const VISIBLE: &str = "Emulation.setVisibleSize";
const CLEAR: &str = "Emulation.clearDeviceMetricsOverride";
const PUBLISHED: &str = "test.sessionPublished";

#[derive(Clone, Debug)]
struct Call {
    method: String,
    params: Value,
    session: Option<SessionId>,
}

#[derive(Default)]
struct Fixture {
    calls: Vec<Call>,
    tabs: Vec<Value>,
    product: String,
    /// Upcoming calls of a method that fail, counted down per call.
    failures: HashMap<String, usize>,
    /// The connection drops right after this method is answered.
    lose_after: Option<String>,
    attachments: usize,
    hold: Option<(String, Arc<Notify>)>,
    /// Window 1's outer bounds; contents are these minus `overhead`, never below `minimum`.
    window: Value,
    overhead: (i64, i64),
    minimum: (i64, i64),
}

struct Native {
    state: Mutex<Fixture>,
    epoch: AtomicU64,
    connected: AtomicBool,
    events: broadcast::Sender<CdpEvent>,
    held: Notify,
}

fn tab(id: i64, url: &str) -> Value {
    json!({"tabId":id,"targetId":format!("target-{id}"),"url":url,"title":"fixture",
        "isActive":true,"isLoading":false,"loadProgress":1.0,"isPinned":false,
        "isHidden":false,"windowId":1})
}

impl Native {
    fn new(tabs: Vec<Value>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(Fixture {
                tabs,
                product: "Chrome/155.0.7727.12".into(),
                window: json!({"left":80,"top":44,"width":1200,"height":1000,"windowState":"normal"}),
                overhead: (240, 81),
                minimum: (626, 200),
                ..Fixture::default()
            }),
            epoch: AtomicU64::new(1),
            connected: AtomicBool::new(true),
            events: broadcast::channel(16).0,
            held: Notify::new(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Fixture> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn fail(&self, method: &str, times: usize) {
        self.lock().failures.insert(method.into(), times);
    }

    fn calls(&self) -> Vec<Call> {
        self.lock().calls.clone()
    }

    fn mark(&self) -> usize {
        self.lock().calls.len()
    }

    fn since(&self, mark: usize) -> Vec<Call> {
        self.calls().into_iter().skip(mark).collect()
    }

    fn emulation(&self) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|call| call.method.starts_with("Emulation."))
            .collect()
    }

    fn reply(
        &self,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CdpError> {
        let mut state = self.lock();
        if !self.connected.load(Ordering::SeqCst) {
            return Err(CdpError::ConnectionLost);
        }
        state.calls.push(Call {
            method: method.into(),
            params: params.clone(),
            session: session.cloned(),
        });
        if state.lose_after.as_deref() == Some(method) {
            self.connected.store(false, Ordering::SeqCst);
        }
        if let Some(remaining) = state.failures.get_mut(method)
            && *remaining > 0
        {
            *remaining -= 1;
            return Err(CdpError::Protocol {
                code: -32000,
                message: format!("injected failure: {method}"),
            });
        }
        Ok(match method {
            "Browser.getVersion" => json!({"protocolVersion":"1.3","product":state.product,
                "revision":"@fixture","userAgent":"Mozilla/5.0 fixture","jsVersion":"15.5"}),
            "Browser.getTabs" => json!({"tabs":state.tabs}),
            "Browser.getTabInfo" => {
                let selected = state
                    .tabs
                    .iter()
                    .find(|tab| {
                        params.get("tabId").is_some_and(|id| *id == tab["tabId"])
                            || params
                                .get("targetId")
                                .is_some_and(|id| *id == tab["targetId"])
                    })
                    .cloned()
                    .ok_or(CdpError::SessionGone)?;
                json!({"tab":selected})
            }
            "Browser.createTab" => {
                let id = state
                    .tabs
                    .iter()
                    .filter_map(|tab| tab["tabId"].as_i64())
                    .max()
                    .unwrap_or_default()
                    + 1;
                let created = tab(id, params["url"].as_str().unwrap_or("about:blank"));
                state.tabs.push(created.clone());
                json!({"tab":created})
            }
            "Browser.closeTab" => {
                state.tabs.retain(|tab| tab["tabId"] != params["tabId"]);
                json!({})
            }
            "Target.attachToTarget" => {
                state.attachments += 1;
                json!({"sessionId":format!(
                    "session-{}-{}",
                    params["targetId"].as_str().unwrap_or_default(),
                    state.attachments
                )})
            }
            "Runtime.evaluate" => {
                return Err(CdpError::Protocol {
                    code: -32000,
                    message: "no page runtime in this fixture".into(),
                });
            }
            "Browser.getWindowBounds" | "Browser.setContentsSize" | "Browser.setWindowBounds"
                if params["windowId"] != 1 =>
            {
                return Err(CdpError::Protocol {
                    code: -32000,
                    message: "Browser window not found".into(),
                });
            }
            "Browser.getWindowBounds" => json!({"bounds":state.window}),
            "Browser.setContentsSize" => {
                if state.window["windowState"] != "normal" {
                    return Err(CdpError::Protocol {
                        code: -32000,
                        message: "Restore window to normal state before setting content size"
                            .into(),
                    });
                }
                let (width, height) = (
                    state.window["width"].as_i64().unwrap_or_default(),
                    state.window["height"].as_i64().unwrap_or_default(),
                );
                let contents = (width - state.overhead.0, height - state.overhead.1);
                if let Some(desired) = params["width"].as_i64() {
                    state.window["width"] =
                        json!((width + desired - contents.0).max(state.minimum.0));
                }
                if let Some(desired) = params["height"].as_i64() {
                    state.window["height"] =
                        json!((height + desired - contents.1).max(state.minimum.1));
                }
                json!({})
            }
            "Browser.setWindowBounds" => {
                for key in ["left", "top", "width", "height", "windowState"] {
                    if let Some(value) = params["bounds"].get(key) {
                        state.window[key] = value.clone();
                    }
                }
                json!({})
            }
            _ => json!({}),
        })
    }

    async fn dispatch(
        &self,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CdpError> {
        let hold = {
            let mut state = self.lock();
            if state.hold.as_ref().is_some_and(|(held, _)| held == method) {
                state.hold.take().map(|(_, release)| release)
            } else {
                None
            }
        };
        if let Some(release) = hold {
            self.held.notify_one();
            release.notified().await;
        }
        self.reply(method, params, session)
    }
}

impl CdpConnection for Native {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(self.dispatch(method, params, session))
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            let value = self
                .dispatch(method, serde_json::from_str(params)?, session)
                .await?;
            Ok(serde_json::to_string(&value)?)
        })
    }

    fn send_at_epoch<'a>(
        &'a self,
        expected: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            let actual = self.connection_epoch();
            if expected != actual {
                return Err(CdpError::ConnectionEpochChanged { expected, actual });
            }
            self.dispatch(method, params, session).await
        })
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn connection_epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }
}

struct Harness {
    native: Arc<Native>,
    browser: Arc<BrowserSession>,
    view: BrowserView,
}

async fn harness(tabs: Vec<Value>) -> Result<(Harness, PageId), CoreError> {
    harness_with(tabs, BrowserSessionHooks::default()).await
}

async fn harness_with(
    tabs: Vec<Value>,
    hooks: BrowserSessionHooks,
) -> Result<(Harness, PageId), CoreError> {
    let native = Native::new(tabs);
    let browser = BrowserSession::new(native.clone(), hooks);
    let page = browser
        .pages
        .list()
        .await?
        .first()
        .map(|page| page.page_id.clone())
        .ok_or("fixture has no page")?;
    let view = BrowserView::unbound(browser.clone(), None);
    Ok((
        Harness {
            native,
            browser,
            view,
        },
        page,
    ))
}

/// Records the moment a session is published, so ordering can be asserted against it.
fn publication_marker(native: &Arc<Native>) -> BrowserSessionHooks {
    let native = native.clone();
    BrowserSessionHooks {
        page_manager: PageManagerHooks {
            on_session_attached: Some(Arc::new(move |_session, _page, session_id| {
                let native = native.clone();
                Box::pin(async move {
                    native.lock().calls.push(Call {
                        method: PUBLISHED.into(),
                        params: json!({}),
                        session: Some(session_id),
                    });
                    Ok(())
                })
            })),
            on_page_detached: None,
        },
    }
}

fn mobile(zoom: f64) -> Result<MobileMode, MobileModeArgError> {
    MobileMode::from_request(true, Some(zoom))
}

fn user_agent() -> &'static str {
    "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Mobile Safari/537.36"
}

fn preset(scale: f64, width: i64, height: i64) -> Vec<(String, Value)> {
    vec![
        (
            UA.into(),
            json!({"userAgent":user_agent(),"userAgentMetadata":{"platform":"Android",
                "platformVersion":"10.0.0","architecture":"","model":"K","mobile":true,
                "formFactors":["Mobile"]}}),
        ),
        (TOUCH.into(), json!({"enabled":true,"maxTouchPoints":1})),
        (
            METRICS.into(),
            json!({"width":400,"height":906,"deviceScaleFactor":2.0,"mobile":true,
                "scale":scale,"screenWidth":400,"screenHeight":906,
                "screenOrientation":{"type":"portraitPrimary","angle":0},
                "dontSetVisibleSize":true}),
        ),
        (VISIBLE.into(), json!({"width":width,"height":height})),
    ]
}

fn desktop() -> Vec<(String, Value)> {
    vec![
        (CLEAR.into(), json!({})),
        (TOUCH.into(), json!({"enabled":false})),
        (UA.into(), json!({"userAgent":""})),
    ]
}

fn sent(calls: &[Call]) -> Vec<(String, Value)> {
    calls
        .iter()
        .map(|call| (call.method.clone(), call.params.clone()))
        .collect()
}

fn on_session(calls: &[Call], session: &SessionId) -> Vec<Call> {
    calls
        .iter()
        .filter(|call| call.session.as_ref() == Some(session))
        .cloned()
        .collect()
}

async fn record_zoom(harness: &Harness, page: &PageId) -> Option<f64> {
    harness
        .browser
        .pages
        .mobile_record(page)
        .await
        .map(|record| record.zoom().percent())
}

async fn page_session(harness: &Harness, page: &PageId) -> Result<ProtocolSession, CoreError> {
    Ok(harness
        .browser
        .pages
        .get_session(page.clone())
        .await?
        .session)
}

fn failure(
    result: Result<MobileModeOutcome, CoreError>,
) -> Result<(String, MobileModeState), String> {
    match result {
        Err(CoreError::MobileMode { step, state, .. }) => Ok((step, state)),
        other => Err(format!("expected a mobile mode failure, got {other:?}")),
    }
}

#[test]
fn requests_are_validated_without_a_browser() {
    for zoom in [0.0, 24.9, 400.1, -100.0, f64::NAN, f64::INFINITY] {
        assert_eq!(
            MobileMode::from_request(true, Some(zoom)),
            Err(MobileModeArgError::ZoomOutOfRange),
            "{zoom}"
        );
    }
    assert_eq!(
        MobileMode::from_request(false, Some(100.0)),
        Err(MobileModeArgError::ZoomWithoutMobile)
    );
    assert_eq!(
        MobileMode::from_request(false, None),
        Ok(MobileMode::Desktop)
    );
    for (requested, percent) in [(None, 100.0), (Some(25.0), 25.0), (Some(400.0), 400.0)] {
        let mode = MobileMode::from_request(true, requested);
        assert_eq!(
            mode.map(|mode| mode.zoom().map(Zoom::percent)),
            Ok(Some(percent))
        );
    }
}

#[test]
fn the_ua_major_comes_only_from_the_browser_product() {
    for (product, major) in [
        ("Chrome/155.0.7727.12", Some(155)),
        ("HeadlessChrome/154.0.1.2", Some(154)),
        ("Chrome/155", Some(155)),
        ("Firefox/120.0", None),
        ("Chrome/", None),
        ("Chrome/0.1.2.3", None),
        ("Chrome/x.1", None),
        ("", None),
    ] {
        assert_eq!(
            browser_major(&json!({ "product": product })).ok(),
            major,
            "{product}"
        );
    }
    assert!(browser_major(&json!({})).is_err());
}

#[tokio::test]
async fn enable_reads_the_root_version_then_sends_the_preset_in_order() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    let outcome = harness
        .view
        .toggle_mobile_mode(page.clone(), MobileMode::from_request(true, None)?)
        .await?;
    assert_eq!(outcome.mode, mobile(100.0)?);
    assert_eq!(outcome.mode.visible_size(), Some((400, 906)));
    assert!(outcome.changed);
    assert!(!outcome.reload_required);

    let session = harness.browser.pages.get_session(page.clone()).await?;
    let calls = harness.native.calls();
    let version = calls
        .iter()
        .position(|call| call.method == "Browser.getVersion")
        .ok_or("version was never read")?;
    assert!(calls[version].session.is_none(), "version is a root read");
    let emulation = harness.native.emulation();
    assert_eq!(sent(&emulation), preset(1.0, 400, 906));
    assert!(
        emulation
            .iter()
            .all(|call| call.session.as_ref() == Some(&session.session_id))
    );
    let first = calls
        .iter()
        .position(|call| call.method.starts_with("Emulation."))
        .ok_or("no emulation")?;
    assert!(version < first);
    assert_eq!(
        calls.last().map(|call| call.method.as_str()),
        Some(VISIBLE),
        "nothing follows the preset: no reload or navigation"
    );
    for forbidden in [
        "Network.setUserAgentOverride",
        "Emulation.setEmitTouchEventsForMouse",
        "Emulation.setFocusEmulationEnabled",
        "Page.reload",
        "Page.navigate",
    ] {
        assert!(
            calls.iter().all(|call| call.method != forbidden),
            "{forbidden}"
        );
    }
    assert_eq!(record_zoom(&harness, &page).await, Some(100.0));
    Ok(())
}

#[tokio::test]
async fn zoom_sets_the_scale_and_the_visible_size() -> TestResult {
    for (zoom, scale, width, height) in [
        (50.0, 0.5, 200, 453),
        (75.0, 0.75, 300, 680),
        (150.0, 1.5, 600, 1359),
        (200.0, 2.0, 800, 1812),
    ] {
        let (harness, page) = harness(vec![tab(1, "https://example.test/")]).await?;
        let outcome = harness
            .view
            .toggle_mobile_mode(page.clone(), mobile(zoom)?)
            .await?;
        assert_eq!(outcome.mode.visible_size(), Some((width, height)));
        assert!(outcome.reload_required);
        assert_eq!(
            sent(&harness.native.emulation()),
            preset(scale, width, height)
        );
    }
    Ok(())
}

#[tokio::test]
async fn an_unparsable_version_changes_nothing() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    harness.native.lock().product = "Firefox/120.0".into();
    let result = harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(150.0)?)
        .await;
    assert!(result.is_err(), "{result:?}");
    assert!(
        harness
            .native
            .calls()
            .iter()
            .any(|call| call.method == "Browser.getVersion")
    );
    assert!(harness.native.emulation().is_empty());
    assert_eq!(record_zoom(&harness, &page).await, None);
    Ok(())
}

#[tokio::test]
async fn changing_zoom_replaces_the_preset_and_repeats_resend_it() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    let first = harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(100.0)?)
        .await?;
    assert!(first.changed);
    let mark = harness.native.emulation().len();
    let zoomed = harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(150.0)?)
        .await?;
    assert!(zoomed.changed);
    assert_eq!(
        sent(&harness.native.emulation()[mark..]),
        preset(1.5, 600, 1359)
    );
    let mark = harness.native.emulation().len();
    let repeat = harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(150.0)?)
        .await?;
    assert!(!repeat.changed);
    assert_eq!(
        sent(&harness.native.emulation()[mark..]),
        preset(1.5, 600, 1359)
    );
    assert_eq!(record_zoom(&harness, &page).await, Some(150.0));
    Ok(())
}

#[tokio::test]
async fn disable_clears_only_the_owned_settings() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "https://example.test/")]).await?;
    let session = page_session(&harness, &page).await?;
    harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(150.0)?)
        .await?;
    assert_eq!(input_scale(&session), 1.5);
    let mark = harness.native.mark();
    let outcome = harness
        .view
        .toggle_mobile_mode(page.clone(), MobileMode::Desktop)
        .await?;
    assert!(outcome.changed);
    assert!(outcome.reload_required);
    assert_eq!(sent(&harness.native.since(mark)), desktop());
    assert_eq!(record_zoom(&harness, &page).await, None);
    assert_eq!(input_scale(&session), 1.0);
    assert!(
        harness
            .native
            .calls()
            .iter()
            .all(|call| call.method != "Emulation.setFocusEmulationEnabled")
    );
    Ok(())
}

#[tokio::test]
async fn disabling_a_desktop_tab_is_safe() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    let outcome = harness
        .view
        .toggle_mobile_mode(page.clone(), MobileMode::Desktop)
        .await?;
    assert!(!outcome.changed);
    assert!(!outcome.reload_required);
    assert_eq!(sent(&harness.native.emulation()), desktop());
    assert!(
        harness
            .native
            .calls()
            .iter()
            .all(|call| call.method != "Browser.getVersion")
    );
    Ok(())
}

#[tokio::test]
async fn a_failed_step_from_desktop_undoes_everything_including_itself() -> TestResult {
    for step in [UA, TOUCH, METRICS, VISIBLE] {
        let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
        let session = page_session(&harness, &page).await?;
        harness.native.fail(step, 1);
        let result = harness
            .view
            .toggle_mobile_mode(page.clone(), mobile(150.0)?)
            .await;
        assert_eq!(
            failure(result)?,
            (step.to_string(), MobileModeState::Desktop)
        );
        let emulation = harness.native.emulation();
        let failed = emulation
            .iter()
            .position(|call| call.method == step)
            .ok_or("failed step was not sent")?;
        assert_eq!(sent(&emulation[failed + 1..]), desktop(), "{step}");
        assert_eq!(record_zoom(&harness, &page).await, None);
        assert_eq!(input_scale(&session), 1.0);
    }
    Ok(())
}

#[tokio::test]
async fn a_failed_zoom_change_restores_the_previous_preset() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    let session = page_session(&harness, &page).await?;
    harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(100.0)?)
        .await?;
    harness.native.fail(VISIBLE, 1);
    let mark = harness.native.emulation().len();
    let result = harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(150.0)?)
        .await;
    assert_eq!(
        failure(result)?,
        (VISIBLE.to_string(), MobileModeState::Previous)
    );
    let mut expected = preset(1.5, 600, 1359);
    expected.extend(preset(1.0, 400, 906));
    assert_eq!(sent(&harness.native.emulation()[mark..]), expected);
    assert_eq!(record_zoom(&harness, &page).await, Some(100.0));
    assert_eq!(input_scale(&session), 1.0);
    Ok(())
}

#[tokio::test]
async fn an_unconfirmed_rollback_reports_unknown_and_keeps_the_record() -> TestResult {
    // From desktop: the step and its undo both fail.
    let (from_desktop, page) = harness(vec![tab(1, "about:blank")]).await?;
    from_desktop.native.fail(METRICS, 1);
    from_desktop.native.fail(CLEAR, 1);
    let result = from_desktop
        .view
        .toggle_mobile_mode(page.clone(), mobile(150.0)?)
        .await;
    assert_eq!(
        failure(result)?,
        (METRICS.to_string(), MobileModeState::Unknown)
    );
    assert_eq!(record_zoom(&from_desktop, &page).await, None);

    // From 100%: restoring the previous preset fails too.
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    let session = page_session(&harness, &page).await?;
    harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(100.0)?)
        .await?;
    harness.native.fail(VISIBLE, 2);
    let result = harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(150.0)?)
        .await;
    assert_eq!(
        failure(result)?,
        (VISIBLE.to_string(), MobileModeState::Unknown)
    );
    assert_eq!(record_zoom(&harness, &page).await, Some(100.0));
    assert_eq!(input_scale(&session), 1.0);
    Ok(())
}

#[tokio::test]
async fn a_lost_connection_reports_unknown() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(100.0)?)
        .await?;
    harness.native.lock().lose_after = Some(UA.into());
    let result = harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(150.0)?)
        .await;
    assert_eq!(
        failure(result)?,
        (TOUCH.to_string(), MobileModeState::Unknown)
    );
    assert_eq!(record_zoom(&harness, &page).await, Some(100.0));
    Ok(())
}

#[tokio::test]
async fn a_failed_disable_still_sends_every_clear_and_drops_the_record() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    let session = page_session(&harness, &page).await?;
    harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(150.0)?)
        .await?;
    harness.native.fail(TOUCH, 1);
    let mark = harness.native.mark();
    let result = harness
        .view
        .toggle_mobile_mode(page.clone(), MobileMode::Desktop)
        .await;
    assert_eq!(
        failure(result)?,
        (TOUCH.to_string(), MobileModeState::Unknown)
    );
    assert_eq!(sent(&harness.native.since(mark)), desktop());
    assert_eq!(record_zoom(&harness, &page).await, None);
    assert_eq!(input_scale(&session), 1.0);
    Ok(())
}

#[tokio::test]
async fn a_cancelled_caller_does_not_stop_the_sequence() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    let session = page_session(&harness, &page).await?;
    let release = Arc::new(Notify::new());
    harness.native.lock().hold = Some((TOUCH.into(), release.clone()));
    {
        let call = harness
            .view
            .toggle_mobile_mode(page.clone(), mobile(150.0)?);
        tokio::pin!(call);
        tokio::select! {
            result = &mut call => return Err(format!("finished while held: {result:?}").into()),
            () = harness.native.held.notified() => {}
        }
    }
    // The caller is gone after the first command. The owned task finishes the sequence.
    release.notify_one();
    for _ in 0..200 {
        if record_zoom(&harness, &page).await.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(record_zoom(&harness, &page).await, Some(150.0));
    assert_eq!(sent(&harness.native.emulation()), preset(1.5, 600, 1359));
    assert_eq!(input_scale(&session), 1.5);
    Ok(())
}

#[tokio::test]
async fn a_toggle_waits_for_the_page_screenshot_lock() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    let session = page_session(&harness, &page).await?;
    let lock = crate::screenshot::screenshot_mutex(&session);
    let guard = lock.lock().await;
    let mode = mobile(150.0)?;
    let task = tokio::spawn({
        let view = harness.view.clone();
        let page = page.clone();
        async move { view.toggle_mobile_mode(page, mode).await }
    });
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(harness.native.emulation().is_empty());
    drop(guard);
    task.await??;
    assert_eq!(sent(&harness.native.emulation()), preset(1.5, 600, 1359));
    Ok(())
}

#[tokio::test]
async fn reattach_reapplies_the_record_before_the_session_is_published() -> TestResult {
    let native = Native::new(vec![tab(1, "about:blank")]);
    let browser = BrowserSession::new(native.clone(), publication_marker(&native));
    let page = browser
        .pages
        .list()
        .await?
        .first()
        .map(|page| page.page_id.clone())
        .ok_or("no page")?;
    let harness = Harness {
        view: BrowserView::unbound(browser.clone(), None),
        native,
        browser,
    };
    harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(150.0)?)
        .await?;
    let old = harness.browser.pages.get_session(page.clone()).await?;
    harness.browser.pages.detach_session(&old.session_id).await;
    assert_eq!(input_scale(&old.session), 1.0);

    let mark = harness.native.mark();
    let fresh = harness.browser.pages.get_session(page.clone()).await?;
    assert_ne!(fresh.session_id, old.session_id);
    let calls = on_session(&harness.native.since(mark), &fresh.session_id);
    let methods = calls
        .iter()
        .map(|call| call.method.as_str())
        .collect::<Vec<_>>();
    let ready = methods
        .iter()
        .position(|method| *method == "Runtime.runIfWaitingForDebugger")
        .ok_or("attach setup missing")?;
    let published = methods
        .iter()
        .position(|method| *method == PUBLISHED)
        .ok_or("publication missing")?;
    let reapplied = calls
        .iter()
        .enumerate()
        .filter(|(_, call)| call.method.starts_with("Emulation."))
        .collect::<Vec<_>>();
    assert_eq!(
        sent(
            &reapplied
                .iter()
                .map(|(_, call)| (*call).clone())
                .collect::<Vec<_>>()
        ),
        preset(1.5, 600, 1359)
    );
    assert!(
        reapplied
            .iter()
            .all(|(index, _)| *index > ready && *index < published)
    );
    assert!(
        harness
            .native
            .since(mark)
            .iter()
            .all(|call| call.method != "Browser.getVersion")
    );
    assert_eq!(input_scale(&fresh.session), 1.5);
    assert_eq!(record_zoom(&harness, &page).await, Some(150.0));
    Ok(())
}

#[tokio::test]
async fn reapply_follows_focus_emulation_on_agent_tabs() -> TestResult {
    let (harness, _) = harness(vec![tab(1, "about:blank")]).await?;
    let page = harness
        .browser
        .pages
        .new_page("about:blank", NewPageOptions::default())
        .await?;
    harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(200.0)?)
        .await?;
    let old = harness.browser.pages.get_session(page.clone()).await?;
    harness.browser.pages.detach_session(&old.session_id).await;
    let mark = harness.native.mark();
    let fresh = harness.browser.pages.get_session(page.clone()).await?;
    let emulation = on_session(&harness.native.since(mark), &fresh.session_id)
        .into_iter()
        .filter(|call| call.method.starts_with("Emulation."))
        .collect::<Vec<_>>();
    let mut expected = vec![(
        "Emulation.setFocusEmulationEnabled".to_string(),
        json!({"enabled":true}),
    )];
    expected.extend(preset(2.0, 800, 1812));
    assert_eq!(sent(&emulation), expected);
    Ok(())
}

#[tokio::test]
async fn an_unrecorded_page_attaches_without_emulation() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    harness.browser.pages.get_session(page.clone()).await?;
    assert!(harness.native.emulation().is_empty());
    Ok(())
}

#[tokio::test]
async fn a_failed_reapply_clears_the_tab_drops_the_record_and_still_attaches() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(150.0)?)
        .await?;
    let old = harness.browser.pages.get_session(page.clone()).await?;
    harness.browser.pages.detach_session(&old.session_id).await;
    harness.native.fail(METRICS, 1);
    let mark = harness.native.mark();
    let fresh = harness.browser.pages.get_session(page.clone()).await?;
    let emulation = on_session(&harness.native.since(mark), &fresh.session_id)
        .into_iter()
        .filter(|call| call.method.starts_with("Emulation."))
        .collect::<Vec<_>>();
    let mut expected = preset(1.5, 600, 1359)[..3].to_vec();
    expected.extend(desktop());
    assert_eq!(sent(&emulation), expected);
    assert_eq!(record_zoom(&harness, &page).await, None);
    assert_eq!(input_scale(&fresh.session), 1.0);
    Ok(())
}

#[tokio::test]
async fn closing_a_page_drops_its_record_and_input_scale() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank"), tab(2, "about:blank")]).await?;
    let session = page_session(&harness, &page).await?;
    harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(150.0)?)
        .await?;
    harness.browser.pages.close(page.clone()).await?;
    assert_eq!(record_zoom(&harness, &page).await, None);
    assert_eq!(input_scale(&session), 1.0);
    Ok(())
}

#[tokio::test]
async fn a_pruned_page_drops_its_record_and_input_scale() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank"), tab(2, "about:blank")]).await?;
    let session = page_session(&harness, &page).await?;
    harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(150.0)?)
        .await?;
    harness.native.lock().tabs.retain(|tab| tab["tabId"] != 1);
    harness.browser.pages.list().await?;
    assert_eq!(record_zoom(&harness, &page).await, None);
    assert_eq!(input_scale(&session), 1.0);
    Ok(())
}

#[tokio::test]
async fn a_replaced_target_keeps_the_record_and_gets_it_reapplied() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    let old = harness.browser.pages.get_session(page.clone()).await?;
    harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(150.0)?)
        .await?;
    harness.native.lock().tabs[0]["targetId"] = json!("target-1-replacement");
    let listed = harness.browser.pages.list().await?;
    assert_eq!(
        listed.first().map(|page| page.page_id.clone()),
        Some(page.clone())
    );
    assert_eq!(input_scale(&old.session), 1.0);
    assert_eq!(record_zoom(&harness, &page).await, Some(150.0));

    let mark = harness.native.mark();
    let fresh = harness.browser.pages.get_session(page.clone()).await?;
    assert_eq!(fresh.target_id.as_str(), "target-1-replacement");
    let emulation = on_session(&harness.native.since(mark), &fresh.session_id)
        .into_iter()
        .filter(|call| call.method.starts_with("Emulation."))
        .collect::<Vec<_>>();
    assert_eq!(sent(&emulation), preset(1.5, 600, 1359));
    assert_eq!(input_scale(&fresh.session), 1.5);
    Ok(())
}

#[tokio::test]
async fn a_second_page_never_gets_the_preset() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank"), tab(2, "about:blank")]).await?;
    harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(150.0)?)
        .await?;
    let other = harness
        .browser
        .pages
        .list()
        .await?
        .into_iter()
        .map(|page| page.page_id)
        .find(|id| *id != page)
        .ok_or("second page missing")?;
    let second = harness.browser.pages.get_session(other.clone()).await?;
    assert!(
        on_session(&harness.native.calls(), &second.session_id)
            .iter()
            .all(|call| !call.method.starts_with("Emulation."))
    );
    assert_eq!(input_scale(&second.session), 1.0);
    assert_eq!(record_zoom(&harness, &other).await, None);
    Ok(())
}

#[tokio::test]
async fn mouse_input_is_sent_in_zoomed_view_space() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    let session = page_session(&harness, &page).await?;
    harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(150.0)?)
        .await?;
    let mark = harness.native.mark();
    dispatch_click(&session, 100.0, 200.0, MouseButton::Left, 1, 0).await?;
    dispatch_hover(&session, 10.0, 20.0).await?;
    dispatch_drag(
        &session,
        Point { x: 10.0, y: 20.0 },
        Point { x: 30.0, y: 40.0 },
    )
    .await?;
    dispatch_scroll(&session, 100.0, 200.0, 0.0, 120.0).await?;
    let params = harness
        .native
        .since(mark)
        .into_iter()
        .map(|call| call.params)
        .collect::<Vec<_>>();
    assert_eq!(
        params,
        vec![
            json!({"type":"mouseMoved","x":150.0,"y":300.0}),
            json!({"type":"mousePressed","x":150.0,"y":300.0,"button":"left","clickCount":1,"modifiers":0}),
            json!({"type":"mouseReleased","x":150.0,"y":300.0,"button":"left","clickCount":1,"modifiers":0}),
            json!({"type":"mouseMoved","x":15.0,"y":30.0}),
            json!({"type":"mouseMoved","x":15.0,"y":30.0}),
            json!({"type":"mousePressed","x":15.0,"y":30.0,"button":"left","clickCount":1}),
            json!({"type":"mouseMoved","x":45.0,"y":60.0}),
            json!({"type":"mouseReleased","x":45.0,"y":60.0,"button":"left","clickCount":1}),
            json!({"type":"mouseWheel","x":150.0,"y":300.0,"deltaX":0.0,"deltaY":180.0}),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn input_scale_never_crosses_sessions_connections_or_epochs() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    let session = page_session(&harness, &page).await?;
    harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(150.0)?)
        .await?;
    let id = session
        .session_id()
        .cloned()
        .ok_or("page session has no id")?;

    let sibling = ProtocolSession::for_session(harness.native.clone(), "session-other".into());
    let other_browser = Native::new(Vec::new());
    let same_id_elsewhere = ProtocolSession::for_session(other_browser.clone(), id);
    assert_eq!(input_scale(&session), 1.5);
    assert_eq!(input_scale(&sibling), 1.0);
    assert_eq!(input_scale(&same_id_elsewhere), 1.0);
    dispatch_hover(&same_id_elsewhere, 10.0, 20.0).await?;
    assert_eq!(
        other_browser.calls().last().map(|call| call.params.clone()),
        Some(json!({"type":"mouseMoved","x":10.0,"y":20.0}))
    );

    // A reconnect leaves the old session's registration behind.
    harness.native.epoch.fetch_add(1, Ordering::SeqCst);
    assert_eq!(input_scale(&session), 1.0);
    Ok(())
}

const WINDOW_METHODS: [&str; 6] = [
    "Browser.getWindowBounds",
    "Browser.setContentsSize",
    "Browser.setWindowBounds",
    "Browser.activateTab",
    "Browser.activateWindow",
    "Target.activateTarget",
];

fn window_calls(calls: &[Call]) -> Vec<(String, Value)> {
    sent(
        &calls
            .iter()
            .filter(|call| WINDOW_METHODS.contains(&call.method.as_str()))
            .cloned()
            .collect::<Vec<_>>(),
    )
}

fn fit(zoom: f64) -> Result<MobileModeRequest, MobileModeArgError> {
    MobileModeRequest::new(true, Some(zoom), Some(true))
}

fn bounds(left: i64, top: i64, width: i64, height: i64) -> WindowBounds {
    WindowBounds {
        left,
        top,
        width,
        height,
    }
}

#[test]
fn resize_window_is_validated_with_the_rest_of_the_request() {
    assert_eq!(
        MobileModeRequest::new(false, None, Some(true)),
        Err(MobileModeArgError::ResizeWindowWithoutMobile)
    );
    for (enabled, resize, window) in [
        (true, Some(true), WindowFit::Resize),
        (true, Some(false), WindowFit::Keep),
        (true, None, WindowFit::Keep),
        (false, Some(false), WindowFit::Keep),
        (false, None, WindowFit::Keep),
    ] {
        assert_eq!(
            MobileModeRequest::new(enabled, None, resize).map(|request| request.window),
            Ok(window)
        );
    }
}

#[tokio::test]
async fn a_default_toggle_never_touches_the_window() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    let on = harness
        .view
        .toggle_mobile_mode(page.clone(), mobile(150.0)?)
        .await?;
    let off = harness
        .view
        .toggle_mobile_mode(page.clone(), MobileMode::Desktop)
        .await?;
    assert_eq!((on.window, off.window), (None, None));
    assert!(window_calls(&harness.native.calls()).is_empty());
    Ok(())
}

#[tokio::test]
async fn resize_window_fits_the_content_area_and_reports_actual_bounds() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    let session = harness.browser.pages.get_session(page.clone()).await?;
    let mark = harness.native.mark();
    let outcome = harness
        .view
        .toggle_mobile_mode(page.clone(), fit(100.0)?)
        .await?;
    assert_eq!(
        outcome.window,
        Some(WindowOutcome::Fitted {
            window: WindowId(1),
            bounds: bounds(80, 44, 640, 987),
            requested: (400, 906),
            exact: (true, true),
        })
    );
    let calls = harness.native.since(mark);
    let size = |width: i64, height: i64| {
        (
            "Browser.setContentsSize".to_string(),
            json!({"windowId":1,"width":width,"height":height}),
        )
    };
    let read = ("Browser.getWindowBounds".to_string(), json!({"windowId":1}));
    assert_eq!(
        window_calls(&calls),
        vec![
            read.clone(),
            size(400, 906),
            read.clone(),
            size(401, 907),
            read.clone(),
            size(400, 906),
            read,
        ]
    );
    // The resize resets the tab's view to the content area, so the zoomed view is re-sent last.
    let last = calls.last().ok_or("no calls")?;
    assert_eq!(
        (last.method.as_str(), &last.params, &last.session),
        (
            VISIBLE,
            &json!({"width":400,"height":906}),
            &Some(session.session_id)
        )
    );
    Ok(())
}

#[tokio::test]
async fn a_clamped_fit_reports_the_limited_axis() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    let outcome = harness
        .view
        .toggle_mobile_mode(page.clone(), fit(50.0)?)
        .await?;
    assert_eq!(
        outcome.window,
        Some(WindowOutcome::Fitted {
            window: WindowId(1),
            bounds: bounds(80, 44, 626, 534),
            requested: (200, 453),
            exact: (false, true),
        })
    );
    Ok(())
}

#[tokio::test]
async fn a_window_that_is_not_normal_is_left_alone() -> TestResult {
    for state in ["maximized", "fullscreen", "minimized"] {
        let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
        harness.native.lock().window["windowState"] = json!(state);
        let outcome = harness
            .view
            .toggle_mobile_mode(page.clone(), fit(100.0)?)
            .await?;
        match outcome.window {
            Some(WindowOutcome::NotFitted {
                window: Some(WindowId(1)),
                reason,
            }) => assert!(
                reason.contains(state) && reason.contains("normal"),
                "{reason}"
            ),
            other => return Err(format!("{state}: {other:?}").into()),
        }
        assert_eq!(
            window_calls(&harness.native.calls()),
            vec![("Browser.getWindowBounds".to_string(), json!({"windowId":1}))]
        );
        assert_eq!(record_zoom(&harness, &page).await, Some(100.0));
    }
    Ok(())
}

#[tokio::test]
async fn disable_restores_a_window_nobody_touched_since_the_fit() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    harness
        .view
        .toggle_mobile_mode(page.clone(), fit(100.0)?)
        .await?;
    harness
        .view
        .toggle_mobile_mode(page.clone(), fit(150.0)?)
        .await?;
    let mark = harness.native.mark();
    let outcome = harness
        .view
        .toggle_mobile_mode(page.clone(), MobileMode::Desktop)
        .await?;
    assert_eq!(
        outcome.window,
        Some(WindowOutcome::Restored {
            window: WindowId(1),
            bounds: bounds(80, 44, 1200, 1000),
        })
    );
    assert!(window_calls(&harness.native.since(mark)).contains(&(
        "Browser.setWindowBounds".to_string(),
        json!({"windowId":1,"bounds":{"left":80,"top":44,"width":1200,"height":1000}})
    )));
    Ok(())
}

#[tokio::test]
async fn disable_leaves_a_window_that_changed_after_the_fit() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    harness
        .view
        .toggle_mobile_mode(page.clone(), fit(100.0)?)
        .await?;
    harness.native.lock().window["width"] = json!(900);
    let mark = harness.native.mark();
    let outcome = harness
        .view
        .toggle_mobile_mode(page.clone(), MobileMode::Desktop)
        .await?;
    match outcome.window {
        Some(WindowOutcome::NotRestored {
            window: WindowId(1),
            reason,
        }) => assert!(reason.contains("left as is"), "{reason}"),
        other => return Err(format!("{other:?}").into()),
    }
    assert!(
        window_calls(&harness.native.since(mark))
            .iter()
            .all(|(method, _)| method == "Browser.getWindowBounds")
    );
    assert_eq!(harness.native.lock().window["width"], json!(900));
    Ok(())
}

#[tokio::test]
async fn a_failed_fit_keeps_the_mode_and_leaves_nothing_to_restore() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    harness.native.fail("Browser.setContentsSize", 1);
    let outcome = harness
        .view
        .toggle_mobile_mode(page.clone(), fit(100.0)?)
        .await?;
    match outcome.window {
        Some(WindowOutcome::NotFitted { reason, .. }) => {
            assert!(reason.contains("Browser.setContentsSize"), "{reason}");
        }
        other => return Err(format!("{other:?}").into()),
    }
    assert_eq!(record_zoom(&harness, &page).await, Some(100.0));
    let off = harness
        .view
        .toggle_mobile_mode(page.clone(), MobileMode::Desktop)
        .await?;
    assert_eq!(off.window, None);
    Ok(())
}

#[tokio::test]
async fn resize_window_without_enable_is_refused_before_any_command() -> TestResult {
    let (harness, page) = harness(vec![tab(1, "about:blank")]).await?;
    let mark = harness.native.mark();
    let request = MobileModeRequest {
        mode: MobileMode::Desktop,
        window: WindowFit::Resize,
    };
    assert!(
        harness
            .view
            .toggle_mobile_mode(page, request)
            .await
            .is_err()
    );
    assert!(harness.native.since(mark).is_empty());
    Ok(())
}
