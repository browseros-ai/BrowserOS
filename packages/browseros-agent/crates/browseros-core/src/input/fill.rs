//! Shared fill contract for MCP act and the run SDK. DOM helpers only inspect,
//! focus, and select; the browser's native editor changes the text and emits input
//! events. Success means the value was typed into the resolved field. What the field
//! holds afterwards is the page's call: input handlers may reformat it (a code field
//! grouping digits, a phone mask) or reject it. Fill does not judge that; act reports
//! the field as a snapshot shows it, and the caller decides.
//!
//! Date, time, colour and range inputs are the exception: they have no text editor, so fill
//! sets their value in the isolated world and fires input/change, as Playwright does.

use super::{
    Input, InputTarget, Point,
    geometry::{focus_element_js, get_element_center, get_input_value, scroll_into_view},
    keyboard::press_combo,
    mouse::{MouseButton, dispatch_click},
};
use crate::{CoreError, PageJs, ProtocolSession, Ref, pointer::PointerAction};
use serde_json::{Value, json};

const PREPARE_FILL_JS: &str = include_str!("../assets/prepare-fill.js");
const SET_INPUT_VALUE_JS: &str = include_str!("../assets/set-input-value.js");

/// Controls without a JS caret API need a native collapse step for append.
enum PreparedSelection {
    Ready,
    CollapseToEnd,
    /// A date, time, colour or range input, which takes a value rather than typed text.
    SetValue,
}

impl Input {
    /// Types `value` into the field, replacing its text, or appending when clear is
    /// explicitly false. Errors mean nothing reliable was typed: a stale ref, a field
    /// that cannot take text, or focus that went elsewhere.
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
        let kind = prepare_fill(session, target.backend_node_id, "validate").await?;
        scroll_into_view(session, target.backend_node_id).await;
        if matches!(kind, PreparedSelection::SetValue) {
            // No click: on a date or colour input it opens the browser's picker.
            return set_input_value(session, target.backend_node_id, value)
                .await
                .map(|()| None);
        }
        let mut point = get_element_center(session, target.backend_node_id).await?;
        self.check_click_point(session, &target, point).await?;
        if let Some(pointer) = self.drawing_pointer().await
            && pointer.move_to(session, PointerAction::Fill, point).await
        {
            point = get_element_center(session, target.backend_node_id).await?;
            self.check_click_point(session, &target, point).await?;
            pointer.click_effect(session, point).await;
        }
        // Native focus delivery can be deferred on background tabs until the
        // first input event. Click before checking focus so a page focus handler
        // cannot redirect the subsequent text insertion to another field.
        let at = self.dispatch_point(session, point).await;
        dispatch_click(session, at.x, at.y, MouseButton::Left, 1, 0).await?;
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
        // No readback and no retry: a controlled field may reformat or reject the
        // text on purpose, and typing it again would only repeat that.
        Ok(Some(point))
    }
}

async fn prepare_fill(
    session: &ProtocolSession,
    backend_node_id: i64,
    mode: &str,
) -> Result<PreparedSelection, CoreError> {
    let result: Value = PageJs::isolated(session)
        .call(backend_node_id, PREPARE_FILL_JS, &[json!(mode)])
        .await?;
    if result == Value::Bool(true) {
        Ok(PreparedSelection::Ready)
    } else if result.as_str() == Some("collapseToEnd") {
        Ok(PreparedSelection::CollapseToEnd)
    } else if result.as_str() == Some("setValue") {
        Ok(PreparedSelection::SetValue)
    } else {
        Err(CoreError::Message(
            result
                .as_str()
                .unwrap_or("Could not prepare the field for input.")
                .to_string(),
        ))
    }
}

async fn set_input_value(
    session: &ProtocolSession,
    backend_node_id: i64,
    value: &str,
) -> Result<(), CoreError> {
    let result: Value = PageJs::isolated(session)
        .call(backend_node_id, SET_INPUT_VALUE_JS, &[json!(value)])
        .await?;
    if result == Value::Bool(true) {
        Ok(())
    } else {
        Err(CoreError::Message(
            result
                .as_str()
                .unwrap_or("Could not set the field's value.")
                .to_string(),
        ))
    }
}
