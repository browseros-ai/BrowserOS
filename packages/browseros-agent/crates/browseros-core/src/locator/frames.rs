//! Frame chains are parsed by Playwright before crossing the host/CDP seam. Parsed
//! bodies are JSON (CSS ASTs and nested selector ASTs), so quoting/regex survives hops.

use super::{
    LocatorEngine,
    world::{object_id, release},
};
use crate::{CoreError, FrameId, PageId, ProtocolSession};
use serde_json::{Value, json};

pub(super) struct Scope {
    pub session: ProtocolSession,
    pub frame: FrameId,
    pub parsed: Value,
}

impl LocatorEngine {
    /// Translate a point from `r.session`'s viewport into the element's document
    /// viewport for `expectHitTarget`. Keep the original point for trusted input.
    /// This handles frame offsets, borders and scrolling; it is not a transform matrix
    /// for rotated/skewed/scaled frame elements.
    pub async fn hit_target_point(
        &self,
        r: &super::Resolved,
        point: crate::input::Point,
    ) -> Result<crate::input::Point, CoreError> {
        let model = r
            .session
            .send_value(
                "DOM.getBoxModel",
                json!({"backendNodeId": r.backend_node_id}),
            )
            .await?;
        let quad = model["model"]["border"]
            .as_array()
            .filter(|q| q.len() == 8)
            .ok_or_else(|| CoreError::from("Element has no border quad for hit testing"))?;
        let coords = quad
            .iter()
            .map(|v| {
                v.as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| CoreError::from("Invalid element border quad"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        // Both measurements describe this same border box, but CDP includes the
        // enclosing same-process frame offsets and DOMRect is document-local. Their
        // origins differ by the full offset, including nested frame borders/scroll.
        // Measure on the existing isolated handle: no parent DOM access (SOP), frame
        // ancestry cache, or main-world adoption is needed. OOPIFs keep their session.
        let rect = super::world::runtime_result(r.session.send_value("Runtime.callFunctionOn", json!({
            "objectId": r.object_id,
            "functionDeclaration": "function() { if (!this.isConnected) return null; const r = this.getBoundingClientRect(); return {x:r.x, y:r.y}; }",
            "returnByValue": true
        })).await?)?;
        let rect = &rect["value"];
        if rect.is_null() {
            return Err(CoreError::DocumentChanged);
        }
        let local_x = rect["x"]
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or_else(|| CoreError::from("Invalid local element bounds"))?;
        let local_y = rect["y"]
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or_else(|| CoreError::from("Invalid local element bounds"))?;
        let viewport_x = coords
            .iter()
            .step_by(2)
            .copied()
            .fold(f64::INFINITY, f64::min);
        let viewport_y = coords
            .iter()
            .skip(1)
            .step_by(2)
            .copied()
            .fold(f64::INFINITY, f64::min);
        Ok(crate::input::Point {
            x: point.x - viewport_x + local_x,
            y: point.y - viewport_y + local_y,
        })
    }

    pub(super) async fn scope(
        &self,
        page: PageId,
        selector: &str,
    ) -> Result<Option<Scope>, CoreError> {
        let mut session = self.pages.get_session(page.clone()).await?.session;
        let mut frame = root_frame(&session).await?;
        let parsed = self
            .worlds
            .call(
                &session,
                &frame,
                "function(selector) { return this.parseSelector(selector); }",
                vec![json!(selector)],
                true,
            )
            .await?;
        let chunks = split(parsed["value"].clone())?;
        let last = chunks.len().saturating_sub(1);
        for (index, parsed) in chunks.into_iter().enumerate() {
            if index == last {
                return Ok(Some(Scope {
                    session,
                    frame,
                    parsed,
                }));
            }
            // Frame locators are strict even when the final query returns an array.
            let element = self.worlds.call(&session, &frame, r#"function(parsed) {
                const el = this.querySelector(parsed, document, true);
                if (el && el.nodeName !== 'IFRAME' && el.nodeName !== 'FRAME')
                    throw this.createStacklessError('Selector resolved to ' + this.previewNode(el) + ', <iframe> was expected');
                return el;
            }"#, vec![parsed], false).await?;
            if element.get("objectId").is_none() {
                return Ok(None);
            }
            let object = object_id(&element)?;
            let described = session
                .send_value("DOM.describeNode", json!({"objectId": object, "depth": 0}))
                .await;
            release(&session, &object).await;
            let described = described?;
            let Some(child) = described["node"]["frameId"].as_str() else {
                return Ok(None);
            };
            frame = FrameId(child.to_owned());
            session = self
                .frames
                .resolve_frame_target(page.clone(), Some(frame.clone()), Some(&session))
                .await?
                .session;
        }
        Err(CoreError::from("Selector has no parts"))
    }
}

pub(super) async fn root_frame(session: &ProtocolSession) -> Result<FrameId, CoreError> {
    let tree = session.send_value("Page.getFrameTree", json!({})).await?;
    tree["frameTree"]["frame"]["id"]
        .as_str()
        .map(|id| FrameId(id.to_owned()))
        .ok_or_else(|| CoreError::from("Page has no root frame"))
}

/// Mirrors splitSelectorByFrame from the pinned upstream selectorParser, using the
/// already-parsed parts. Nested frame hops cannot be evaluated inside a DOM query.
fn split(parsed: Value) -> Result<Vec<Value>, CoreError> {
    let parts = parsed["parts"]
        .as_array()
        .ok_or_else(|| CoreError::from("Selector has no parsed parts"))?;
    let capture = parsed["capture"].as_u64().map(|index| index as usize);
    let mut chunks = Vec::new();
    let mut chunk = Vec::new();
    let mut chunk_capture = None;
    for (index, part) in parts.iter().enumerate() {
        validate_nested(part)?;
        if part["name"] == "internal:control" && part["body"] == "any-frame" {
            return Err(CoreError::from(
                "Unsupported frame control selector: any-frame",
            ));
        }
        if part["name"] == "internal:control" && part["body"] == "enter-frame" {
            if chunk.is_empty() {
                return Err(CoreError::from(
                    "Selector cannot start with entering frame, select the iframe first",
                ));
            }
            if chunk_capture.is_some() {
                return Err(CoreError::from(
                    "Can not capture the selector before diving into the frame. Only use * after the last frame has been selected",
                ));
            }
            chunks.push(json!({"parts": chunk}));
            chunk = Vec::new();
        } else {
            if capture == Some(index) {
                chunk_capture = Some(chunk.len());
            }
            chunk.push(part.clone());
        }
    }
    if chunk.is_empty() {
        return Err(CoreError::from("Selector cannot end with entering frame"));
    }
    let mut final_chunk = json!({"parts": chunk});
    if let Some(capture) = chunk_capture {
        final_chunk["capture"] = json!(capture);
    }
    chunks.push(final_chunk);
    Ok(chunks)
}

fn validate_nested(part: &Value) -> Result<(), CoreError> {
    if let Some(parts) = part["body"]["parsed"]["parts"].as_array() {
        for nested in parts {
            if nested["name"] == "internal:control"
                && (nested["body"] == "enter-frame" || nested["body"] == "any-frame")
            {
                return Err(CoreError::from(
                    "Frame locators are not allowed inside composite locators",
                ));
            }
            validate_nested(nested)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_preserves_capture_and_nested_ast() -> Result<(), CoreError> {
        let parsed = json!({"capture": 2, "parts": [
            {"name": "css", "source": "iframe", "body": []},
            {"name": "internal:control", "body": "enter-frame"},
            {"name": "internal:has", "body": {"parsed": {"parts": [{"name": "internal:text", "body": "\"x >> y\"i"}]}}}
        ]});
        let chunks = split(parsed.clone())?;
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[1]["capture"], 0);
        assert_eq!(chunks[1]["parts"][0], parsed["parts"][2]);
        let mut invalid = parsed;
        invalid["capture"] = json!(0);
        assert!(split(invalid).is_err());
        Ok(())
    }

    #[test]
    fn malformed_and_nested_frame_hops_are_rejected() {
        let enter = json!({"name": "internal:control", "body": "enter-frame"});
        assert!(split(json!({"parts": [enter]})).is_err());
        assert!(split(json!({"parts": [{"name": "css", "body": []}, enter]})).is_err());
        assert!(
            split(
                json!({"parts": [{"name": "internal:has", "body": {"parsed": {"parts": [enter]}}}]})
            )
            .is_err()
        );
    }
}
