//! The stateless session handle: how an agent on MCP 2026-07-28, which has no transport
//! session, keeps one browser session across calls.
//!
//! `service` mints the handle and the agent echoes it as the `session` argument. An agent can
//! only echo a handle its model was shown, and MCP clients show the model different parts of a
//! result: never `_meta`, the `content` text for most tools, and only `structuredContent` for a
//! tool that declares an output schema (Claude Code). The handle therefore rides in each of
//! those places and is declared in both schemas a client reads. Every placement lives here
//! because missing one is silent: the model never sees the handle, sends none back, and each
//! call starts another empty session in the cockpit.
//!
//! ```text
//! tools/list  offer(tool)     input `session` argument, output `session` field
//! tools/call  take(args)      the presented handle, stripped before the tool runs
//!             attach(result)  _meta, a content line, structuredContent.session
//! ```

use crate::ids::SessionId;
use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, ContentBlock, JsonObject, MetaObject, Tool},
};
use serde_json::{Value, json};
use std::sync::Arc;

/// The argument the agent echoes, and the structured field it reads the handle from.
const FIELD: &str = "session";
/// `_meta` key the handle is returned under, namespaced by owner per the MCP `_meta`
/// convention. For clients that read it programmatically; models never see `_meta`.
const META_KEY: &str = "com.browseros.neo/session";
// Scoped to the path that actually emits it. The handle rides only on connections
// without a transport session id; the other path withholds it and ignores one that
// is presented, so promising it to every client pointed those agents at a control
// they do not have and away from the tab group id, which is the signal that works
// for them. Naming the alternative here matters because this text is read at the
// moment an agent is deciding how to keep continuity.
pub const SESSION_ARG_DESCRIPTION: &str = "Opaque logical task handle, including its browser_target binding. You receive one only if your client connects without its own transport session id: the server returns it in every tool result's `_meta` under `com.browseros.neo/session`, in result text, and as `session` in structured results. Pass it as `session` on later calls; natural successors retain the task's latest target even across server restart. If you never see one, your continuity across a remade connection is your tab group id instead: pass a known groupId on tabs action=\"new\" to recover its task binding before creation. Existing tabs keep their actual ownership. Omit session on the first call or to start a separate task after Stop; a stopped handle stays stopped. Stale browser targets require explicit browser_target reselection or clear.";
/// The sentence of [`SESSION_ARG_DESCRIPTION`] that promises the server's own tab group.
/// `tab_grouping` rewrites it under explicit grouping.
pub(super) const SESSION_GROUP_CONTINUITY: &str = "If you never see one, your continuity across a remade connection is your tab group id instead: pass a known groupId on tabs action=\"new\" to recover its task binding before creation. Existing tabs keep their actual ownership. ";
const OUTPUT_DESCRIPTION: &str =
    "This browser session's handle. Pass it as the `session` argument on every following call.";

/// `tool` as every client is offered it: an optional `session` argument, and a `session`
/// field in its output schema when it declares one, so a client that validates structured
/// results against that schema accepts the handle [`attach`] puts there.
pub(super) fn offer(mut tool: Tool) -> Tool {
    tool.input_schema = Arc::new(with_property(
        tool.input_schema.as_ref().clone(),
        SESSION_ARG_DESCRIPTION,
    ));
    // Root-level even for the `oneOf` target schemas, which apply it to every branch.
    tool.output_schema = tool
        .output_schema
        .map(|output| Arc::new(with_property(output.as_ref().clone(), OUTPUT_DESCRIPTION)));
    tool
}

fn with_property(mut schema: JsonObject, description: &str) -> JsonObject {
    let properties = schema
        .entry("properties")
        .or_insert_with(|| Value::Object(JsonObject::new()));
    if let Value::Object(properties) = properties {
        properties.insert(
            FIELD.to_string(),
            json!({ "type": "string", "description": description }),
        );
    }
    schema
}

