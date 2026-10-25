use crate::{
    CoreError, FrameId, ProtocolSession, input::geometry::resolve_object_id, observer::Observer,
    pointer, snapshot::RefEntry,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex as StdMutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
use tokio::time::timeout;

const SCREENSHOT_CAPTURE_DEADLINE: Duration = Duration::from_millis(1_200);
const SCREENCAST_FRAME_DEADLINE: Duration = Duration::from_secs(2);

const OVERLAY_ATTR: &str = "data-browseros-screenshot-annotation";
const OVERLAY_SCRIPT: &str = include_str!("../assets/screenshot-overlay.js");
static OVERLAY_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenshotFormat {
    Png,
    Jpeg,
    Webp,
}

impl ScreenshotFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Webp => "webp",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

#[derive(Debug, Clone, Default)]
pub struct ScreenshotCaptureOptions {
    pub format: Option<ScreenshotFormat>,
    pub quality: Option<i64>,
    pub full_page: Option<bool>,
    pub annotate: Option<bool>,
    pub clip: Option<Viewport>,
}

/// Viewport boxes are viewport-relative and apply the clip scale. Full-page
/// boxes add the sampled scroll offsets when that best-effort read succeeds.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenshotAnnotationBox {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScreenshotAnnotation {
    pub ref_id: String,
    pub number: i64,
    pub role: String,
    pub name: Option<String>,
    pub box_: ScreenshotAnnotationBox,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScreenshotCaptureResult {
    pub data: String,
    pub mime_type: String,
    pub annotations: Vec<ScreenshotAnnotation>,
}

#[derive(Debug, Clone)]
struct RawAnnotation {
    ref_id: String,
    number: i64,
    role: String,
    name: Option<String>,
    rect: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Deserialize)]
struct CaptureScreenshotResult {
    data: String,
}

#[derive(Debug, Deserialize)]
struct RuntimeResult {
    result: RemoteObject,
}

