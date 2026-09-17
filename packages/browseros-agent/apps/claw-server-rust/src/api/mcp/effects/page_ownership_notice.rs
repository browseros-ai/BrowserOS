//! Tells an agent whose tab it just acted on.
//!
//! # Ownership is a LABEL, never a permission
//!
//! This module exists so an agent knows whose tab it is looking at: the user's,
//! another agent's, or its own. It must never gate a dispatch, and nothing here may
//! turn a result into an error. Agents are allowed to act on the user's tabs and on
//! other agents' tabs. This browser exists for agents, and the user grants that
//! access by setting it up and signing it in.
//!
//! There was a guard here once, `guards::page_ownership`, that rejected any dispatch
//! against a page the calling conversation did not own. Two things followed, both bad:
//!
//! 1. The user's own tabs became permanently unreachable. The guard's own comment
//!    conceded it: unclaimed user pages were rejected and never auto-claimed. An agent
//!    could not act on a tab the user had opened for it.
//! 2. Because ownership keys on a per-session id, and the 2026-07-28 MCP revision
//!    removed protocol-level sessions, any lost session handle minted a new identity
//!    and orphaned the agent's own pages mid-task. A label was deciding access, so
//!    losing the label lost the work.
//!
//! Both were the same mistake: treating a label as a boundary. The guard is gone and
//! must not come back.
//!
//! If a future change needs to restrict what an agent may touch, that is a consent
//! decision belonging to the user, expressed somewhere the user can see and change.
//! It does not belong here, and it is not ownership.
//!
//! The rule: **ownership answers "whose is this", never "may I".**

use crate::api::mcp::dispatch::{ToolEffect, ToolEffectContext, extract_page_id};
use browseros_core::PageId;
use browseros_mcp::ToolResult;
use futures_util::future::BoxFuture;

/// Appends a one-line note when the acted-on page belongs to someone else.
///
/// Returns `None` far more often than not: the common case is an agent working in its
/// own tab, which needs no annotation. Silence means "yours".
pub fn apply(context: ToolEffectContext<'_>) -> BoxFuture<'_, anyhow::Result<Option<ToolResult>>> {
    Box::pin(async move {
        // Nothing to say about a call that failed for its own reasons.
        if context.result.is_error {
            return Ok(None);
        }
        let Some(identity) = &context.call.identity else {
            return Ok(None);
        };
        let Some(page_id) = extract_page_id(context.call) else {
            return Ok(None);
        };
        let page_id = PageId(page_id);

        let owner = context.call.state.sessions.owner_of_page(&page_id).await;
        let notice = match owner {
            // Claimed by this conversation: the ordinary case, say nothing.
            Some(owner) if owner == identity.ownership_key => return Ok(None),
            Some(owner) => {
                // Same source `tabs list` labels from, so a page reads the same way
                // whether the agent listed it or acted on it.
                let label = context
                    .call
                    .state
                    .sessions
                    .snapshot()
                    .await
                    .into_iter()
                    .find(|session| session.convo_id() == &owner)
                    .map_or_else(
                        || owner.as_str().to_string(),
                        |session| session.agent().label().to_string(),
                    );
                format!(
                    "Note: page {} belongs to another agent ({label}). You are allowed to use it; \
                     leave it as you found it unless the user asked you to change it.",
                    page_id.0
                )
            }
            // No claim at all means the user opened it themselves.
            None => format!(
                "Note: page {} is one of the user's own tabs, not opened by an agent. You are \
                 allowed to use it; leave it as you found it unless the user asked you to change it.",
                page_id.0
            ),
        };

        Ok(Some(append_notice(context.result, notice)))
    })
}

/// Adds the note as an extra text block, leaving the original content untouched so a
/// caller parsing the first block is unaffected.
fn append_notice(result: &ToolResult, notice: String) -> ToolResult {
    let mut annotated = result.clone();
    annotated
        .content
        .push(rmcp::model::ContentBlock::text(notice));
    annotated
}

const _: ToolEffect = apply;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::mcp::dispatch::ToolEffectContext;
    use crate::ids::ConvoId;
    use rmcp::model::ContentBlock;
    use serde_json::json;

    fn text_of(result: &ToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    async fn run(
        call: &crate::api::mcp::dispatch::ToolCall,
        result: &ToolResult,
    ) -> Option<ToolResult> {
        apply(ToolEffectContext {
            call,
            result,
            cancelled: false,
            duration_ms: 1,
        })
        .await
        .unwrap_or(None)
    }

    /// The incident that started this, replayed. A page claimed by a different
    /// conversation, which is what a lost session handle produces, must still be usable.
    /// This is the assertion the old guard failed.
    #[tokio::test]
    async fn another_conversations_page_is_usable_and_labelled() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("navigate", json!({ "page": 7 })).await?;
        call.state
            .sessions
            .ownership()
            .claim_page(ConvoId::new("other"), PageId(7))
            .await;

        let ok = ToolResult::text("navigated", None);
        let annotated = run(&call, &ok).await;

        let annotated = annotated.unwrap_or_else(|| panic!("expected a notice"));
        assert!(!annotated.is_error, "ownership must never fail a call");
        let text = text_of(&annotated);
        assert!(text.contains("navigated"), "original result lost: {text}");
        assert!(text.contains("another agent"), "{text}");
        assert!(text.contains("You are allowed to use it"), "{text}");
        Ok(())
    }

    /// A page with no claim at all is one the user opened. Agents may use those too.
    #[tokio::test]
    async fn a_user_tab_is_usable_and_identified_as_theirs() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("navigate", json!({ "page": 3 })).await?;
        let ok = ToolResult::text("navigated", None);

        let annotated = run(&call, &ok)
            .await
            .unwrap_or_else(|| panic!("expected a notice"));
        assert!(!annotated.is_error);
        let text = text_of(&annotated);
        assert!(text.contains("user's own tabs"), "{text}");
        Ok(())
    }

    /// The common case stays quiet. Silence means the page is yours.
    #[tokio::test]
    async fn your_own_page_is_not_annotated() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("navigate", json!({ "page": 9 })).await?;
        let owner = call
            .identity
            .as_ref()
            .unwrap_or_else(|| panic!("identity"))
            .ownership_key
            .clone();
        call.state
            .sessions
            .ownership()
            .claim_page(owner, PageId(9))
            .await;

        let ok = ToolResult::text("navigated", None);
        assert!(run(&call, &ok).await.is_none(), "own pages need no notice");
        Ok(())
    }

    /// A call that failed on its own merits is left alone; ownership never comments on
    /// somebody else's error.
    #[tokio::test]
    async fn a_failed_call_is_left_untouched() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("navigate", json!({ "page": 7 })).await?;
        call.state
            .sessions
            .ownership()
            .claim_page(ConvoId::new("other"), PageId(7))
            .await;

        let failed = ToolResult::error("navigation failed");
        assert!(run(&call, &failed).await.is_none());
        Ok(())
    }
}