/// The handle an agent presented, removed from `args` on every path, including legacy, so no
/// tool ever receives it even from an agent sending it against a schema that never offered it.
pub(super) fn take(args: &mut Value) -> Option<SessionId> {
    args.as_object_mut()?
        .remove(FIELD)?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(SessionId::new)
}

/// `result` with the session's `handle` everywhere a client might show its model. `None` is a
/// legacy call, which has a transport session and gets the result back untouched.
pub(super) fn attach(
    result: Result<CallToolResult, McpError>,
    handle: Option<SessionId>,
    presented: Option<SessionId>,
) -> Result<CallToolResult, McpError> {
    let Some(handle) = handle else {
        return result;
    };
    // An agent that presented a handle and got a different one back has changed session
    // without asking to. Saying so is the difference between an agent that adopts the new
    // handle and one that keeps resending a dead one, which is what fills the cockpit with
    // one-call sessions.
    let replaced = presented.is_some_and(|presented| presented != handle);
    let handle = handle.to_string();
    result.map(|mut call_result| {
        call_result
            .meta
            .get_or_insert_with(MetaObject::new)
            .insert(META_KEY.to_string(), Value::String(handle.clone()));
        if let Some(Value::Object(structured)) = call_result.structured_content.as_mut() {
            structured.insert(FIELD.to_string(), Value::String(handle.clone()));
        }
        let line = if replaced {
            format!(
                "[browseros-neo session: {handle}. The handle you sent is no longer active, so this call continued in a new session. Pass this exact value as the `session` argument from now on, and stop sending the previous one.]"
            )
        } else {
            format!(
                "[browseros-neo session: {handle}. Pass this exact value as the `session` argument on every following call to keep this browser session and its tab ownership.]"
            )
        };
        call_result.content.push(ContentBlock::text(line));
        call_result
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(output_schema: Option<Value>) -> Tool {
        let input = json!({ "type": "object", "properties": { "code": { "type": "string" } } });
        let mut tool = Tool::new(
            "probe",
            "probe",
            Arc::new(input.as_object().cloned().unwrap_or_default()),
        );
        tool.output_schema = output_schema
            .and_then(|schema| schema.as_object().cloned())
            .map(Arc::new);
        tool
    }

    /// What a client shows its model from a result. Claude Code shows only the structured
    /// result for a tool that declares an output schema, and the content text otherwise.
    fn model_view(tool: &Tool, result: &CallToolResult) -> String {
        if tool.output_schema.is_some() {
            return result
                .structured_content
                .as_ref()
                .map(Value::to_string)
                .unwrap_or_default();
        }
        text_of(result)
    }

    fn text_of(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|block| block.as_text().map(|text| text.text.as_str()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn attached(
        result: CallToolResult,
        handle: &str,
        presented: Option<&str>,
    ) -> anyhow::Result<CallToolResult> {
        attach(
            Ok(result),
            Some(SessionId::new(handle)),
            presented.map(SessionId::new),
        )
        .map_err(|error| anyhow::anyhow!("{error:?}"))
    }

    #[test]
    fn the_model_sees_the_handle_whatever_part_of_the_result_its_client_shows() -> anyhow::Result<()>
    {
        let structured = json!({ "type": "object", "properties": { "ok": { "type": "boolean" } } });
        // The browser_target and browser_profiles shape: branches, no root properties.
        let branches = json!({ "type": "object", "oneOf": [{ "properties": { "ok": {} } }] });
        for output_schema in [None, Some(structured), Some(branches)] {
            let offered = offer(tool(output_schema));
            let result = if offered.output_schema.is_some() {
                CallToolResult::structured(json!({ "ok": true }))
            } else {
                CallToolResult::success(vec![ContentBlock::text("ok")])
            };
            let result = attached(result, "handle-xyz", None)?;
            let view = model_view(&offered, &result);
            assert!(view.contains("handle-xyz"), "{offered:?} showed {view}");
            assert_eq!(
                result.meta.as_ref().and_then(|meta| meta.get(META_KEY)),
                Some(&json!("handle-xyz"))
            );
            // A client validating the structured result against the schema it was offered
            // must accept the field the handle rides in.
            if let Some(output) = &offered.output_schema {
                assert_eq!(output["properties"][FIELD]["type"], "string", "{output:?}");
            }
        }
        Ok(())
    }

    #[test]
    fn a_structured_result_keeps_its_own_fields() -> anyhow::Result<()> {
        let result = attached(
            CallToolResult::structured(json!({ "ok": true, "value": 1 })),
            "handle-xyz",
            None,
        )?;
        assert_eq!(
            result.structured_content,
            Some(json!({ "ok": true, "value": 1, "session": "handle-xyz" }))
        );
        let plain = attached(
            CallToolResult::success(vec![ContentBlock::text("ok")]),
            "handle-xyz",
            None,
        )?;
        assert!(plain.structured_content.is_none());
        Ok(())
    }

    #[test]
    fn offer_adds_an_optional_session_argument_that_promises_the_handle_conditionally() {
        let offered = offer(tool(None));
        let schema = Value::Object(offered.input_schema.as_ref().clone());
        assert_eq!(
            schema["properties"][FIELD],
            json!({ "type": "string", "description": SESSION_ARG_DESCRIPTION })
        );
        assert_eq!(schema["properties"]["code"]["type"], "string");
        assert!(schema.get("required").is_none());
        assert!(offered.output_schema.is_none());
        // The promise has to stay conditional and has to keep naming the fallback:
        // an unconditional version of this text is what sent a legacy client after
        // a handle it never receives.
        assert!(
            SESSION_ARG_DESCRIPTION.contains("only if your client connects without"),
            "the handle must not be promised unconditionally"
        );
        assert!(
            SESSION_ARG_DESCRIPTION.contains("your tab group id instead"),
            "a client that gets no handle must be told what to use"
        );
        assert!(
            !SESSION_ARG_DESCRIPTION.contains("and its tab ownership"),
            "tab ownership continuity comes from the group id, not this handle"
        );
        assert!(SESSION_ARG_DESCRIPTION.contains(SESSION_GROUP_CONTINUITY));
    }

    #[test]
    fn take_returns_the_presented_handle_and_strips_it() {
        let mut args = json!({ "code": "return 1", "session": " handle-xyz " });
        assert_eq!(take(&mut args), Some(SessionId::new("handle-xyz")));
        assert_eq!(args, json!({ "code": "return 1" }));

        for blank in [json!(""), json!("  "), json!(7)] {
            let mut args = json!({ "session": blank });
            assert_eq!(take(&mut args), None);
            assert_eq!(args, json!({}));
        }
        assert_eq!(take(&mut json!({ "code": "return 1" })), None);
    }

    /// The agent has to be able to see that its session changed. `_meta` is not surfaced to
    /// the model by MCP clients, so the difference has to be in the content.
    #[test]
    fn a_replaced_handle_says_so_in_the_content() -> anyhow::Result<()> {
        let ok = || CallToolResult::success(vec![ContentBlock::text("ok")]);
        let text = text_of(&attached(ok(), "successor", Some("dead-handle"))?);
        assert!(text.contains("no longer active"), "{text}");
        assert!(text.contains("stop sending the previous one"), "{text}");

        // An agent that presented the handle it got back is not told anything unusual.
        let text = text_of(&attached(ok(), "same", Some("same"))?);
        assert!(!text.contains("no longer active"), "{text}");
        Ok(())
    }

    #[test]
    fn a_legacy_call_is_returned_untouched() -> anyhow::Result<()> {
        let result = attach(
            Ok(CallToolResult::structured(json!({ "ok": true }))),
            None,
            None,
        )
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_eq!(result.structured_content, Some(json!({ "ok": true })));
        assert!(result.meta.is_none());
        assert_eq!(result.content.len(), 1);
        Ok(())
    }
}
