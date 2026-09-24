//! Shared fill contract for MCP act and the run SDK. DOM helpers only inspect,
//! focus, and select; the browser's native editor changes the text and emits input
//! events. Success means the resolved field contains the expected value.

use super::{
    Input, InputTarget, Point,
    geometry::{
        call_on_element, focus_element_js, get_element_center, get_input_value, scroll_into_view,
    },
    keyboard::press_combo,
    mouse::{MouseButton, dispatch_click},
};
use crate::{CoreError, ProtocolSession, Ref};
use serde_json::{Value, json};

const PREPARE_FILL_JS: &str = include_str!("../assets/prepare-fill.js");

/// Controls without a JS caret API need a native collapse step for append.
enum PreparedSelection {
    Ready,
    CollapseToEnd,
}

impl Input {
    pub async fn fill_backend_node(
        &self,
        _session: &ProtocolSession,
        _backend_node_id: i64,
        _value: &str,
    ) -> Result<(), CoreError> {
        Err(CoreError::Message(
            "not implemented yet: fill_backend_node".to_string(),
        ))
    }

    /// Replaces the field's text, or appends when clear is explicitly false.
    /// Application outcomes (such as search readiness) are separate from this
    /// value guarantee. Neither failed readback nor a detached ref means empty.
    pub async fn fill(
        &self,
        ref_id: &Ref,
        value: &str,
        clear: bool,
    ) -> Result<Option<Point>, CoreError> {
        let resolved = self.observer.resolve_ref(ref_id).await?;
        self.fill_node(
            &resolved.session,
            InputTarget::from_ref_entry(&resolved.entry),
            value,
            clear,
        )
        .await
        .map_err(|err| CoreError::Message(format!("Could not fill {ref_id}: {err}")))
    }

    async fn fill_node(
        &self,
        session: &ProtocolSession,
        target: InputTarget,
        value: &str,
        clear: bool,
    ) -> Result<Option<Point>, CoreError> {
        // Validate before focus/selection so rejecting a field cannot send input
        // to whatever happens to be focused elsewhere on the page.
        prepare_fill(session, target.backend_node_id, "validate").await?;
        scroll_into_view(session, target.backend_node_id).await;
        let point = get_element_center(session, target.backend_node_id).await?;
        self.check_click_point(session, &target, point).await?;
        // Native focus delivery can be deferred on background tabs until the
        // first input event. Click before checking focus so a page focus handler
        // cannot redirect the subsequent text insertion to another field.
        dispatch_click(session, point.x, point.y, MouseButton::Left, 1, 0).await?;
        focus_element_js(session, target.backend_node_id).await?;
        let selection = prepare_fill(
            session,
            target.backend_node_id,
            if clear { "replace" } else { "append" },
        )
        .await?;
        let key_session = self.page_session().await?;
        if matches!(selection, PreparedSelection::CollapseToEnd) {
            press_combo(&key_session, "ArrowRight").await?;
        }
        let before = get_input_value(session, target.backend_node_id).await?;
        let expected = if clear {
            value.to_string()
        } else {
            format!("{before}{value}")
        };

        // Keyboard/editor events route through the top-level page; DOM
        // inspection and selection stay in the ref's frame (including OOPIFs).
        // A single native insertion avoids character-by-character Enter submits
        // and focus handoffs midway through a fill. Use type/press for key events.
        // Selection events can run after the select call returns. Confirm focus
        // again after any native caret movement and before sending field text.
        prepare_fill(session, target.backend_node_id, "focused").await?;
        if value.is_empty() {
            if clear && !before.is_empty() {
                press_combo(&key_session, "Backspace").await?;
            }
        } else {
            let _: Value = key_session
                .send("Input.insertText", json!({ "text": value }))
                .await?;
        }

        // Input handlers and their microtasks have run by this readback. Do not
        // retry an edit: a controlled field may intentionally reject/normalize it.
        // Already-correct text is an idempotent success, even if a redundant
        // edit is canceled; fill promises the value, not application readiness.
        // Keep values out of errors, since either side may contain a password.
        if get_input_value(session, target.backend_node_id).await? != expected {
            return Err(CoreError::Message(
                "The field did not retain the requested value. It may reject or normalize input."
                    .to_string(),
            ));
        }
        Ok(Some(point))
    }
}

async fn prepare_fill(
    session: &ProtocolSession,
    backend_node_id: i64,
    mode: &str,
) -> Result<PreparedSelection, CoreError> {
    let result = call_on_element(
        session,
        backend_node_id,
        PREPARE_FILL_JS,
        Some(vec![json!(mode)]),
    )
    .await?;
    if result == Value::Bool(true) {
        Ok(PreparedSelection::Ready)
    } else if result.as_str() == Some("collapseToEnd") {
        Ok(PreparedSelection::CollapseToEnd)
    } else {
        Err(CoreError::Message(
            result
                .as_str()
                .unwrap_or("Could not prepare the field for input.")
                .to_string(),
        ))
    }
}