#[derive(Debug, Deserialize)]
struct RemoteObject {
    value: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameOwnerResult {
    backend_node_id: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameTreeResult {
    frame_tree: FrameTreeNode,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameTreeNode {
    frame: Frame,
    child_frames: Option<Vec<FrameTreeNode>>,
}

#[derive(Debug, Deserialize)]
struct Frame {
    id: String,
}

pub async fn capture_screenshot_with_annotations(
    page_session: ProtocolSession,
    observer: Arc<Observer>,
    options: ScreenshotCaptureOptions,
) -> Result<ScreenshotCaptureResult, CoreError> {
    if !options.annotate.unwrap_or(false) {
        return run_exclusive_screenshot_capture(&page_session, || async {
            capture_plain_screenshot(&page_session, &options).await
        })
        .await;
    }

    run_exclusive_screenshot_capture(&page_session, || async {
        capture_annotated_screenshot(page_session.clone(), observer.clone(), options.clone()).await
    })
    .await
}

async fn capture_plain_screenshot(
    page_session: &ProtocolSession,
    options: &ScreenshotCaptureOptions,
) -> Result<ScreenshotCaptureResult, CoreError> {
    let format = options.format.unwrap_or(ScreenshotFormat::Png);
    let hidden = pointer::hide_for_capture(page_session).await;
    let result = capture_image(page_session, options).await;
    if hidden {
        pointer::restore_after_capture(page_session).await;
    }
    let result = result?;
    Ok(ScreenshotCaptureResult {
        data: result.data,
        mime_type: format!("image/{}", format.as_str()),
        annotations: Vec::new(),
    })
}

async fn capture_annotated_screenshot(
    page_session: ProtocolSession,
    observer: Arc<Observer>,
    options: ScreenshotCaptureOptions,
) -> Result<ScreenshotCaptureResult, CoreError> {
    let format = options.format.unwrap_or(ScreenshotFormat::Png);
    let full_page = options.full_page.unwrap_or(false);
    let token = create_overlay_token();
    let object_group = token.clone();
    let mut object_sessions = Vec::new();
    let mut overlay_injected = false;

    let result = async {
        let capture_area = if full_page {
            None
        } else if let Some(clip) = options.clip {
            Some(Rect {
                x: 0.0,
                y: 0.0,
                width: clip.width,
                height: clip.height,
            })
        } else {
            read_viewport_rect(&page_session).await.ok()
        };
        let annotations = clip_annotations(
            collect_annotations(
                &page_session,
                &observer,
                &object_group,
                &mut object_sessions,
            )
            .await?,
            capture_area,
        );
        let scroll = if full_page {
            read_scroll_offsets(&page_session).await.ok()
        } else {
            None
        };
        if !annotations.is_empty() {
            inject_annotation_overlay(&page_session, &token, full_page, &annotations, scroll)
                .await?;
            overlay_injected = true;
        }
        let hidden = pointer::hide_for_capture(&page_session).await;
        let captured = capture_image(&page_session, &options).await;
        if hidden {
            pointer::restore_after_capture(&page_session).await;
        }
        let captured = captured?;
        Ok(ScreenshotCaptureResult {
            data: captured.data,
            mime_type: format!("image/{}", format.as_str()),
            annotations: project_annotations(
                &annotations,
                scroll,
                if full_page {
                    None
                } else {
                    options.clip.map(|clip| clip.scale)
                },
            ),
        })
    }
    .await;

    if overlay_injected {
        let _ = remove_annotation_overlay(&page_session, &token).await;
    }
    release_object_group(&object_sessions, &object_group).await;
    result
}

async fn capture_image(
    page_session: &ProtocolSession,
    options: &ScreenshotCaptureOptions,
) -> Result<CaptureScreenshotResult, CoreError> {
    let format = options.format.unwrap_or(ScreenshotFormat::Png);
    let full_page = options.full_page.unwrap_or(false);
    let params = screenshot_params(format, options.quality, full_page, options.clip);

    // 部分 Windows 云桌面最小化后，Chromium 的 Surface 截图可能一直不返回。
    // 仅对 JPEG 视口截图设置短等待，正常路径保持原有格式、裁剪和 CDP 行为。
    if format != ScreenshotFormat::Jpeg || full_page {
        return page_session.send("Page.captureScreenshot", params).await;
    }
    match timeout(
        SCREENSHOT_CAPTURE_DEADLINE,
        page_session.send("Page.captureScreenshot", params),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => capture_screencast_frame(page_session, options).await,
    }
}

async fn capture_screencast_frame(
    page_session: &ProtocolSession,
    options: &ScreenshotCaptureOptions,
) -> Result<CaptureScreenshotResult, CoreError> {
    let mut params = json!({
        "format": "jpeg",
        "quality": options.quality.unwrap_or(80),
        "maxFramesInFlight": 1
    });
    if let Some(clip) = options.clip {
        // Screencast 返回的是完整视口，不支持任意子区域裁剪，不能悄悄返回错误图片。
        let metrics: Value = page_session
            .send("Page.getLayoutMetrics", json!({}))
            .await?;
        let viewport = metrics
            .get("cssLayoutViewport")
            .or_else(|| metrics.get("layoutViewport"));
        let width = viewport
            .and_then(|value| value.get("clientWidth"))
            .and_then(Value::as_f64);
        let height = viewport
            .and_then(|value| value.get("clientHeight"))
            .and_then(Value::as_f64);
        if width.is_none_or(|width| (width - clip.width).abs() > 1.0)
            || height.is_none_or(|height| (height - clip.height).abs() > 1.0)
        {
            return Err(CoreError::Message(
                "CDP screenshot timed out; screencast cannot preserve a partial clip".into(),
            ));
        }
        params["maxWidth"] = json!((clip.width * clip.scale).round().max(1.0) as u32);
        params["maxHeight"] = json!((clip.height * clip.scale).round().max(1.0) as u32);
    }

    // 在发送开始命令前订阅，防止 Chromium 的第一帧在订阅前丢失。
    let mut events = page_session.subscribe_events();
    timeout(
        SCREENCAST_FRAME_DEADLINE,
        page_session.send::<_, Value>("Page.startScreencast", params),
    )
    .await
    .map_err(|_| CoreError::Message("timed out starting screenshot screencast".into()))??;

    let frame = timeout(SCREENCAST_FRAME_DEADLINE, async {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(error) => {
                    return Err(CoreError::Message(format!(
                        "screencast event stream: {error}"
                    )));
                }
            };
            if event.method != "Page.screencastFrame"
                || event.session_id.as_ref() != page_session.session_id()
            {
                continue;
            }
            if let Some(data) = event.params.get("data").and_then(Value::as_str)
                && !data.is_empty()
            {
                return Ok(CaptureScreenshotResult {
                    data: data.to_string(),
                });
            }
        }
    })
    .await
    .map_err(|_| CoreError::Message("timed out waiting for screenshot screencast frame".into()));

    // 无论有没有得到帧都停止 screencast，且不能再被 60 秒的 CDP 默认超时拖住。
    timeout(
        Duration::from_millis(500),
        page_session.send::<_, Value>("Page.stopScreencast", json!({})),
    )
    .await
    .map_err(|_| CoreError::Message("timed out stopping screenshot screencast".into()))??;
    frame?
}

fn screenshot_params(
    format: ScreenshotFormat,
    quality: Option<i64>,
    full_page: bool,
    clip: Option<Viewport>,
) -> Value {
    let mut params = serde_json::Map::new();
    params.insert(
        "format".to_string(),
        Value::String(format.as_str().to_string()),
    );
    params.insert("fromSurface".to_string(), Value::Bool(true));
    params.insert("captureBeyondViewport".to_string(), Value::Bool(full_page));
    if format != ScreenshotFormat::Png
        && let Some(quality) = quality
    {
        params.insert("quality".to_string(), Value::from(quality));
    }
    if !full_page && let Some(clip) = clip {
        params.insert(
            "clip".to_string(),
            json!({
                "x": clip.x,
                "y": clip.y,
                "width": clip.width,
                "height": clip.height,
                "scale": clip.scale
            }),
        );
    }
    Value::Object(params)
}

async fn collect_annotations(
    page_session: &ProtocolSession,
    observer: &Observer,
    object_group: &str,
    object_sessions: &mut Vec<ProtocolSession>,
) -> Result<Vec<RawAnnotation>, CoreError> {
    let snapshot = observer.snapshot().await?;
    let mut entries = snapshot
        .refs
        .entries_in_order()
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| annotation_number(entry.ref_id.as_str()));

