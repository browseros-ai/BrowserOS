use std::{collections::HashSet, future::Future, time::Duration};

use browseros_cdp::CdpError;
use browseros_core::{
    BrowserView, CoreError, FrameId, PageId, ProtocolSession, TargetError,
    screenshot::{ScreenshotCaptureOptions, ScreenshotFormat, Viewport},
};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::types::{DocumentId, FrameRef, PageError, Rect};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const VISIBLE_RECT: &str = r"function(selector) {
    for (const element of this.querySelectorAll(selector)) {
        const r = element.getBoundingClientRect();
        if (r.width > 0 && r.height > 0 && element.checkVisibility({checkOpacity:true,checkVisibilityCSS:true}))
            return {x:r.x,y:r.y,width:r.width,height:r.height};
    }
    return null;
}";

pub trait CaptchaPage: Send + Sync {
    fn eval_main(&self, expression: &str) -> impl Future<Output = Result<Value, PageError>> + Send;
    /// Hidden and zero-size frames retain their measured rect; callers must not click or capture them.
    fn frames(&self) -> impl Future<Output = Result<Vec<FrameRef>, PageError>> + Send;
    fn eval_in_frame(
        &self,
        frame: &FrameRef,
        expression: &str,
    ) -> impl Future<Output = Result<Value, PageError>> + Send;
    fn click_at(&self, x: f64, y: f64) -> impl Future<Output = Result<(), PageError>> + Send;
    fn screenshot_clip(&self, rect: Rect)
    -> impl Future<Output = Result<String, PageError>> + Send;
    fn document(&self) -> impl Future<Output = Result<DocumentId, PageError>> + Send;
    /// Viewport rect (top-level CSS px) of the first visible element matching a CSS selector
    /// inside `frame`, searching through closed shadow roots (CDP DOM.getDocument pierce).
    /// None when nothing visible matches.
    fn query_rect_in_frame(
        &self,
        frame: &FrameRef,
        selector: &str,
    ) -> impl Future<Output = Result<Option<Rect>, PageError>> + Send;
}

pub struct LivePage {
    pub view: BrowserView,
    pub page: PageId,
    last_point: Mutex<(f64, f64)>,
}

impl LivePage {
    pub fn new(view: BrowserView, page: PageId) -> Self {
        Self {
            view,
            page,
            last_point: Mutex::new((0.0, 0.0)),
        }
    }
    async fn top(&self) -> Result<ProtocolSession, PageError> {
        Ok(browser(self.view.page_session(self.page.clone()))
            .await?
            .session)
    }
    async fn frame(&self, id: &str) -> Result<ProtocolSession, PageError> {
        browser(
            self.view
                .frame_session(self.page.clone(), FrameId(id.to_owned())),
        )
        .await
    }
}

