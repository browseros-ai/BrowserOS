//! Shared text fill contract for MCP act, run and locator actions. The native
//! editor changes text; locator fills also support controls without a text caret
//! through value assignment. Both paths verify the value after input handlers.

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

// These controls have no text selection to replace with Input.insertText.
// Text fields return to the shared native editor path so beforeinput cancellation
// and framework input handlers keep the same semantics as ref-based fills.
const FILL_CONTROL_JS: &str = r#"
function(value) {
    const select = this.tagName === 'SELECT';
    const direct = this.tagName === 'INPUT' &&
        ['color', 'date', 'time', 'datetime-local', 'month', 'range', 'week'].includes(this.type);
    if (!select && !direct) return { handled: false };
    if (!this.isConnected) return { error: 'Element is detached.' };
    if (this.matches(':disabled') || this.readOnly)
        return { error: 'Element is disabled or read-only.' };
    const style = this.ownerDocument.defaultView.getComputedStyle(this);
    if (!this.getClientRects().length || ['hidden', 'collapse'].includes(style.visibility))
        return { error: 'Element is not visible.' };
    if (!select) value = value.trim();
    if (select && !Array.from(this.options).some(option => option.value === value))
        return { error: 'Select has no option with the requested value.' };
    this.focus();
    if (!this.isConnected || this.getRootNode().activeElement !== this)
        return { error: 'Element did not receive focus.' };
    this.value = value;
    if (this.value !== value) return { error: 'Malformed value for this control.' };
    this.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
    this.dispatchEvent(new Event('change', { bubbles: true }));
    return { handled: true, expected: value };
}
"#;

/// Controls without a JS caret API need a native collapse step for append.
enum PreparedSelection {
    Ready,
    CollapseToEnd,
}

impl Input {
    pub async fn fill_backend_node(
        &self,
        session: &ProtocolSession,
        backend_node_id: i64,
        value: &str,
    ) -> Result<(), CoreError> {
        let control = call_on_element(
            session,
            backend_node_id,
            FILL_CONTROL_JS,
            Some(vec![json!(value)]),
        )
        .await?;
        match control.get("handled").and_then(Value::as_bool) {
            Some(false) => self
                .fill_node(
                    session,
                    InputTarget::from_backend_node(backend_node_id),
                    value,
                    true,
                )
                .await
                .map(|_point| ()),
            Some(true) => {
                let expected =
                    control
                        .get("expected")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            CoreError::Message(
                                "Could not determine the expected control value.".to_string(),
                            )
                        })?;
                verify_fill(session, backend_node_id, expected).await
            }
            None => Err(CoreError::Message(
                control
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("Could not fill the control.")
                    .to_string(),
            )),
        }
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
        verify_fill(session, target.backend_node_id, &expected).await?;
        Ok(Some(point))
    }
}