    let mut out = Vec::new();
    for entry in entries {
        if let Some(annotation) = collect_annotation(
            page_session,
            observer,
            object_group,
            object_sessions,
            &entry,
        )
        .await
        {
            out.push(annotation);
        }
    }
    Ok(out)
}

async fn collect_annotation(
    page_session: &ProtocolSession,
    observer: &Observer,
    object_group: &str,
    object_sessions: &mut Vec<ProtocolSession>,
    entry: &RefEntry,
) -> Option<RawAnnotation> {
    let resolved = observer.resolve_ref(&entry.ref_id).await.ok()?;
    let local_rect = read_element_rect(
        &resolved.session,
        resolved.backend_node_id,
        object_group,
        object_sessions,
    )
    .await?;
    let rect = if let Some(frame_id) = &entry.frame_id {
        project_frame_rect(
            page_session,
            object_group,
            object_sessions,
            frame_id,
            local_rect,
        )
        .await?
    } else {
        local_rect
    };
    Some(RawAnnotation {
        ref_id: entry.ref_id.as_str().to_string(),
        number: annotation_number(entry.ref_id.as_str()),
        role: entry.role.clone(),
        name: (!entry.name.is_empty()).then(|| entry.name.clone()),
        rect,
    })
}

async fn project_frame_rect(
    page_session: &ProtocolSession,
    object_group: &str,
    object_sessions: &mut Vec<ProtocolSession>,
    frame_id: &FrameId,
    rect: Rect,
) -> Option<Rect> {
    if frame_depth(page_session, frame_id).await.ok().flatten()? != 1 {
        return None;
    }
    let owner = page_session
        .send::<_, FrameOwnerResult>("DOM.getFrameOwner", json!({ "frameId": frame_id.as_str() }))
        .await
        .ok()?;
    let offset = read_frame_content_offset(
        page_session,
        owner.backend_node_id,
        object_group,
        object_sessions,
    )
    .await?;
    Some(Rect {
        x: offset.x + rect.x,
        y: offset.y + rect.y,
        width: rect.width,
        height: rect.height,
    })
}

