use rmcp::model::{CallToolResult, Tool, ToolAnnotations};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{
    dispatch::{browser_unavailable_message, linked_cancel_token},
    observers::audit::{LocalToolDispatch, record_local_tool_dispatch},
};
use crate::{
    AppState,
    ids::DispatchId,
    services::{captcha::SolveRefusal, sessions::Session},
};

pub(super) async fn call_solve_captcha(
    state: &AppState,
    session: &std::sync::Arc<Session>,
    agent_label: &str,
    raw_args: &Value,
    client_cancel: CancellationToken,
    human_help_on: bool,
) -> CallToolResult {
    let dispatch_id = DispatchId::new();
    let dispatch_cancel = CancellationToken::new();
    if !session
        .try_register_dispatch(dispatch_id.clone(), dispatch_cancel.clone())
        .await
    {
        return browseros_mcp::ToolResult::error("BrowserOS neo session is no longer live")
            .into_call_tool_result();
    }
    let started = std::time::Instant::now();
    let result = async {
        let Some(page) = raw_args.get("page").and_then(Value::as_u64)
            .and_then(|page| u32::try_from(page).ok()).filter(|page| *page >= 1) else {
                return browseros_mcp::ToolResult::error("solve_captcha: page must be an integer greater than or equal to 1");
            };
        let Some(browser) = state.browser.session().await else {
            return browseros_mcp::ToolResult::error(browser_unavailable_message(state).await);
        };
        let binding = match state.targets.binding_for_execution(session.id()).await {
            Ok(binding) => binding,
            Err(error) => return browseros_mcp::ToolResult::error(format!("solve_captcha: {error}")),
        };
        let view = browseros_core::BrowserView::capture(browser, binding, None).await;
        let cancel = linked_cancel_token(session.child_token(), client_cancel, dispatch_cancel);
        let Some(captcha) = &state.captcha else {
            return browseros_mcp::ToolResult::error("solve_captcha: no provider is configured");
        };
        match captcha.solve(view, browseros_core::PageId(page), cancel, human_help_on).await {
            Ok(outcome) => {
                let value = json!(outcome);
                browseros_mcp::ToolResult {
                    content: vec![rmcp::model::ContentBlock::text(value.to_string())],
                    structured_content: Some(value), is_error: false, audit_hints: None,
                }
            }
            Err(refusal) => browseros_mcp::ToolResult::error(match refusal {
                SolveRefusal::InvalidPage(message) => format!("solve_captcha: {message}"),
                SolveRefusal::Busy(active) => captcha.reservations().busy_message(active.page)
                    .unwrap_or_else(|| format!("page {} is busy: solve_captcha (request {}) is working on a CAPTCHA. Wait for it to return, then take a fresh snapshot.", active.page, active.request_id)),
                SolveRefusal::BrowserUnavailable => browser_unavailable_message(state).await,
            }),
        }
    }.await;
    if let Err(error) = record_local_tool_dispatch(
        state,
        LocalToolDispatch {
            session,
            agent_label,
            tool_name: "solve_captcha",
            raw_args,
            result: &result,
            duration_ms: i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX),
            dispatch_id: dispatch_id.clone(),
        },
    )
    .await
    {
        tracing::warn!(error = %error, "local tool audit submission failed");
    }
    session.finish_dispatch(&dispatch_id).await;
    result.into_call_tool_result()
}

pub fn solve_captcha_tool() -> Tool {
    let schema = serde_json::Map::from_iter([
        ("type".into(), json!("object")),
        (
            "properties".into(),
            json!({"page": {"type": "integer", "minimum": 1}}),
        ),
        ("required".into(), json!(["page"])),
    ]);
    let output = json!({
        "type":"object",
        "required":["status","type","method","reason","requestId","page","durationMs","attempts","paidTasks","costUsd","message","events"],
        "properties": {
            "status":{"type":"string","enum":["solved","token_applied","already_clear","unsupported","failed","cancelled","needs_human"]},
            "type":{"type":["string","null"],"enum":["recaptcha_v2","recaptcha_v3","turnstile","cloudflare_challenge","hcaptcha","geetest","aws_waf","arkose","unknown",null]},
            "method":{"type":"string","enum":["none","auto_pass","checkbox","tiles","token"]},
            "reason":{"type":"string","enum":["auto_passed","checkbox_passed","tiles_accepted","token_accepted","awaiting_submit","no_captcha","already_passed","not_blocking","unsupported_type","scan_incomplete","acceptance_unconfirmed","provider_error","browser_error","deadline_exceeded","caller_cancelled","tab_closed","navigated","no_provider","provider_unavailable","tiles_exhausted","challenge_not_passed","spend_limit_reached"]},
            "requestId":{"type":"string","pattern":"^cap_[0-9A-HJKMNP-TV-Z]{26}$"},
            "page":{"type":"integer","minimum":1},
            "durationMs":{"type":"integer","minimum":0},
            "attempts":{"type":"object","required":["checkbox","tileRounds","token"],"properties":{
                "checkbox":{"type":"integer","minimum":0},"tileRounds":{"type":"integer","minimum":0},"token":{"type":"integer","minimum":0}}},
            "paidTasks":{"type":"integer","minimum":0},
            "costUsd":{"type":"number","minimum":0},
            "message":{"type":"string"},
            "events":{"type":"array","items":{"type":"object","required":["event","requestId","page","atMs","data"],"properties":{
                "event":{"type":"string","enum":["captcha.detected","captcha.solving","captcha.solved","captcha.token_applied","captcha.failed","captcha.handoff"]},
                "requestId":{"type":"string"},"page":{"type":"integer","minimum":1},"atMs":{"type":"integer","minimum":0},"data":{"type":"object"}}}}
        }
    });
    let Value::Object(output) = output else {
        unreachable!()
    };
    Tool::new(
        "solve_captcha",
        "Call when a snapshot shows a CAPTCHA or an action appears blocked. Detects, solves, and verifies the CAPTCHA on the page. token_applied: the token is in the page; submit and check. Do not act on the page while this runs. Never click CAPTCHAs yourself.",
        schema,
    ).with_raw_output_schema(std::sync::Arc::new(output))
        .with_annotations(ToolAnnotations::with_title("Solve a CAPTCHA").read_only(false).destructive(false).idempotent(false).open_world(true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tests_that_tool_schema_requires_a_positive_integer_page() {
        let tool = solve_captcha_tool();
        assert_eq!(tool.name, "solve_captcha");
        assert_eq!(tool.input_schema.get("type"), Some(&json!("object")));
        assert_eq!(tool.input_schema.get("required"), Some(&json!(["page"])));
        assert_eq!(
            tool.input_schema.get("properties"),
            Some(&json!({"page": {"type": "integer", "minimum": 1}}))
        );
    }

    #[test]
    fn tests_that_output_schema_accepts_token_applied() -> anyhow::Result<()> {
        let tool = solve_captcha_tool();
        let output = serde_json::to_value(tool.output_schema)?;
        for (pointer, value) in [
            ("/properties/status/enum", "token_applied"),
            ("/properties/reason/enum", "awaiting_submit"),
            (
                "/properties/events/items/properties/event/enum",
                "captcha.token_applied",
            ),
        ] {
            assert!(
                output
                    .pointer(pointer)
                    .and_then(Value::as_array)
                    .is_some_and(|values| values.contains(&json!(value))),
                "{value}"
            );
        }
        assert!(
            tool.description
                .as_deref()
                .is_some_and(|text| text.contains("token_applied"))
        );
        Ok(())
    }
}