async fn verify_fill(
    session: &ProtocolSession,
    backend_node_id: i64,
    expected: &str,
) -> Result<(), CoreError> {
    if get_input_value(session, backend_node_id).await? != expected {
        return Err(CoreError::Message(
            "The field did not retain the requested value. It may reject or normalize input."
                .to_string(),
        ));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::backend_node_tests::{BackendHarness, click_responses, element_result};

    fn text_fill_responses(before: &str, value: &str, after: Value) -> Vec<(&'static str, Value)> {
        let mut responses = element_result(json!({ "handled": false }));
        responses.extend(element_result(json!(true)));
        responses.extend(click_responses(Value::Null));
        responses.extend(element_result(json!(true))); // focus
        responses.extend(element_result(json!(true))); // select all
        responses.extend(element_result(json!(before)));
        responses.extend(element_result(json!(true))); // confirm focus
        if value.is_empty() {
            if !before.is_empty() {
                responses.extend((0..2).map(|_| ("Input.dispatchKeyEvent", json!({}))));
            }
        } else {
            responses.push(("Input.insertText", json!({})));
        }
        responses.extend(element_result(after));
        responses
    }

    #[tokio::test]
    async fn backend_fill_replaces_using_native_input_then_reads_back() -> Result<(), CoreError> {
        let value = "replacement\nwith \"quotes\"";
        let h = BackendHarness::new(text_fill_responses("old", value, json!(value))).await?;
        h.input.fill_backend_node(&h.session, 42, value).await?;
        let calls = h.assert_sequence()?;
        let modes = calls
            .iter()
            .filter(|call| {
                call.method == "Runtime.callFunctionOn"
                    && call.params["functionDeclaration"] == PREPARE_FILL_JS
            })
            .map(|call| call.params["arguments"][0]["value"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(modes, [Some("validate"), Some("replace"), Some("focused")]);
        let insert = calls.iter().find(|call| call.method == "Input.insertText");
        assert_eq!(
            insert.map(|call| &call.params),
            Some(&json!({ "text": value }))
        );
        Ok(())
    }

    #[tokio::test]
    async fn backend_fill_empty_deletes_selection_and_empty_field_is_noop() -> Result<(), CoreError>
    {
        for before in ["old", ""] {
            let h = BackendHarness::new(text_fill_responses(before, "", json!(""))).await?;
            h.input.fill_backend_node(&h.session, 42, "").await?;
            let calls = h.assert_sequence()?;
            let keys = calls
                .iter()
                .filter(|call| call.method == "Input.dispatchKeyEvent")
                .collect::<Vec<_>>();
            assert_eq!(keys.len(), if before.is_empty() { 0 } else { 2 });
            for call in keys {
                assert_eq!(call.params["key"], "Backspace");
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn backend_fill_rejects_failed_readback_without_retrying_or_leaking_values()
    -> Result<(), CoreError> {
        for after in [json!("normalized-secret"), Value::Null] {
            let h =
                BackendHarness::new(text_fill_responses("old-secret", "new-secret", after)).await?;
            let result = h
                .input
                .fill_backend_node(&h.session, 42, "new-secret")
                .await;
            assert!(result.is_err());
            let message = result.err().map(|err| err.to_string()).unwrap_or_default();
            assert!(!message.contains("secret"));
            h.assert_sequence()?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn backend_fill_rejects_invalid_or_covered_text_field_before_editing()
    -> Result<(), CoreError> {
        let mut responses = element_result(json!({ "handled": false }));
        responses.extend(element_result(json!("Element is disabled or read-only.")));
        let h = BackendHarness::new(responses).await?;
        assert!(
            h.input
                .fill_backend_node(&h.session, 42, "text")
                .await
                .is_err()
        );
        h.assert_sequence()?;

        let mut responses = element_result(json!({ "handled": false }));
        responses.extend(element_result(json!(true)));
        responses.extend(click_responses(json!("div#overlay")));
        let h = BackendHarness::new(responses).await?;
        assert!(
            matches!(h.input.fill_backend_node(&h.session, 42, "text").await,
            Err(CoreError::ElementCovered { target, blocker })
                if target.backend_node_id == Some(42) && blocker == "div#overlay")
        );
        h.assert_sequence()?;
        Ok(())
    }

    #[tokio::test]
    async fn backend_fill_control_verifies_after_dispatching_events() -> Result<(), CoreError> {
        for value in ["2026-09-23", "option-value", ""] {
            let mut responses = element_result(json!({ "handled": true, "expected": value }));
            responses.extend(element_result(json!(value)));
            let h = BackendHarness::new(responses).await?;
            h.input.fill_backend_node(&h.session, 42, value).await?;
            let calls = h.assert_sequence()?;
            assert_eq!(calls[1].params["functionDeclaration"], FILL_CONTROL_JS);
            assert_eq!(calls[1].params["arguments"], json!([{ "value": value }]));
        }
        Ok(())
    }

    #[tokio::test]
    async fn backend_fill_control_surfaces_preparation_and_retention_errors()
    -> Result<(), CoreError> {
        let h = BackendHarness::new(element_result(
            json!({ "error": "Malformed value for this control." }),
        ))
        .await?;
        assert!(
            h.input
                .fill_backend_node(&h.session, 42, "invalid")
                .await
                .is_err()
        );
        h.assert_sequence()?;

        let mut responses = element_result(json!({ "handled": true, "expected": "2026-09-23" }));
        responses.extend(element_result(json!("2026-09-24")));
        let h = BackendHarness::new(responses).await?;
        assert!(
            h.input
                .fill_backend_node(&h.session, 42, "2026-09-23")
                .await
                .is_err()
        );
        h.assert_sequence()?;
        Ok(())
    }
}