async fn read_element_rect(
    session: &ProtocolSession,
    backend_node_id: i64,
    object_group: &str,
    object_sessions: &mut Vec<ProtocolSession>,
) -> Option<Rect> {
    let object_id = resolve_object_id(session, backend_node_id, Some(object_group))
        .await
        .ok()?;
    object_sessions.push(session.clone());
    let result: RuntimeResult = session
        .send(
            "Runtime.callFunctionOn",
            json!({
                "functionDeclaration": "function(){var r=this.getBoundingClientRect();return{x:r.x,y:r.y,width:r.width,height:r.height}}",
                "objectId": object_id,
                "returnByValue": true
            }),
        )
        .await
        .ok()?;
    let rect = parse_rect(result.result.value?)?;
    (rect.width > 0.0 && rect.height > 0.0).then_some(rect)
}

async fn read_frame_content_offset(
    session: &ProtocolSession,
    backend_node_id: i64,
    object_group: &str,
    object_sessions: &mut Vec<ProtocolSession>,
) -> Option<PointOffset> {
    let object_id = resolve_object_id(session, backend_node_id, Some(object_group))
        .await
        .ok()?;
    object_sessions.push(session.clone());
    let result: RuntimeResult = session
        .send(
            "Runtime.callFunctionOn",
            json!({
                "functionDeclaration": "function(){var r=this.getBoundingClientRect();return{x:r.x+(this.clientLeft||0),y:r.y+(this.clientTop||0)}}",
                "objectId": object_id,
                "returnByValue": true
            }),
        )
        .await
        .ok()?;
    let value = result.result.value?;
    Some(PointOffset {
        x: value.get("x")?.as_f64()?,
        y: value.get("y")?.as_f64()?,
    })
}

#[derive(Debug, Clone, Copy)]
struct PointOffset {
    x: f64,
    y: f64,
}

async fn frame_depth(
    page_session: &ProtocolSession,
    frame_id: &FrameId,
) -> Result<Option<usize>, CoreError> {
    let tree: FrameTreeResult = page_session.send("Page.getFrameTree", json!({})).await?;
    Ok(find_frame_depth(&tree.frame_tree, frame_id, 0))
}

fn find_frame_depth(node: &FrameTreeNode, frame_id: &FrameId, depth: usize) -> Option<usize> {
    if node.frame.id == frame_id.as_str() {
        return Some(depth);
    }
    for child in node.child_frames.as_deref().unwrap_or(&[]) {
        if let Some(found) = find_frame_depth(child, frame_id, depth + 1) {
            return Some(found);
        }
    }
    None
}

async fn read_viewport_rect(session: &ProtocolSession) -> Result<Rect, CoreError> {
    let result: RuntimeResult = session
        .send(
            "Runtime.evaluate",
            json!({
                "expression": "({x:0,y:0,width:window.innerWidth||0,height:window.innerHeight||0})",
                "returnByValue": true,
                "awaitPromise": false
            }),
        )
        .await?;
    Ok(result.result.value.and_then(parse_rect).unwrap_or(Rect {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    }))
}

