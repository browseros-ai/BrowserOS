use super::*;
use crate::config::{Behavior, ServerMode};

#[tokio::test]
async fn terminal_audit_counts_the_final_wire_content_once() -> anyhow::Result<()> {
    for mode in [ServerMode::Desktop, ServerMode::Hosted] {
        let call = crate::api::mcp::test_support::tool_call_with_behavior(
            "tabs",
            json!({ "action": "list" }),
            mode,
            Behavior {
                playwright_run: true,
                ..Behavior::defaults(mode)
            },
        )
        .await?;
        let initial = ToolResult::text("initial", Some(json!({ "internal": true })));
        let final_result = run_effects(
            ToolEffectContext {
                call: &call,
                result: &initial,
                cancelled: false,
                duration_ms: 1,
            },
            &[
                NamedToolEffect {
                    name: "replace",
                    run: replacement_effect,
                },
                NamedToolEffect {
                    name: "session-naming",
                    run: effects::session_naming::apply,
                },
            ],
        )
        .await;
        run_observers(
            ToolObserverContext {
                call: &call,
                result: &final_result,
                cancelled: false,
                duration_ms: 1,
            },
            &[NamedToolObserver {
                name: "audit",
                run: observers::audit::apply,
            }],
        )
        .await;
        call.state
            .audit_worker
            .flush_session(call.session_id.as_str())
            .await?;
        let wire = wire_result(final_result.clone(), true);
        assert_eq!(wire.content.len(), if mode.is_hosted() { 2 } else { 1 });
        assert_eq!(wire.structured_content, final_result.structured_content);

        let row = call
            .state
            .audit_log
            .list_dispatches(ListDispatchesQuery::default())
            .await?
            .rows
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("dispatch missing"))?;
        assert_eq!(row.token_estimator_version, TOKEN_ESTIMATOR_VERSION);
        assert_eq!(
            row.tool_input_token_estimate,
            estimate_tool_input_tokens(call.tool().name, &call.raw_args)
        );
        assert_eq!(
            row.tool_output_token_estimate,
            estimate_tool_output_tokens(&wire.content)
        );
    }
    Ok(())
}
