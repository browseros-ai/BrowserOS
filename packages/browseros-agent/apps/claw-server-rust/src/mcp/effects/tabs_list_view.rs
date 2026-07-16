use crate::mcp::dispatch::{ToolEffect, ToolEffectContext};
use browseros_core::PageId;
use browseros_mcp::ToolResult;
use futures_util::future::BoxFuture;
use rmcp::model::ContentBlock;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};

/// Replaces successful tabs-list results with the caller's ownership view.
pub fn apply(context: ToolEffectContext<'_>) -> BoxFuture<'_, anyhow::Result<Option<ToolResult>>> {
    Box::pin(async move {
        if context.result.is_error || !context.call.flags.list_tabs {
            return Ok(None);
        }
        let Some(identity) = &context.call.identity else {
            return Ok(None);
        };
        let Some(Value::Object(structured)) = context.result.structured_content.as_ref() else {
            return Ok(None);
        };
        let Some(pages) = structured.get("pages").and_then(Value::as_array) else {
            return Ok(None);
        };
        let live_page_ids = pages
            .iter()
            .filter_map(page_id)
            .map(PageId)
            .collect::<BTreeSet<_>>();
        let ownership = context.call.state.sessions.ownership();
        for stale_page in ownership.prune_missing_pages(&live_page_ids).await {
            identity.session.forget_first_capture(&stale_page).await;
        }
        let labels = context
            .call
            .state
            .sessions
            .snapshot()
            .await
            .into_iter()
            .map(|session| {
                (
                    session.agent().ownership_key(),
                    session.agent().label().to_string(),
                )
            })
            .collect::<HashMap<_, _>>();
        let mut annotated = Vec::with_capacity(pages.len());
        for page in pages {
            let owner = match page_id(page) {
                Some(page_id) => ownership.owner_of_page(&PageId(page_id)).await,
                None => None,
            };
            annotated.push(annotate_page(
                page,
                owner.as_ref(),
                &identity.ownership_key,
                &labels,
            ));
        }
        Ok(Some(ToolResult {
            content: vec![ContentBlock::text(render_tabs(&annotated))],
            structured_content: Some(json!({ "pages": annotated })),
            is_error: false,
        }))
    })
}

fn page_id(page: &Value) -> Option<u32> {
    page.get("page")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn annotate_page(
    page: &Value,
    owner: Option<&crate::domain::AgentKey>,
    caller: &crate::domain::AgentKey,
    labels: &HashMap<crate::domain::AgentKey, String>,
) -> Value {
    let mut annotated = page.clone();
    let Value::Object(fields) = &mut annotated else {
        return annotated;
    };
    let (ownership, owner_agent_id, owner_label) = match owner {
        None => ("user", Value::Null, Value::Null),
        Some(owner) if owner == caller => (
            "mine",
            Value::String(owner.as_str().to_string()),
            Value::Null,
        ),
        Some(owner) => (
            "other-agent",
            Value::String(owner.as_str().to_string()),
            labels
                .get(owner)
                .cloned()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
    };
    fields.insert(
        "ownership".to_string(),
        Value::String(ownership.to_string()),
    );
    fields.insert("ownerAgentId".to_string(), owner_agent_id);
    fields.insert("ownerLabel".to_string(), owner_label);
    annotated
}

fn render_tabs(pages: &[Value]) -> String {
    if pages.is_empty() {
        return "(no open pages)".to_string();
    }
    let mut sections = Vec::new();
    for (header, ownership) in [
        ("Your tabs:", "mine"),
        ("User's tabs:", "user"),
        ("Other agents' tabs:", "other-agent"),
    ] {
        let lines = pages
            .iter()
            .filter(|page| page.get("ownership").and_then(Value::as_str) == Some(ownership))
            .filter_map(format_tab_line)
            .collect::<Vec<_>>();
        if !lines.is_empty() {
            sections.push(format!("{header}\n{}", lines.join("\n")));
        }
    }
    sections.join("\n\n")
}

fn format_tab_line(page: &Value) -> Option<String> {
    let page_id = page.get("page").and_then(Value::as_u64)?;
    let url = page.get("url").and_then(Value::as_str).unwrap_or_default();
    let title = page
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let title = if title.is_empty() {
        String::new()
    } else {
        format!(" ({title})")
    };
    let owner = if page.get("ownership").and_then(Value::as_str) == Some("other-agent") {
        page.get("ownerLabel")
            .and_then(Value::as_str)
            .map(|label| format!(", owned by {label}"))
            .unwrap_or_default()
    } else {
        String::new()
    };
    Some(format!("[{page_id}] {url}{title}{owner}"))
}

const _: ToolEffect = apply;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn annotates_all_tabs_in_three_ownership_buckets_and_prunes_stale_claims()
    -> anyhow::Result<()> {
        let call = crate::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let identity = call.identity.as_ref().unwrap_or_else(|| unreachable!());
        let other_session = crate::domain::Session::new(
            crate::domain::SessionId::new("s2"),
            crate::domain::AgentRef::Ephemeral {
                agent_id: crate::domain::AgentId::new("cowork-a"),
                slug: "other".to_string(),
                label: "Cowork".to_string(),
            },
            tokio::time::Instant::now(),
        );
        call.state.sessions.insert_for_testing(other_session).await;
        call.state
            .sessions
            .ownership()
            .claim_page(identity.ownership_key.clone(), PageId(2))
            .await;
        call.state
            .sessions
            .ownership()
            .claim_page(crate::domain::AgentKey::new("other"), PageId(3))
            .await;
        call.state
            .sessions
            .ownership()
            .claim_page(crate::domain::AgentKey::new("other"), PageId(9))
            .await;
        let result = ToolResult::text(
            "all tabs",
            Some(json!({
                "pages": [
                    { "page": 2, "url": "https://owned.test", "title": "Owned" },
                    { "page": 4, "url": "https://user.test", "title": "User" },
                    { "page": 3, "url": "https://other.test", "title": "Other" }
                ]
            })),
        );
        let annotated = apply(ToolEffectContext {
            call: &call,
            result: &result,
            cancelled: false,
            duration_ms: 1,
        })
        .await
        .unwrap_or_else(|error| panic!("effect failed: {error}"))
        .unwrap_or(result);
        assert_eq!(
            annotated.structured_content,
            Some(json!({
                "pages": [
                    {
                        "page": 2,
                        "url": "https://owned.test",
                        "title": "Owned",
                        "ownership": "mine",
                        "ownerAgentId": "codex",
                        "ownerLabel": null
                    },
                    {
                        "page": 4,
                        "url": "https://user.test",
                        "title": "User",
                        "ownership": "user",
                        "ownerAgentId": null,
                        "ownerLabel": null
                    },
                    {
                        "page": 3,
                        "url": "https://other.test",
                        "title": "Other",
                        "ownership": "other-agent",
                        "ownerAgentId": "other",
                        "ownerLabel": "Cowork"
                    }
                ]
            }))
        );
        assert_eq!(
            annotated.content,
            vec![ContentBlock::text(
                "Your tabs:\n[2] https://owned.test (Owned)\n\nUser's tabs:\n[4] https://user.test (User)\n\nOther agents' tabs:\n[3] https://other.test (Other), owned by Cowork"
            )]
        );
        assert_eq!(
            call.state
                .sessions
                .ownership()
                .owner_of_page(&PageId(9))
                .await,
            None
        );
        Ok(())
    }

    #[tokio::test]
    async fn empty_tabs_list_keeps_the_ts_empty_state() -> anyhow::Result<()> {
        let call = crate::mcp::test_support::tool_call("tabs", json!({ "action": "list" })).await?;
        let result = ToolResult::text("all tabs", Some(json!({ "pages": [] })));
        let annotated = apply(ToolEffectContext {
            call: &call,
            result: &result,
            cancelled: false,
            duration_ms: 1,
        })
        .await?
        .unwrap_or(result);
        assert_eq!(
            annotated.content,
            vec![ContentBlock::text("(no open pages)")]
        );
        assert_eq!(annotated.structured_content, Some(json!({ "pages": [] })));
        Ok(())
    }
}
