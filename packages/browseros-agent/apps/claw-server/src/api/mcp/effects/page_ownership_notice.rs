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
        // `run` has no top-level page argument, so the outer arguments cannot say which
        // pages a script touched. The script hook recorded them; report those instead.
        if let Some(notice) = run_script_notice(context.call) {
            return Ok(Some(append_notice(context.result, notice)));
        }
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
                // A reconnect gives one client a new conversation, so an agent
                // reading its own earlier tab lands here with its own name in
                // `label`. Saying only "another agent" told it to abandon its
                // own work, so the note names the group to pass back when the
                // owner is in fact itself.
                //
                // A live session answers this exactly. A retired one is gone
                // from the snapshot while its claims survive until reaping, so
                // that case falls back to reading the slug off the conversation
                // id, which is the only attribution left.
                let same_client = context
                    .call
                    .state
                    .sessions
                    .snapshot()
                    .await
                    .into_iter()
                    .find(|session| session.convo_id() == &owner)
                    .map_or_else(
                        || crate::identity::convo_id_belongs_to_slug(&owner, identity.agent.slug()),
                        |session| session.agent().slug() == identity.agent.slug(),
                    );
                if same_client {
                    format!(
                        "Note: page {} was opened by {label} in an earlier session. If that was \
                         you and you are continuing that work, pass its tab group id as groupId \
                         on tabs action=\"new\" to take it back; tabs action=\"list\" reports \
                         the id. Otherwise you are still allowed to use it: leave it as you found \
                         it unless the user asked you to change it.",
                        page_id.0
                    )
                } else {
                    format!(
                        "Note: page {} belongs to another agent ({label}). You are allowed to use it; \
                         leave it as you found it unless the user asked you to change it.",
                        page_id.0
                    )
                }
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

/// Summarises the pages a `run` script touched that were not its own.
fn run_script_notice(call: &crate::api::mcp::dispatch::ToolCall) -> Option<String> {
    let pages = call.foreign_pages.lock().ok()?;
    if pages.is_empty() {
        return None;
    }
    let listed = pages
        .iter()
        .map(|(page, owner)| format!("page {page} belongs to {owner}"))
        .collect::<Vec<_>>()
        .join("; ");
    Some(format!(
        "Note: this script used tabs that are not yours. {listed}. You are allowed to use \
         them; leave them as you found them unless the user asked you to change them."
    ))
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

    /// A reconnect gives one client a new conversation, so an agent reading its
    /// own earlier tab sees its own name as the owner. Telling it only that the
    /// page is another agent's made a careful agent abandon its own work, so the
    /// note has to say it can take it back.
    #[tokio::test]
    async fn an_own_earlier_session_is_named_as_reclaimable() -> anyhow::Result<()> {
        let call = crate::api::mcp::test_support::tool_call(
            "evaluate",
            serde_json::json!({ "page": 7, "code": "return 1" }),
        )
        .await?;
        let identity = call.identity.as_ref().unwrap_or_else(|| unreachable!());
        // Same client, earlier conversation: what a reconnect leaves behind.
        let earlier = crate::services::sessions::Session::new(
            crate::ids::SessionId::new("earlier"),
            identity.agent.clone(),
            crate::identity::ConversationIdentity::new(identity.agent.slug(), "prior".to_string()),
            identity.agent_label.clone(),
            tokio::time::Instant::now(),
        );
        call.state
            .sessions
            .insert_for_testing(earlier.clone())
            .await;
        call.state
            .sessions
            .ownership()
            .claim_page(earlier.convo_id().clone(), PageId(7))
            .await;
        let result = ToolResult::text("script returned 1", None);
        let annotated = apply(ToolEffectContext {
            call: &call,
            result: &result,
            cancelled: false,
            duration_ms: 1,
        })
        .await
        .unwrap_or_else(|error| panic!("effect failed: {error}"))
        .unwrap_or_else(|| panic!("expected a notice"));
        let text = text_of(&annotated);
        assert!(text.contains("earlier session"), "{text}");
        assert!(text.contains("groupId"), "names the way back: {text}");
        assert!(
            !text.contains("belongs to another agent"),
            "must not tell an agent its own work is someone else's: {text}"
        );
        Ok(())
    }

    /// The retired case, which is the common one: the earlier session has left
    /// the live snapshot but its claims survive until reaping, so the only
    /// attribution left is the slug on the conversation id.
    #[tokio::test]
    async fn an_own_retired_session_is_still_named_as_reclaimable() -> anyhow::Result<()> {
        let call = crate::api::mcp::test_support::tool_call(
            "evaluate",
            serde_json::json!({ "page": 7, "code": "return 1" }),
        )
        .await?;
        let slug = call
            .identity
            .as_ref()
            .unwrap_or_else(|| unreachable!())
            .agent
            .slug()
            .to_string();
        // No session inserted: the conversation is gone, the claim is not.
        call.state
            .sessions
            .ownership()
            .claim_page(ConvoId::new(format!("{slug}-prior")), PageId(7))
            .await;
        let result = ToolResult::text("script returned 1", None);
        let annotated = apply(ToolEffectContext {
            call: &call,
            result: &result,
            cancelled: false,
            duration_ms: 1,
        })
        .await
        .unwrap_or_else(|error| panic!("effect failed: {error}"))
        .unwrap_or_else(|| panic!("expected a notice"));
        let text = text_of(&annotated);
        assert!(text.contains("earlier session"), "{text}");
        assert!(text.contains("groupId"), "names the way back: {text}");
        Ok(())
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

    /// `run` takes no `page` argument, so without the script hook's record the agent
    /// would be told nothing about the tabs its script actually used.
    #[tokio::test]
    async fn a_run_script_reports_the_foreign_pages_it_touched() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("run", json!({ "code": "return 1" })).await?;
        {
            let mut pages = call
                .foreign_pages
                .lock()
                .map_err(|_| anyhow::anyhow!("foreign_pages poisoned"))?;
            pages.insert(7, "another agent (research)".to_string());
            pages.insert(9, "the user".to_string());
        }

        let ok = ToolResult::text("script returned 1", None);
        let annotated = run(&call, &ok)
            .await
            .unwrap_or_else(|| panic!("expected a notice"));
        assert!(!annotated.is_error, "ownership must never fail a call");
        let text = text_of(&annotated);
        assert!(text.contains("script returned 1"), "original lost: {text}");
        assert!(
            text.contains("page 7 belongs to another agent (research)"),
            "{text}"
        );
        assert!(text.contains("page 9 belongs to the user"), "{text}");
        Ok(())
    }

    /// A script that stayed in its own tabs is not annotated.
    #[tokio::test]
    async fn a_run_script_in_its_own_tabs_is_quiet() -> anyhow::Result<()> {
        let call =
            crate::api::mcp::test_support::tool_call("run", json!({ "code": "return 1" })).await?;
        let ok = ToolResult::text("script returned 1", None);
        assert!(run(&call, &ok).await.is_none());
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
