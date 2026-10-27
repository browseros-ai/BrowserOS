use crate::{CoreError, PageJs, ProtocolSession, input::Point};
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

    let rect: Bounds = PageJs::isolated(session)
        .call::<Option<Bounds>>(backend_node_id, BOUNDS_JS, &[])
        .await?
        .ok_or_else(|| CoreError::Message("Could not get element bounds.".to_string()))?;
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
    PageJs::isolated(session)
        .call(
            backend_node_id,
            HIT_TEST_BLOCKER_JS,
            &[json!(point.x), json!(point.y)],
        )
        .await
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

// `DOM.focus` needs `DOM.getDocument` to have run for the session first, which
// fresh MCP sessions have not done, so it fails with "Document needs to be
// requested first" (#2640). `HTMLElement.focus()` over `Runtime.callFunctionOn`
// has no such dependency, the same runtime path `js_click` already uses.
//
// Success is confirmed against the resulting active element, not merely that a
// `focus` method exists: every HTMLElement exposes `focus()`, but disabled or
// otherwise unfocusable targets stay unfocused, and reporting those as success
// would let `act focus` claim it moved focus when it did not. `getRootNode()`
// resolves the active element inside the element's own root so shadow-DOM
// targets are not mistaken for failures.
pub async fn focus_element_js(
    session: &ProtocolSession,
    backend_node_id: i64,
) -> Result<(), CoreError> {
    let focused: Option<bool> = PageJs::isolated(session)
        .call(
            backend_node_id,
            "function(){if(typeof this.focus!=='function'){return false}this.focus();return this.getRootNode().activeElement===this}",
            &[],
        )
        .await?;
    if focused.unwrap_or(false) {
        Ok(())
    } else {
        Err(CoreError::Message(
            "Element cannot receive focus.".to_string(),
        ))
    }
}

pub async fn js_click(session: &ProtocolSession, backend_node_id: i64) -> Result<(), CoreError> {
    let _: Value = PageJs::isolated(session)
        .call(backend_node_id, "function(){this.click()}", &[])
        .await?;
    Ok(())
}

/// Read the connected field, preserving the distinction between empty and unreadable.
pub async fn get_input_value(
    session: &ProtocolSession,
    backend_node_id: i64,
) -> Result<String, CoreError> {
    let value: Option<String> = PageJs::isolated(session)
        .call(backend_node_id, READ_INPUT_VALUE_JS, &[])
        .await?;
    value.ok_or_else(|| {
        CoreError::Message("Could not read the field value. Take a new snapshot.".to_string())
    })
}

// Blink keeps a lone <br> as the caret placeholder in an emptied editing host.
// Its innerText is a newline, but it represents no user text. Do not trim actual
// whitespace or multiple line breaks from a nonempty editor.
const READ_INPUT_VALUE_JS: &str = r#"
function() {
    if (!this.isConnected) return null;
    if (typeof this.value === 'string') return this.value;
    if (!this.isContentEditable) return null;
    if (this.childNodes.length === 1 && this.firstChild.nodeName === 'BR') return '';
    return this.innerText;
}
"#;

fn quad_center(quad: &[f64]) -> Point {
    Point {
        x: (quad[0] + quad[2] + quad[4] + quad[6]) / 4.0,
        y: (quad[1] + quad[3] + quad[5] + quad[7]) / 4.0,
    }
}

#[cfg(test)]
mod tests {
    use super::{click_blocker_at_point, focus_element_js};
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
        Bool(bool),
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
                            HitTestResponse::Bool(value) => {
                                Ok(json!({ "result": { "value": value } }))
                            }
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

    #[tokio::test]
    async fn focus_element_js_focuses_through_runtime() -> Result<(), CoreError> {
        let (connection, session) = session_with(HitTestResponse::Bool(true));

        focus_element_js(&session, 10).await?;

        let calls = match connection.calls.lock() {
            Ok(calls) => calls.clone(),
            Err(_err) => Vec::new(),
        };
        // Focus must go through Runtime.callFunctionOn (this.focus()), never the
        // DOM domain that needs a prior DOM.getDocument (#2640), and it must
        // confirm the element actually became active rather than trusting that a
        // focus method exists.
        assert_eq!(calls.len(), 1);
        let function = calls
            .first()
            .and_then(|call| call.get("functionDeclaration"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(function.contains("this.focus()"));
        assert!(function.contains("activeElement===this"));
        Ok(())
    }

    #[tokio::test]
    async fn focus_element_js_reports_unfocusable_element() {
        let (_connection, session) = session_with(HitTestResponse::Bool(false));

        let result = focus_element_js(&session, 10).await;

        assert!(result.is_err());
    }
}
