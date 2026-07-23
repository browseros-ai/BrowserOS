use crate::{CoreError, ProtocolSession, input::Point};
use serde::Deserialize;
use serde_json::{Value, json};

const BOUNDS_JS: &str = include_str!("../assets/geometry.js");
const HIT_TEST_BLOCKER_JS: &str = r#"
function(x, y) {
    const target = this;
    const rootDocument = () => {
        let doc = target.ownerDocument || document;
        while (doc.defaultView && doc.defaultView.frameElement) {
            doc = doc.defaultView.frameElement.ownerDocument;
        }
        return doc;
    };
    const up = (node) => {
        if (!node) return null;
        return node.parentNode || node.host || (node.getRootNode && node.getRootNode().host) || null;
    };
    const reaches = (from, to) => {
        for (let node = from; node; node = up(node)) {
            if (node === to) return true;
        }
        return false;
    };
    const blockerAt = (doc, el, pointX, pointY) => {
        let currentDoc = doc;
        let localX = pointX;
        let localY = pointY;
        let hit = currentDoc.elementFromPoint(localX, localY);
        while (hit && (hit.tagName === 'IFRAME' || hit.tagName === 'FRAME') && hit !== el) {
            let childDoc = null;
            try {
                childDoc = hit.contentDocument;
            } catch (_err) {
                childDoc = null;
            }
            if (!childDoc) break;
            const rect = hit.getBoundingClientRect();
            localX -= rect.x + hit.clientLeft;
            localY -= rect.y + hit.clientTop;
            currentDoc = childDoc;
            hit = currentDoc.elementFromPoint(localX, localY);
        }
        if (!hit || hit === el || reaches(hit, el) || reaches(el, hit)) return null;
        const hitLabel = hit.closest ? hit.closest('label') : null;
        if (hitLabel && (hitLabel.control === el || hitLabel.contains(el))) return null;
        const elLabel = el.closest ? el.closest('label') : null;
        if (elLabel && elLabel.contains(hit)) return null;
        let desc = hit.tagName ? hit.tagName.toLowerCase() : 'element';
        if (hit.id) {
            desc += '#' + hit.id;
        } else if (typeof hit.className === 'string' && hit.className.trim()) {
            desc += '.' + hit.className.trim().split(/\s+/)[0];
        }
        return desc;
    };
    return blockerAt(rootDocument(), target, x, y);
}
"#;

#[derive(Debug, Deserialize)]
struct QuadsResult {
    quads: Option<Vec<Vec<f64>>>,
}

#[derive(Debug, Deserialize)]
struct BoxModelResult {
    model: Option<BoxModel>,
}

#[derive(Debug, Deserialize)]
struct BoxModel {
    content: Vec<f64>,
}