impl CaptchaPage for LivePage {
    async fn eval_main(&self, expression: &str) -> Result<Value, PageError> {
        evaluate(&self.top().await?, expression, None).await
    }
    async fn frames(&self) -> Result<Vec<FrameRef>, PageError> {
        let top = self.top().await?;
        let tree = send(&top, "Page.getFrameTree", json!({})).await?;
        let main = &tree["frameTree"]["frame"];
        let viewport = self
            .eval_main("({width:innerWidth,height:innerHeight})")
            .await?;
        let origin = Rect {
            x: 0.0,
            y: 0.0,
            width: number(&viewport, "width")?,
            height: number(&viewport, "height")?,
        };
        let mut result = vec![FrameRef {
            frame_id: text(main, "id")?,
            url: text(main, "url")?,
            rect: Some(origin),
        }];
        let mut seen_frames = HashSet::from([result[0].frame_id.clone()]);
        let mut seen_sessions = HashSet::new();
        let mut pending = vec![(top, Some(origin))];
        while let Some((parent, origin)) = pending.pop() {
            if !seen_sessions.insert(parent.session_id().cloned()) {
                continue;
            }
            let dom = send(
                &parent,
                "DOM.getDocument",
                json!({"depth":-1,"pierce":true}),
            )
            .await?;
            let mut owners = Vec::new();
            collect_owners(&dom["root"], &mut owners);
            for id in owners {
                if !seen_frames.insert(id.clone()) {
                    continue;
                }
                let session = self.frame(&id).await?;
                let tree = send(&session, "Page.getFrameTree", json!({})).await?;
                let frame = find_frame(&tree["frameTree"], &id)
                    .ok_or_else(|| bad("Frame missing from its session tree"))?;
                let measured = async {
                    let owner = send(&parent, "DOM.getFrameOwner", json!({"frameId":id})).await?;
                    let model = send(
                        &parent,
                        "DOM.getBoxModel",
                        json!({"backendNodeId":owner["backendNodeId"]}),
                    )
                    .await?;
                    quad_rect(&model["model"]["content"])
                }
                .await;
                let rect = match measured {
                    Ok(local) => origin.map(|origin| translate(local, origin)),
                    Err(PageError::Browser(_)) => None,
                    Err(error) => return Err(error),
                };
                result.push(FrameRef {
                    frame_id: id,
                    url: text(frame, "url")?,
                    rect,
                });
                if !session.same_session(&parent) {
                    pending.push((session, rect));
                }
            }
        }
        Ok(result)
    }
    async fn eval_in_frame(&self, frame: &FrameRef, expression: &str) -> Result<Value, PageError> {
        let session = self.frame(&frame.frame_id).await?;
        let world = send(
            &session,
            "Page.createIsolatedWorld",
            json!({"frameId":frame.frame_id,"worldName":"browseros-captcha-read"}),
        )
        .await?;
        let context = world["executionContextId"]
            .as_i64()
            .ok_or_else(|| bad("Missing frame execution context"))?;
        evaluate(&session, expression, Some(context)).await
    }
    async fn click_at(&self, x: f64, y: f64) -> Result<(), PageError> {
        if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
            return Err(bad("Invalid click point"));
        }
        let mut last = self.last_point.lock().await;
        let session = self.top().await?;
        for step in 1..=4 {
            let fraction = f64::from(step) / 4.0;
            send(&session,"Input.dispatchMouseEvent",json!({"type":"mouseMoved","x":last.0+(x-last.0)*fraction,"y":last.1+(y-last.1)*fraction})).await?;
            tokio::time::sleep(Duration::from_millis(12)).await;
        }
        *last = (x, y);
        send(
            &session,
            "Input.dispatchMouseEvent",
            json!({"type":"mousePressed","x":x,"y":y,"button":"left","clickCount":1}),
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(90)).await;
        send(
            &session,
            "Input.dispatchMouseEvent",
            json!({"type":"mouseReleased","x":x,"y":y,"button":"left","clickCount":1}),
        )
        .await?;
        Ok(())
    }
    async fn screenshot_clip(&self, rect: Rect) -> Result<String, PageError> {
        if !valid_rect(rect) || rect.x < 0.0 || rect.y < 0.0 {
            return Err(bad("Invalid screenshot clip"));
        }
        let scroll = self.eval_main("({x:scrollX,y:scrollY})").await?;
        let capture = browser(self.view.screenshot(
            self.page.clone(),
            ScreenshotCaptureOptions {
                format: Some(ScreenshotFormat::Png),
                full_page: Some(false),
                annotate: Some(false),
                clip: Some(Viewport {
                    x: rect.x + number(&scroll, "x")?,
                    y: rect.y + number(&scroll, "y")?,
                    width: rect.width,
                    height: rect.height,
                    scale: 1.0,
                }),
                ..Default::default()
            },
        ))
        .await?;
        Ok(capture.data)
    }
    async fn document(&self) -> Result<DocumentId, PageError> {
        let tree = send(&self.top().await?, "Page.getFrameTree", json!({})).await?;
        let frame = &tree["frameTree"]["frame"];
        Ok(DocumentId {
            url: text(frame, "url")?,
            loader_id: text(frame, "loaderId")?,
        })
    }
    async fn query_rect_in_frame(
        &self,
        frame: &FrameRef,
        selector: &str,
    ) -> Result<Option<Rect>, PageError> {
        let origin = frame
            .rect
            .ok_or_else(|| bad("Frame viewport origin is unavailable"))?;
        if !valid_rect(origin) {
            return Ok(None);
        }
        let session = self.frame(&frame.frame_id).await?;
        let tree = send(&session, "Page.getFrameTree", json!({})).await?;
        let dom = send(
            &session,
            "DOM.getDocument",
            json!({"depth":-1,"pierce":true}),
        )
        .await?;
        let document = if tree["frameTree"]["frame"]["id"] == frame.frame_id {
            Some(&dom["root"])
        } else {
            find_document(&dom["root"], &frame.frame_id)
        }
        .ok_or_else(|| bad("Frame document is unavailable"))?;
        let mut roots = vec![document];
        collect_shadow_roots(document, &mut roots);
        for root in roots {
            let resolved = send(
                &session,
                "DOM.resolveNode",
                json!({"backendNodeId":root["backendNodeId"]}),
            )
            .await?;
            let object = text(&resolved["object"], "objectId")?;
            let response = send(&session,"Runtime.callFunctionOn",json!({"objectId":object,"functionDeclaration":VISIBLE_RECT,"arguments":[{"value":selector}],"returnByValue":true,"awaitPromise":true})).await;
            let _ = send(
                &session,
                "Runtime.releaseObject",
                json!({"objectId":object}),
            )
            .await;
            let value = evaluation_value(response?)?;
            if !value.is_null() {
                let rect: Rect =
                    serde_json::from_value(value).map_err(|error| bad(&error.to_string()))?;
                if valid_rect(rect) {
                    return Ok(Some(translate(rect, origin)));
                }
            }
        }
        Ok(None)
    }
}

