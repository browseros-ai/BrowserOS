use crate::{api::mcp::prompt::TASK_GROUP_CLEANUP, services::sessions::Session};
use rmcp::model::{CallToolResult, ContentBlock};

/// Runs after both browser effects and local tools, including an early `name_session`.
pub(in crate::api::mcp) fn apply(
    result: &mut CallToolResult,
    session: Option<&Session>,
    enabled: bool,
    cancelled: bool,
) {
    if !enabled || cancelled || result.is_error == Some(true) {
        return;
    }
    let Some(session) = session else {
        return;
    };
    if session.child_token().is_cancelled() || !session.try_mark_cleanup_tip() {
        return;
    }
    result
        .content
        .push(ContentBlock::text(format!("Tip: {TASK_GROUP_CLEANUP}")));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        identity::{ClientIdentity, ConversationIdentity},
        ids::SessionId,
    };
    use serde_json::json;
    use std::sync::Arc;

    fn session() -> Arc<Session> {
        Session::new(
            SessionId::new("cleanup-session"),
            ClientIdentity::Ephemeral {
                slug: "codex".to_string(),
                label: "Codex".to_string(),
            },
            ConversationIdentity::new("codex", "agile-alpaca".to_string()),
            "Codex".to_string(),
            tokio::time::Instant::now(),
        )
    }

    fn success() -> CallToolResult {
        CallToolResult::success(vec![ContentBlock::text("ok")])
    }

    #[test]
    fn tests_that_cleanup_appends_once_and_preserves_the_entire_result() -> anyhow::Result<()> {
        let session = session();
        let original: CallToolResult = serde_json::from_value(json!({
            "content": [
                { "type": "text", "text": "opened" },
                { "type": "image", "data": "aW1hZ2U=", "mimeType": "image/png" }
            ],
            "structuredContent": { "pages": [1] },
            "isError": false,
            "_meta": { "custom": "retained" }
        }))?;
        let mut result = original.clone();
        apply(&mut result, Some(&session), true, false);
        assert_eq!(result.content.len(), original.content.len() + 1);
        let tip = result
            .content
            .last()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing cleanup tip"))?;
        let mut expected = original.clone();
        expected.content.push(tip.clone());
        assert_eq!(result, expected);
        let text = tip
            .as_text()
            .ok_or_else(|| anyhow::anyhow!("tip is not text"))?;
        assert!(
            text.text
                .starts_with("Tip: Close tab groups after the agent finishes a task")
        );
        assert!(
            text.text
                .contains("tab_groups action=\"close\" groupId=\"<your task group id>\"")
        );
        assert!(text.text.contains("other agents' groups"));
        assert!(
            text.text
                .contains("explicit user requests to keep results open")
        );
        assert!(text.text.len() < 500);
        let mut repeated = original.clone();
        apply(&mut repeated, Some(&session), true, false);
        assert_eq!(repeated, original);
        Ok(())
    }

    #[test]
    fn tests_that_cleanup_errors_off_and_cancelled_calls_keep_the_tip_available() {
        let session = session();
        let mut error = CallToolResult::error(vec![ContentBlock::text("failed")]);
        let original_error = error.clone();
        apply(&mut error, Some(&session), true, false);
        assert_eq!(error, original_error);
        for (enabled, cancelled) in [(false, false), (true, true), (false, true)] {
            let mut result = success();
            apply(&mut result, Some(&session), enabled, cancelled);
            assert_eq!(result, success());
        }
        let mut result = success();
        apply(&mut result, Some(&session), true, false);
        assert_eq!(result.content.len(), 2);
    }

    #[test]
    fn tests_that_cleanup_missing_identity_keeps_the_tip_available() {
        let mut result = success();
        apply(&mut result, None, true, false);
        assert_eq!(result, success());
        apply(&mut result, Some(&session()), true, false);
        assert_eq!(result.content.len(), 2);
    }

    #[tokio::test]
    async fn tests_that_cleanup_is_independent_of_both_rename_and_rename_nudges() {
        for renamed in [false, true] {
            let session = session();
            if renamed {
                session.rename("already-named".to_string()).await;
            } else {
                for _ in 0..5 {
                    assert!(session.take_rename_nudge().await.is_some());
                }
                assert!(session.take_rename_nudge().await.is_none());
            }
            let mut result = success();
            apply(&mut result, Some(&session), true, false);
            assert_eq!(result.content.len(), 2);
        }
    }

    #[test]
    fn tests_that_cleanup_separate_sessions_each_receive_a_tip() {
        for _ in 0..2 {
            let mut result = success();
            apply(&mut result, Some(&session()), true, false);
            assert_eq!(result.content.len(), 2);
        }
    }

    #[test]
    fn tests_that_cleanup_concurrent_responses_share_one_tip() -> anyhow::Result<()> {
        let session = session();
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let workers = (0..8)
            .map(|_| {
                let session = session.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let mut result = success();
                    barrier.wait();
                    apply(&mut result, Some(&session), true, false);
                    result.content.len() - 1
                })
            })
            .collect::<Vec<_>>();
        let mut tips = 0;
        for worker in workers {
            tips += worker
                .join()
                .map_err(|_| anyhow::anyhow!("cleanup worker panicked"))?;
        }
        assert_eq!(tips, 1);
        Ok(())
    }

    #[test]
    fn tests_that_cleanup_cancelled_sessions_leave_the_response_unchanged() {
        let session = session();
        session.cancel();
        let mut result = success();
        apply(&mut result, Some(&session), true, false);
        assert_eq!(result, success());
    }
}