#[derive(Debug, Deserialize)]
struct ResolveNodeResult {
    object: Option<RemoteObject>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteObject {
    object_id: Option<String>,
    value: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct CallFunctionResult {
    result: RemoteObject,
}

#[derive(Debug, Deserialize)]
struct PushNodesResult {
    #[serde(rename = "nodeIds")]
    node_ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
struct Bounds {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// The point is in `session`'s coordinate basis. Dispatch input through the same
/// session so OOPIF coordinates remain valid without a manual transform.
pub async fn get_element_center(
    session: &ProtocolSession,
    backend_node_id: i64,
) -> Result<Point, CoreError> {
    let quads = session
        .send::<_, QuadsResult>(
            "DOM.getContentQuads",
            json!({ "backendNodeId": backend_node_id }),
        )
        .await;
    if let Ok(quads) = quads
        && let Some(quad) = quads.quads.and_then(|quads| quads.into_iter().next())
        && quad.len() >= 8
    {
        return Ok(quad_center(&quad));
    }

    let model = session
        .send::<_, BoxModelResult>(
            "DOM.getBoxModel",
            json!({ "backendNodeId": backend_node_id }),
        )
        .await;
    if let Ok(model) = model
        && let Some(model) = model.model
        && model.content.len() >= 8
    {
        return Ok(quad_center(&model.content));
    }

    let object_id = resolve_object_id(session, backend_node_id, None).await?;
    let bounds: CallFunctionResult = session
        .send(
            "Runtime.callFunctionOn",
            json!({
                "functionDeclaration": BOUNDS_JS,
                "objectId": object_id,
                "returnByValue": true
            }),
        )
        .await?;
    let value = bounds
        .result
        .value
        .ok_or_else(|| CoreError::Message("Could not get element bounds.".to_string()))?;
    let rect: Bounds =
        serde_json::from_value(value).map_err(|err| CoreError::Message(err.to_string()))?;
    Ok(Point {
        x: rect.x + rect.w / 2.0,
        y: rect.y + rect.h / 2.0,
    })
}

pub async fn scroll_into_view(session: &ProtocolSession, backend_node_id: i64) {
    let _ = session
        .send::<_, Value>(
            "DOM.scrollIntoViewIfNeeded",
            json!({ "backendNodeId": backend_node_id }),
        )
        .await;
}

pub async fn click_blocker_at_point(
    session: &ProtocolSession,
    backend_node_id: i64,
    point: Point,
) -> Result<Option<String>, CoreError> {
    let object_id = resolve_object_id(session, backend_node_id, None).await?;
    let result: CallFunctionResult = session
        .send(
            "Runtime.callFunctionOn",
            json!({
                "functionDeclaration": HIT_TEST_BLOCKER_JS,
                "objectId": object_id,
                "returnByValue": true,
                "arguments": [
                    { "value": point.x },
                    { "value": point.y }
                ]
            }),
        )
        .await?;
    Ok(result
        .result
        .value
        .and_then(|value| value.as_str().map(ToString::to_string)))
}

pub async fn focus_element(
    session: &ProtocolSession,
    backend_node_id: i64,
) -> Result<(), CoreError> {
    let pushed: PushNodesResult = session
        .send(
            "DOM.pushNodesByBackendIdsToFrontend",
            json!({ "backendNodeIds": [backend_node_id] }),
        )
        .await?;
    let Some(node_id) = pushed.node_ids.first() else {
        return Err(CoreError::Message(
            "Element not found in DOM. Take a new snapshot.".to_string(),
        ));
    };
    let _: Value = session
        .send("DOM.focus", json!({ "nodeId": node_id }))
        .await?;
    Ok(())
}

pub async fn js_click(session: &ProtocolSession, backend_node_id: i64) -> Result<(), CoreError> {
    let object_id = resolve_object_id(session, backend_node_id, None).await?;
    let _: Value = session
        .send(
            "Runtime.callFunctionOn",
            json!({ "functionDeclaration": "function(){this.click()}", "objectId": object_id }),
        )
        .await?;
    Ok(())
}

pub async fn get_input_value(session: &ProtocolSession, backend_node_id: i64) -> String {
    call_on_element(
        session,
        backend_node_id,
        "function(){return this.value??this.textContent??\"\"}",
        None,
    )
    .await
    .ok()
    .and_then(|value| value.as_str().map(ToString::to_string))
    .unwrap_or_default()
}

pub async fn call_on_element(
    session: &ProtocolSession,
    backend_node_id: i64,
    function_declaration: &str,
    args: Option<Vec<Value>>,
) -> Result<Value, CoreError> {
    let object_id = resolve_object_id(session, backend_node_id, None).await?;
    let result: CallFunctionResult = session
        .send(
            "Runtime.callFunctionOn",
            call_function_params(function_declaration, object_id, args),
        )
        .await?;
    Ok(result.result.value.unwrap_or(Value::Null))
}

fn call_function_params(
    function_declaration: &str,
    object_id: String,
    args: Option<Vec<Value>>,
) -> Value {
    let mut params = serde_json::Map::new();
    params.insert(
        "functionDeclaration".to_string(),
        Value::String(function_declaration.to_string()),
    );
    params.insert("objectId".to_string(), Value::String(object_id));
    params.insert("returnByValue".to_string(), Value::Bool(true));
    if let Some(args) = args {
        params.insert(
            "arguments".to_string(),
            Value::Array(
                args.into_iter()
                    .map(|value| json!({ "value": value }))
                    .collect(),
            ),
        );
    }
    Value::Object(params)
}

pub async fn resolve_object_id(
    session: &ProtocolSession,
    backend_node_id: i64,
    object_group: Option<&str>,
) -> Result<String, CoreError> {
    let mut params = serde_json::Map::new();
    params.insert("backendNodeId".to_string(), Value::from(backend_node_id));
    if let Some(object_group) = object_group {
        params.insert(
            "objectGroup".to_string(),
            Value::String(object_group.to_string()),
        );
    }
    let resolved: ResolveNodeResult = session
        .send("DOM.resolveNode", Value::Object(params))
        .await?;
    resolved
        .object
        .and_then(|object| object.object_id)
        .ok_or_else(|| {
            CoreError::Message("Element not found in DOM. Take a new snapshot.".to_string())
        })
}

fn quad_center(quad: &[f64]) -> Point {
    Point {
        x: (quad[0] + quad[2] + quad[4] + quad[6]) / 4.0,
        y: (quad[1] + quad[3] + quad[5] + quad[7]) / 4.0,
    }
}

#[cfg(test)]
mod tests {
    use super::{call_function_params, click_blocker_at_point};
    use crate::{CoreError, ProtocolSession, connection::CdpConnection, input::Point};
    use browseros_cdp::{CdpError, CdpEvent};
    use futures_util::future::BoxFuture;
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};
    use tokio::sync::broadcast;