async fn browser<T>(future: impl Future<Output = Result<T, CoreError>>) -> Result<T, PageError> {
    tokio::time::timeout(REQUEST_TIMEOUT, future)
        .await
        .map_err(|_| PageError::Timeout)?
        .map_err(page_error)
}
async fn send(session: &ProtocolSession, method: &str, params: Value) -> Result<Value, PageError> {
    browser(session.send_value(method, params)).await
}
async fn evaluate(
    session: &ProtocolSession,
    expression: &str,
    context: Option<i64>,
) -> Result<Value, PageError> {
    let mut params =
        json!({"expression":expression,"returnByValue":true,"awaitPromise":true,"timeout":2000});
    if let Some(context) = context {
        params["contextId"] = json!(context);
    }
    evaluation_value(send(session, "Runtime.evaluate", params).await?)
}
fn evaluation_value(response: Value) -> Result<Value, PageError> {
    if let Some(details) = response.get("exceptionDetails") {
        return Err(bad(details["exception"]["description"]
            .as_str()
            .or_else(|| details["text"].as_str())
            .unwrap_or("JavaScript exception")));
    }
    response
        .get("result")
        .ok_or_else(|| bad("Missing JavaScript result"))?;
    Ok(response["result"]["value"].clone())
}
fn page_error(error: CoreError) -> PageError {
    match error {
        CoreError::UnknownPage(_)
        | CoreError::UnknownPageShort(_)
        | CoreError::Target(TargetError::PageTargetChanged(_))
        | CoreError::Cdp(
            CdpError::SessionGone
            | CdpError::NotConnected
            | CdpError::ConnectionLost
            | CdpError::ConnectionEpochChanged { .. },
        ) => PageError::Closed,
        CoreError::DocumentChanged => PageError::Navigated,
        CoreError::Cdp(CdpError::Timeout { .. }) => PageError::Timeout,
        CoreError::Cdp(CdpError::Protocol { code: -32800, .. }) => PageError::Cancelled,
        CoreError::Cdp(CdpError::Protocol { ref message, .. })
            if message.to_ascii_lowercase().contains("cancelled")
                || message.to_ascii_lowercase().contains("canceled") =>
        {
            PageError::Cancelled
        }
        error => bad(&error.to_string()),
    }
}
fn bad(message: &str) -> PageError {
    PageError::Browser(message.to_owned())
}
fn text(value: &Value, key: &str) -> Result<String, PageError> {
    value[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| bad(&format!("Missing {key}")))
}
fn number(value: &Value, key: &str) -> Result<f64, PageError> {
    value[key]
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| bad(&format!("Invalid {key}")))
}
fn valid_rect(rect: Rect) -> bool {
    [rect.x, rect.y, rect.width, rect.height]
        .into_iter()
        .all(f64::is_finite)
        && rect.width > 0.0
        && rect.height > 0.0
}
fn translate(rect: Rect, origin: Rect) -> Rect {
    Rect {
        x: rect.x + origin.x,
        y: rect.y + origin.y,
        ..rect
    }
}
fn quad_rect(quad: &Value) -> Result<Rect, PageError> {
    let points = quad
        .as_array()
        .filter(|points| points.len() == 8)
        .ok_or_else(|| bad("Invalid frame content quad"))?;
    let values = points
        .iter()
        .map(|v| {
            v.as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| bad("Invalid frame content quad"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let x = values
        .iter()
        .step_by(2)
        .copied()
        .fold(f64::INFINITY, f64::min);
    let y = values
        .iter()
        .skip(1)
        .step_by(2)
        .copied()
        .fold(f64::INFINITY, f64::min);
    Ok(Rect {
        x,
        y,
        width: values
            .iter()
            .step_by(2)
            .copied()
            .fold(f64::NEG_INFINITY, f64::max)
            - x,
        height: values
            .iter()
            .skip(1)
            .step_by(2)
            .copied()
            .fold(f64::NEG_INFINITY, f64::max)
            - y,
    })
}
fn collect_owners(node: &Value, frames: &mut Vec<String>) {
    if let Some(id) = node["frameId"].as_str() {
        frames.push(id.to_owned());
    }
    for key in ["children", "shadowRoots"] {
        for child in node[key].as_array().into_iter().flatten() {
            collect_owners(child, frames);
        }
    }
    if let Some(document) = node.get("contentDocument") {
        collect_owners(document, frames);
    }
}
fn find_frame<'a>(tree: &'a Value, id: &str) -> Option<&'a Value> {
    if tree["frame"]["id"] == id {
        return Some(&tree["frame"]);
    }
    tree["childFrames"]
        .as_array()?
        .iter()
        .find_map(|child| find_frame(child, id))
}
fn find_document<'a>(node: &'a Value, id: &str) -> Option<&'a Value> {
    if node["frameId"] == id {
        return node.get("contentDocument");
    }
    for key in ["children", "shadowRoots"] {
        for child in node[key].as_array().into_iter().flatten() {
            if let Some(document) = find_document(child, id) {
                return Some(document);
            }
        }
    }
    node.get("contentDocument")
        .and_then(|document| find_document(document, id))
}
fn collect_shadow_roots<'a>(node: &'a Value, roots: &mut Vec<&'a Value>) {
    for root in node["shadowRoots"].as_array().into_iter().flatten() {
        roots.push(root);
        collect_shadow_roots(root, roots);
    }
    for child in node["children"].as_array().into_iter().flatten() {
        collect_shadow_roots(child, roots);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use browseros_cdp::{CdpError, CdpEvent};
    use browseros_core::{BrowserSession, CdpConnection, SessionId};
    use futures_util::future::BoxFuture;
    use serde_json::json;
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };
    use tokio::sync::broadcast;

    type TestResult = Result<(), Box<dyn std::error::Error>>;
    type Call = (String, Value, Option<SessionId>);
    struct Native {
        calls: Mutex<Vec<Call>>,
        evaluations: Mutex<VecDeque<Result<Value, CdpError>>>,
        events: broadcast::Sender<CdpEvent>,
    }
    impl Native {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                evaluations: Mutex::new(VecDeque::new()),
                events: broadcast::channel(32).0,
            })
        }
        fn reply(
            &self,
            method: &str,
            params: Value,
            session: Option<&SessionId>,
        ) -> Result<Value, CdpError> {
            self.calls.lock().map_err(|_| CdpError::SessionGone)?.push((
                method.into(),
                params.clone(),
                session.cloned(),
            ));
            let child = session.is_some_and(|id| id.as_str() == "s-widget");
            let tab = json!({"tabId":1,"targetId":"page","url":"http://fixture.test","title":"fixture","isActive":true,"isLoading":false,"loadProgress":1.0,"isPinned":false,"isHidden":false,"windowId":1});
            Ok(match method {
                "Browser.getTabs" => json!({"tabs":[tab]}),
                "Browser.getTabInfo" => json!({"tab":tab}),
                "Target.attachToTarget" => {
                    json!({"sessionId":format!("s-{}",params["targetId"].as_str().unwrap_or_default())})
                }
                "Page.getFrameTree" if child => {
                    json!({"frameTree":{"frame":{"id":"widget","parentId":"root","url":"https://widget.test","loaderId":"widget-loader"}}})
                }
                "Page.getFrameTree" => {
                    json!({"frameTree":{"frame":{"id":"root","url":"http://fixture.test","loaderId":"main-loader"}}})
                }
                "DOM.getDocument" if child => {
                    json!({"root":{"nodeType":9,"backendNodeId":40,"children":[{"nodeName":"BODY","nodeType":1,"backendNodeId":41,"shadowRoots":[{"shadowRootType":"closed","nodeType":11,"backendNodeId":42,"children":[{"nodeName":"INPUT","nodeType":1,"backendNodeId":43}]}]}]}})
                }
                "DOM.getDocument" => {
                    json!({"root":{"nodeType":9,"backendNodeId":1,"children":[{"shadowRoots":[{"shadowRootType":"closed","children":[{"nodeName":"IFRAME","frameId":"widget","backendNodeId":5}]}]}]}})
                }
                "DOM.getFrameOwner" => json!({"backendNodeId":5}),
                "DOM.describeNode" => json!({"node":{"frameId":"widget"}}),
                "DOM.getBoxModel" => json!({"model":{"content":[30,40,330,40,330,105,30,105]}}),
                "Page.createIsolatedWorld" => json!({"executionContextId":77}),
                "Runtime.evaluate" => {
                    if let Some(result) = self
                        .evaluations
                        .lock()
                        .map_err(|_| CdpError::SessionGone)?
                        .pop_front()
                    {
                        return result;
                    }
                    let expression = params["expression"].as_str().unwrap_or_default();
                    json!({"result":{"value":if expression.contains("scrollX") {json!({"x":125,"y":300})} else if expression.contains("innerWidth") {json!({"width":800,"height":600})} else if params["contextId"] == 77 {json!("https://widget.test")} else {json!("main")}}})
                }
                "DOM.resolveNode" => {
                    json!({"object":{"objectId":format!("node-{}",params["backendNodeId"])}})
                }
                "Runtime.callFunctionOn" => {
                    json!({"result":{"value":if params["objectId"] == "node-42" && params["arguments"][0]["value"] == "input[type=checkbox]" {json!({"x":9,"y":20,"width":24,"height":24})} else {Value::Null}}})
                }
                "Page.captureScreenshot" => json!({"data":"iVBORw0KGgo="}),
                "Page.enable"
                | "DOM.enable"
                | "Runtime.enable"
                | "Accessibility.enable"
                | "Target.setAutoAttach"
                | "Runtime.releaseObject"
                | "Input.dispatchMouseEvent" => json!({}),
                _ => {
                    return Err(CdpError::Protocol {
                        code: -1,
                        message: format!("unexpected {method}"),
                    });
                }
            })
        }
        fn calls(&self, method: &str) -> Result<Vec<Call>, String> {
            Ok(self
                .calls
                .lock()
                .map_err(|_| "poisoned")?
                .iter()
                .filter(|call| call.0 == method)
                .cloned()
                .collect())
        }
    }
    impl CdpConnection for Native {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move { self.reply(method, params, session) })
        }
        fn send_raw_json<'a>(
            &'a self,
            method: &'a str,
            params: &'a str,
            session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async move {
                Ok(serde_json::to_string(&self.reply(
                    method,
                    serde_json::from_str(params)?,
                    session,
                )?)?)
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
    fn fixture() -> (Arc<Native>, LivePage) {
        let native = Native::new();
        let view = BrowserView::unbound(
            BrowserSession::new(native.clone(), Default::default()),
            None,
        );
        (native, LivePage::new(view, PageId(1)))
    }
    fn widget() -> FrameRef {
        FrameRef {
            frame_id: "widget".into(),
            url: "https://widget.test".into(),
            rect: Some(Rect {
                x: 30.0,
                y: 40.0,
                width: 300.0,
                height: 65.0,
            }),
        }
    }

    #[tokio::test]
    async fn tests_that_main_and_child_evaluation_use_the_right_worlds() -> TestResult {
        let (native, page) = fixture();
        assert_eq!(page.eval_main("location.href").await?, json!("main"));
        assert_eq!(
            page.eval_in_frame(&widget(), "location.href").await?,
            json!("https://widget.test")
        );
        let calls = native.calls("Runtime.evaluate")?;
        assert!(calls[0].1.get("contextId").is_none());
        assert_eq!(calls[1].1["contextId"], 77);
        assert_eq!(calls[1].2.as_ref().map(SessionId::as_str), Some("s-widget"));
        assert_eq!(calls[0].1["awaitPromise"], true);
        assert_eq!(calls[0].1["returnByValue"], true);
        Ok(())
    }
    #[tokio::test]
    async fn tests_that_exceptions_and_transport_failures_are_typed() -> TestResult {
        let (native, page) = fixture();
        let cases = [
            (
                Ok(
                    json!({"exceptionDetails":{"text":"Uncaught","exception":{"description":"Error: broken"}}}),
                ),
                PageError::Browser("Error: broken".into()),
            ),
            (Err(CdpError::SessionGone), PageError::Closed),
            (
                Err(CdpError::Timeout {
                    method: "Runtime.evaluate".into(),
                }),
                PageError::Timeout,
            ),
            (
                Err(CdpError::Protocol {
                    code: -32800,
                    message: "cancelled".into(),
                }),
                PageError::Cancelled,
            ),
            (
                Err(CdpError::Protocol {
                    code: -32000,
                    message: "other".into(),
                }),
                PageError::Browser("CDP error: other".into()),
            ),
        ];
        for (response, expected) in cases {
            native
                .evaluations
                .lock()
                .map_err(|_| "poisoned")?
                .push_back(response);
            assert_eq!(page.eval_main("1").await, Err(expected));
        }
        Ok(())
    }
    #[tokio::test]
    async fn tests_that_frames_include_closed_shadow_owners_and_document_identity() -> TestResult {
        let (_, page) = fixture();
        let frames = page.frames().await?;
        assert_eq!(frames.len(), 2);
        assert!(frames.contains(&widget()));
        assert_eq!(
            page.document().await?,
            DocumentId {
                url: "http://fixture.test".into(),
                loader_id: "main-loader".into()
            }
        );
        Ok(())
    }
    #[tokio::test]
    async fn tests_that_closed_shadow_rects_are_translated_and_missing_selectors_return_none()
    -> TestResult {
        let (_, page) = fixture();
        let rect = page
            .query_rect_in_frame(&widget(), "input[type=checkbox]")
            .await?
            .ok_or("missing rect")?;
        assert_eq!(
            rect,
            Rect {
                x: 39.0,
                y: 60.0,
                width: 24.0,
                height: 24.0
            }
        );
        assert_eq!(rect.center(), (51.0, 72.0));
        assert_eq!(page.query_rect_in_frame(&widget(), "#absent").await?, None);
        Ok(())
    }
    #[tokio::test]
    async fn tests_that_screenshot_clips_add_scroll_without_multiplying_by_dpr() -> TestResult {
        let (native, page) = fixture();
        assert_eq!(
            page.screenshot_clip(Rect {
                x: 10.0,
                y: 20.0,
                width: 200.0,
                height: 100.0
            })
            .await?,
            "iVBORw0KGgo="
        );
        let calls = native.calls("Page.captureScreenshot")?;
        assert_eq!(
            calls[0].1["clip"],
            json!({"x":135.0,"y":320.0,"width":200.0,"height":100.0,"scale":1.0})
        );
        assert_eq!(calls[0].1["format"], "png");
        assert!(
            page.screenshot_clip(Rect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 2.0
            })
            .await
            .is_err()
        );
        Ok(())
    }
    #[tokio::test(start_paused = true)]
    async fn tests_that_clicks_move_press_pause_and_release_on_the_top_page() -> TestResult {
        let (native, page) = fixture();
        let started = tokio::time::Instant::now();
        page.click_at(60.0, 90.0).await?;
        assert!(started.elapsed() >= std::time::Duration::from_millis(60));
        let calls = native.calls("Input.dispatchMouseEvent")?;
        let n = calls.len();
        assert!(n >= 5);
        assert!(calls[..n - 2].iter().all(|c| c.1["type"] == "mouseMoved"));
        assert_eq!(calls[n - 2].1["type"], "mousePressed");
        assert_eq!(calls[n - 1].1["type"], "mouseReleased");
        assert!(
            calls
                .iter()
                .all(|c| c.2.as_ref().map(SessionId::as_str) == Some("s-page"))
        );
        Ok(())
    }
}