async fn read_scroll_offsets(session: &ProtocolSession) -> Result<PointOffset, CoreError> {
    let result: RuntimeResult = session
        .send(
            "Runtime.evaluate",
            json!({
                "expression": "({x: window.scrollX || 0, y: window.scrollY || 0})",
                "returnByValue": true,
                "awaitPromise": false
            }),
        )
        .await?;
    let value = result.result.value.unwrap_or(Value::Null);
    Ok(PointOffset {
        x: value.get("x").and_then(Value::as_f64).unwrap_or(0.0),
        y: value.get("y").and_then(Value::as_f64).unwrap_or(0.0),
    })
}

fn project_annotations(
    annotations: &[RawAnnotation],
    scroll: Option<PointOffset>,
    scale: Option<f64>,
) -> Vec<ScreenshotAnnotation> {
    let scale = scale.unwrap_or(1.0);
    annotations
        .iter()
        .map(|annotation| ScreenshotAnnotation {
            ref_id: annotation.ref_id.clone(),
            number: annotation.number,
            role: annotation.role.clone(),
            name: annotation.name.clone(),
            box_: ScreenshotAnnotationBox {
                x: ((annotation.rect.x + scroll.map(|scroll| scroll.x).unwrap_or(0.0)) * scale)
                    .round() as i64,
                y: ((annotation.rect.y + scroll.map(|scroll| scroll.y).unwrap_or(0.0)) * scale)
                    .round() as i64,
                width: (annotation.rect.width * scale).round() as i64,
                height: (annotation.rect.height * scale).round() as i64,
            },
        })
        .collect()
}

fn clip_annotations(
    annotations: Vec<RawAnnotation>,
    capture_area: Option<Rect>,
) -> Vec<RawAnnotation> {
    let Some(capture_area) = capture_area else {
        return annotations;
    };
    if capture_area.width <= 0.0 || capture_area.height <= 0.0 {
        return annotations;
    }
    annotations
        .into_iter()
        .filter_map(|annotation| {
            intersect_rects(annotation.rect, capture_area)
                .map(|rect| RawAnnotation { rect, ..annotation })
        })
        .collect()
}

fn parse_rect(value: Value) -> Option<Rect> {
    let rect: Rect = serde_json::from_value(value).ok()?;
    rect.x.is_finite().then_some(())?;
    rect.y.is_finite().then_some(())?;
    rect.width.is_finite().then_some(())?;
    rect.height.is_finite().then_some(())?;
    Some(rect)
}

fn intersect_rects(left: Rect, right: Rect) -> Option<Rect> {
    let x1 = left.x.max(right.x);
    let y1 = left.y.max(right.y);
    let x2 = (left.x + left.width).min(right.x + right.width);
    let y2 = (left.y + left.height).min(right.y + right.height);
    (x2 > x1 && y2 > y1).then_some(Rect {
        x: x1,
        y: y1,
        width: x2 - x1,
        height: y2 - y1,
    })
}