    #[derive(Clone, Copy)]
    enum HitTestResponse {
        Blocked(&'static str),
        Clear,
        Error,
    }

    struct MockConnection {
        response: HitTestResponse,
        calls: Mutex<Vec<Value>>,
    }

    impl CdpConnection for MockConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            _session: Option<&'a crate::SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                match method {
                    "DOM.resolveNode" => Ok(json!({ "object": { "objectId": "target-object" } })),
                    "Runtime.callFunctionOn" => {
                        if let Ok(mut calls) = self.calls.lock() {
                            calls.push(params);
                        }
                        match self.response {
                            HitTestResponse::Blocked(blocker) => {
                                Ok(json!({ "result": { "value": blocker } }))
                            }
                            HitTestResponse::Clear => Ok(json!({ "result": { "value": null } })),
                            HitTestResponse::Error => Err(CdpError::Protocol {
                                code: -32000,
                                message: "execution context unavailable".to_string(),
                            }),
                        }
                    }
                    _ => Ok(json!({})),
                }
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a crate::SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async { Ok("{}".to_string()) })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            let (_tx, rx) = broadcast::channel(1);
            rx
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            1
        }
    }

    fn session_with(response: HitTestResponse) -> (Arc<MockConnection>, ProtocolSession) {
        let connection = Arc::new(MockConnection {
            response,
            calls: Mutex::new(Vec::new()),
        });
        (connection.clone(), ProtocolSession::root(connection))
    }

    #[test]
    fn call_function_omits_absent_arguments() {
        let params = call_function_params("function(){return this.checked}", "node-1".into(), None);

        assert_eq!(params.get("arguments"), None);
    }

    #[tokio::test]
    async fn click_blocker_at_point_returns_blocker_descriptor() -> Result<(), CoreError> {
        let (connection, session) = session_with(HitTestResponse::Blocked("div#consent-banner"));

        let blocker = click_blocker_at_point(&session, 10, Point { x: 50.0, y: 25.0 }).await?;

        assert_eq!(blocker, Some("div#consent-banner".to_string()));
        let calls = match connection.calls.lock() {
            Ok(calls) => calls.clone(),
            Err(_err) => Vec::new(),
        };
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls
                .first()
                .and_then(|call| call.get("arguments"))
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );
        Ok(())
    }

    #[tokio::test]
    async fn click_blocker_at_point_allows_descendant_target_hits() -> Result<(), CoreError> {
        let (_connection, session) = session_with(HitTestResponse::Clear);

        let blocker = click_blocker_at_point(&session, 10, Point { x: 50.0, y: 25.0 }).await?;

        assert_eq!(blocker, None);
        Ok(())
    }

    #[tokio::test]
    async fn click_blocker_script_walks_shadow_hosts() -> Result<(), CoreError> {
        let (connection, session) = session_with(HitTestResponse::Clear);

        let blocker = click_blocker_at_point(&session, 10, Point { x: 50.0, y: 25.0 }).await?;

        assert_eq!(blocker, None);
        let calls = match connection.calls.lock() {
            Ok(calls) => calls.clone(),
            Err(_err) => Vec::new(),
        };
        let function = calls
            .first()
            .and_then(|call| call.get("functionDeclaration"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(function.contains("getRootNode"));
        assert!(function.contains(".host"));
        Ok(())
    }

    #[tokio::test]
    async fn click_blocker_at_point_surfaces_hit_test_failures() {
        let (_connection, session) = session_with(HitTestResponse::Error);

        let result = click_blocker_at_point(&session, 10, Point { x: 50.0, y: 25.0 }).await;

        assert!(result.is_err());
    }
}
