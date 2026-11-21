use super::*;
use crate::config::{Behavior, ServerMode};

#[tokio::test]
async fn appends_exactly_five_trailing_tips_and_preserves_image_content() -> anyhow::Result<()> {
    let call = crate::api::mcp::test_support::tool_call_in_mode(
        "tabs",
        json!({ "action": "list" }),
        ServerMode::Hosted,
    )
    .await?;
    let image = ContentBlock::image("aW1hZ2U=".to_string(), "image/png".to_string());
    let original = ToolResult {
        content: vec![ContentBlock::text("ok"), image.clone()],
        is_error: false,
        structured_content: Some(json!({ "pages": [] })),
        audit_hints: None,
    };

    for _ in 0..5 {
        let result = apply_result(&call, &original).await?;
        assert_eq!(result.content[..2], original.content);
        assert_eq!(
            result
                .content
                .last()
                .and_then(ContentBlock::as_text)
                .map(|text| text.text.as_str()),
            Some(
                "Tip: this session is \"codex/agile-alpaca\" — name it by passing taskName=\"<2-3 word task label>\" on your next call"
            )
        );
        assert_eq!(result.structured_content, original.structured_content);
    }
    assert_result_eq(&apply_result(&call, &original).await?, &original);
    Ok(())
}

#[tokio::test]
async fn errors_do_not_consume_a_nudge_and_rename_stops_tips() -> anyhow::Result<()> {
    let call = crate::api::mcp::test_support::tool_call_in_mode(
        "tabs",
        json!({ "action": "new" }),
        ServerMode::Hosted,
    )
    .await?;
    let error = ToolResult::error("failed");
    assert_result_eq(&apply_result(&call, &error).await?, &error);

    let success = ToolResult::text("opened", Some(json!({ "page": 1 })));
    assert_eq!(apply_result(&call, &success).await?.content.len(), 2);
    let session = &call
        .identity
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("identity missing"))?
        .session;
    session.rename("invoice-processing".to_string()).await;
    assert_result_eq(&apply_result(&call, &success).await?, &success);
    Ok(())
}

#[tokio::test]
async fn separate_sessions_have_independent_nudge_budgets() -> anyhow::Result<()> {
    let first = crate::api::mcp::test_support::tool_call_in_mode(
        "tabs",
        json!({ "action": "list" }),
        ServerMode::Hosted,
    )
    .await?;
    let second = crate::api::mcp::test_support::tool_call_in_mode(
        "tabs",
        json!({ "action": "list" }),
        ServerMode::Hosted,
    )
    .await?;
    let success = ToolResult::text("ok", None);

    for _ in 0..5 {
        assert_eq!(apply_result(&first, &success).await?.content.len(), 2);
    }
    assert_result_eq(&apply_result(&first, &success).await?, &success);
    assert_eq!(apply_result(&second, &success).await?.content.len(), 2);
    Ok(())
}

#[tokio::test]
async fn desktop_internal_dispatches_preserve_the_run_reminder_budget() -> anyhow::Result<()> {
    let call = crate::api::mcp::test_support::tool_call_with_behavior(
        "tabs",
        json!({ "action": "list" }),
        ServerMode::Desktop,
        Behavior {
            playwright_run: true,
            ..Behavior::default()
        },
    )
    .await?;
    let success = ToolResult::text("ok", None);
    for _ in 0..6 {
        assert_result_eq(&apply_result(&call, &success).await?, &success);
    }
    let session = &call
        .identity
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("identity missing"))?
        .session;
    for _ in 0..5 {
        assert!(session.take_rename_nudge().await.is_some());
    }
    assert_eq!(session.take_rename_nudge().await, None);
    Ok(())
}