async fn inject_annotation_overlay(
    session: &ProtocolSession,
    token: &str,
    full_page: bool,
    annotations: &[RawAnnotation],
    scroll: Option<PointOffset>,
) -> Result<(), CoreError> {
    let items = annotations
        .iter()
        .map(|annotation| {
            json!({
                "number": annotation.number,
                "x": annotation.rect.x.round(),
                "y": annotation.rect.y.round(),
                "width": annotation.rect.width.round(),
                "height": annotation.rect.height.round()
            })
        })
        .collect::<Vec<_>>();
    let scroll_json = scroll
        .map(|scroll| json!({ "x": scroll.x, "y": scroll.y }))
        .unwrap_or(Value::Null);
    let expression = OVERLAY_SCRIPT
        .replace(
            "__BROWSEROS_ITEMS__",
            &serde_json::to_string(&items).map_err(|err| CoreError::Message(err.to_string()))?,
        )
        .replace(
            "__BROWSEROS_ATTR__",
            &serde_json::to_string(OVERLAY_ATTR)
                .map_err(|err| CoreError::Message(err.to_string()))?,
        )
        .replace(
            "__BROWSEROS_TOKEN__",
            &serde_json::to_string(token).map_err(|err| CoreError::Message(err.to_string()))?,
        )
        .replace(
            "__BROWSEROS_FULL_PAGE__",
            if full_page { "true" } else { "false" },
        )
        .replace("__BROWSEROS_SCROLL__", &scroll_json.to_string());
    let _: Value = session
        .send(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true, "awaitPromise": false }),
        )
        .await?;
    Ok(())
}

async fn remove_annotation_overlay(
    session: &ProtocolSession,
    token: &str,
) -> Result<(), CoreError> {
    let attr =
        serde_json::to_string(OVERLAY_ATTR).map_err(|err| CoreError::Message(err.to_string()))?;
    let token = serde_json::to_string(token).map_err(|err| CoreError::Message(err.to_string()))?;
    let expression = format!(
        "(() => {{ var attr = {attr}; var token = {token}; var existing = document.querySelectorAll('[' + attr + ']'); for (var i = 0; i < existing.length; i++) {{ if (existing[i].getAttribute(attr) === token) existing[i].remove(); }} return true; }})()"
    );
    let _: Value = session
        .send(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true, "awaitPromise": false }),
        )
        .await?;
    Ok(())
}

async fn release_object_group(sessions: &[ProtocolSession], object_group: &str) {
    for session in sessions {
        let _ = session
            .send::<_, Value>(
                "Runtime.releaseObjectGroup",
                json!({ "objectGroup": object_group }),
            )
            .await;
    }
}

async fn run_exclusive_screenshot_capture<F, Fut, T>(
    page_session: &ProtocolSession,
    task: F,
) -> Result<T, CoreError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, CoreError>>,
{
    // Plain captures share this protocol-session lock because annotated capture
    // temporarily overlays the DOM; overlap could include that overlay in a plain image.
    let mutex = screenshot_mutex(page_session);
    let _guard = mutex.lock().await;
    task().await
}

fn screenshot_mutex(page_session: &ProtocolSession) -> Arc<Mutex<()>> {
    static QUEUES: OnceLock<StdMutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let key = page_session
        .session_id()
        .map(ToString::to_string)
        .unwrap_or_else(|| "root".to_string());
    let queues = QUEUES.get_or_init(|| StdMutex::new(HashMap::new()));
    match queues.lock() {
        Ok(mut queues) => queues
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone(),
        Err(_poisoned) => Arc::new(Mutex::new(())),
    }
}

fn create_overlay_token() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let count = OVERLAY_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("browseros-{millis}-{count}")
}

fn annotation_number(ref_id: &str) -> i64 {
    ref_id
        .strip_prefix('e')
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        ScreenshotCaptureOptions, ScreenshotFormat, capture_plain_screenshot,
        capture_screencast_frame,
    };
    use crate::{ProtocolSession, test_support::TestConnection};
    use browseros_cdp::CdpEvent;
    use serde_json::json;

    #[tokio::test]
    async fn tests_that_plain_capture_restores_the_pointer_after_success_and_error()
    -> Result<(), Box<dyn std::error::Error>> {
        for succeeds in [true, false] {
            let mut replies = vec![
                ("Runtime.evaluate", json!({ "result": { "value": true } })),
                ("Runtime.evaluate", json!({ "result": { "value": true } })),
            ];
            if succeeds {
                replies.push(("Page.captureScreenshot", json!({ "data": "cG5n" })));
            }
            let connection = TestConnection::new(replies);
            let session = ProtocolSession::root(connection.clone());
            let result =
                capture_plain_screenshot(&session, &ScreenshotCaptureOptions::default()).await;
            assert_eq!(result.is_ok(), succeeds);
            if let Ok(capture) = result {
                assert_eq!(capture.data, "cG5n");
            }
            let calls = connection.calls()?;
            assert_eq!(calls.len(), 3);
            assert!(
                calls[0].params["expression"]
                    .as_str()
                    .is_some_and(|js| js.contains("p.hide()"))
            );
            assert_eq!(calls[1].method, "Page.captureScreenshot");
            assert!(
                calls[2].params["expression"]
                    .as_str()
                    .is_some_and(|js| js.contains("p.show()"))
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_plain_capture_survives_a_failed_pointer_hide()
    -> Result<(), Box<dyn std::error::Error>> {
        let connection =
            TestConnection::new([("Page.captureScreenshot", json!({ "data": "cG5n" }))]);
        let session = ProtocolSession::root(connection.clone());
        let capture =
            capture_plain_screenshot(&session, &ScreenshotCaptureOptions::default()).await?;
        assert_eq!(capture.data, "cG5n");
        let calls = connection.calls()?;
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].method, "Runtime.evaluate");
        assert_eq!(calls[1].method, "Page.captureScreenshot");
        Ok(())
    }

    #[tokio::test]
    async fn stalled_jpeg_can_use_a_session_scoped_screencast_frame()
    -> Result<(), Box<dyn std::error::Error>> {
        let connection = TestConnection::new([
            ("Page.startScreencast", json!({})),
            ("Page.stopScreencast", json!({})),
        ]);
        let session = ProtocolSession::root(connection.clone());
        let options = ScreenshotCaptureOptions {
            format: Some(ScreenshotFormat::Jpeg),
            quality: Some(45),
            ..Default::default()
        };
        let worker =
            tokio::spawn(async move { capture_screencast_frame(&session, &options).await });

        while !connection
            .calls()?
            .iter()
            .any(|call| call.method == "Page.startScreencast")
        {
            tokio::task::yield_now().await;
        }
        // 其他 CDP 会话的帧不能被当成当前标签页的截图。
        connection.publish_event(CdpEvent {
            method: "Page.screencastFrame".into(),
            params: json!({ "data": "wrong-tab" }),
            session_id: Some(browseros_cdp::SessionId::from("other-tab")),
        });
        connection.publish_event(CdpEvent {
            method: "Page.screencastFrame".into(),
            params: json!({ "data": "jpeg-frame" }),
            session_id: None,
        });
        let screenshot = worker.await??;
        assert_eq!(screenshot.data, "jpeg-frame");
        let calls = connection.calls()?;
        assert_eq!(calls[0].params["quality"], 45);
        assert_eq!(calls[0].params["maxFramesInFlight"], 1);
        assert_eq!(calls[1].method, "Page.stopScreencast");
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn missing_screencast_frame_still_stops_capture() -> Result<(), Box<dyn std::error::Error>>
    {
        let connection = TestConnection::new([
            ("Page.startScreencast", json!({})),
            ("Page.stopScreencast", json!({})),
        ]);
        let session = ProtocolSession::root(connection.clone());
        let worker = tokio::spawn(async move {
            capture_screencast_frame(&session, &ScreenshotCaptureOptions::default()).await
        });
        while !connection
            .calls()?
            .iter()
            .any(|call| call.method == "Page.startScreencast")
        {
            tokio::task::yield_now().await;
        }
        tokio::time::advance(super::SCREENCAST_FRAME_DEADLINE).await;
        assert!(worker.await?.is_err());
        let calls = connection.calls()?;
        assert_eq!(
            calls.last().map(|call| call.method.as_str()),
            Some("Page.stopScreencast")
        );
        Ok(())
    }
}
