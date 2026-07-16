//! MCP session naming via elicitation, ported from the TS claw-server
//! (`src/lib/mcp-session/naming.ts` + `src/mcp/session-naming.ts`). The
//! normalizer, prompt, schema, and retry table must stay in lockstep with
//! the TS oracle so both servers title tab groups identically.

use crate::domain::Session;
use rmcp::{
    RoleServer,
    model::{ElicitRequestParams, ElicitationAction, ElicitationSchema},
    service::{Peer, ServiceError},
};
use serde_json::Value;
use std::time::Duration;
use tracing::debug;

const ELICITATION_TIMEOUT: Duration = Duration::from_secs(120);
const ELICITATION_RETRY_DELAY: Duration = Duration::from_secs(2);
const SMALL_NAME_WORD_LIMIT: usize = 3;
const SMALL_NAME_MAX_LEN: usize = 32;
const SESSION_NAME_INPUT_MAX_LEN: u32 = 64;
const NAME_GUIDANCE: &str = "a small lowercase 2-3 word name for what this session is doing";

/// Normalizes a user-provided session label into a short tab-group slug.
#[must_use]
pub fn normalize_small_name(raw: &str) -> String {
    let lowered = raw.to_lowercase();
    let words = lowered
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .take(SMALL_NAME_WORD_LIMIT)
        .collect::<Vec<_>>();
    let mut name = words.join("-");
    name.truncate(SMALL_NAME_MAX_LEN);
    name.trim_matches('-').to_string()
}

/// Returns the short client namespace used before the session label.
#[must_use]
pub fn client_prefix_from_slug(slug: &str) -> &str {
    slug.split('-')
        .find(|part| !part.is_empty())
        .unwrap_or("agent")
}

/// Builds the BrowserOS tab-group title for a named MCP session.
#[must_use]
pub fn build_session_group_title(prefix: &str, small_name: &str) -> String {
    format!("{prefix}/{small_name}")
}

/// Builds the elicitation message with the concrete client prefix.
#[must_use]
pub fn build_session_name_prompt(prefix: &str) -> String {
    format!(
        "Name this browser session: {NAME_GUIDANCE} (e.g. \"invoice processing\"). Tabs will be grouped as {prefix}/<name>."
    )
}

fn session_name_schema() -> ElicitationSchema {
    ElicitationSchema::builder()
        .required_string_property("name", |schema| {
            schema
                .title("Session name")
                .description(format!(
                    "Use {NAME_GUIDANCE}, such as \"invoice processing\"."
                ))
                .max_length(SESSION_NAME_INPUT_MAX_LEN)
        })
        .build_unchecked()
}

/// Tab-group title the orchestrator should apply for this session right now.
pub async fn desired_group_title(session: &Session) -> String {
    match session.session_label().await {
        Some(label) => {
            build_session_group_title(client_prefix_from_slug(session.agent().slug()), &label)
        }
        None => session.agent().slug().to_string(),
    }
}

/// One elicitation attempt's result, folded into the TS retry table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElicitNameOutcome {
    /// Client accepted and returned a raw `name` string.
    Accepted(String),
    /// Decline, cancel, or accept without a usable string: stop silently.
    NoName,
    /// The 120s window elapsed — the user ignored the prompt; never retry.
    TimedOut,
    /// Transport-level failure (eg. no SSE stream yet): retry once after 2s.
    Failed(String),
}

/// Runs the elicitation retry table and returns the normalized label.
pub async fn elicit_session_name<F, Fut>(mut elicit: F) -> Option<String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ElicitNameOutcome>,
{
    for attempt in 0..2 {
        match elicit().await {
            ElicitNameOutcome::Accepted(raw) => {
                let name = normalize_small_name(&raw);
                return if name.is_empty() { None } else { Some(name) };
            }
            ElicitNameOutcome::NoName => return None,
            ElicitNameOutcome::TimedOut => {
                debug!("mcp session naming elicitation timed out");
                return None;
            }
            ElicitNameOutcome::Failed(reason) => {
                debug!(error = %reason, "mcp session naming elicitation unavailable");
                if attempt == 0 {
                    tokio::time::sleep(ELICITATION_RETRY_DELAY).await;
                }
            }
        }
    }
    None
}

/// Sends one session-name elicitation over the live peer.
pub async fn peer_elicit_session_name(peer: &Peer<RoleServer>, prefix: &str) -> ElicitNameOutcome {
    let params = ElicitRequestParams::FormElicitationParams {
        meta: None,
        message: build_session_name_prompt(prefix),
        requested_schema: session_name_schema(),
    };
    match peer
        .create_elicitation_with_timeout(params, Some(ELICITATION_TIMEOUT))
        .await
    {
        Ok(result) => match result.action {
            ElicitationAction::Accept => result
                .content
                .as_ref()
                .and_then(|content| content.get("name"))
                .and_then(Value::as_str)
                .map(|name| ElicitNameOutcome::Accepted(name.to_string()))
                .unwrap_or(ElicitNameOutcome::NoName),
            _ => ElicitNameOutcome::NoName,
        },
        Err(ServiceError::Timeout { .. }) => ElicitNameOutcome::TimedOut,
        Err(err) => ElicitNameOutcome::Failed(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AgentId, AgentRef, SessionId};
    use serde_json::json;

    #[tokio::test]
    async fn desired_group_title_uses_label_when_named() {
        let session = Session::new(
            SessionId::new("s1"),
            AgentRef::Ephemeral {
                agent_id: AgentId::new("claude-code-abc123"),
                slug: "claude-code".to_string(),
                label: "Claude Code".to_string(),
            },
            tokio::time::Instant::now(),
        );
        assert_eq!(desired_group_title(&session).await, "claude-code");
        session.set_session_label("flight-search".to_string()).await;
        assert_eq!(desired_group_title(&session).await, "claude/flight-search");
    }

    #[test]
    fn normalize_small_name_matches_ts_vectors() {
        assert_eq!(
            normalize_small_name("Invoice Processing!"),
            "invoice-processing"
        );
        assert_eq!(normalize_small_name("  LinkedIn   Jobs "), "linkedin-jobs");
        assert_eq!(
            normalize_small_name("one two three four five"),
            "one-two-three"
        );
        assert_eq!(normalize_small_name("!!!"), "");
        assert_eq!(normalize_small_name(""), "");
        assert_eq!(normalize_small_name("日本語"), "");
        assert_eq!(normalize_small_name(&"x".repeat(60)), "x".repeat(32));
    }

    #[test]
    fn client_prefix_matches_ts_vectors() {
        assert_eq!(client_prefix_from_slug("claude-code"), "claude");
        assert_eq!(client_prefix_from_slug("cursor"), "cursor");
        assert_eq!(client_prefix_from_slug(""), "agent");
    }

    #[test]
    fn group_title_combines_prefix_and_name() {
        assert_eq!(
            build_session_group_title("claude", "invoice-processing"),
            "claude/invoice-processing"
        );
    }

    #[test]
    fn prompt_names_the_group_namespace() {
        let prompt = build_session_name_prompt("claude");
        assert!(prompt.contains("Tabs will be grouped as claude/<name>"));
        assert!(prompt.starts_with("Name this browser session:"));
    }

    #[test]
    fn schema_matches_ts_requested_schema() -> anyhow::Result<()> {
        let schema = serde_json::to_value(session_name_schema())?;
        assert_eq!(
            schema,
            json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "title": "Session name",
                        "description": "Use a small lowercase 2-3 word name for what this session is doing, such as \"invoice processing\".",
                        "maxLength": 64
                    }
                },
                "required": ["name"]
            })
        );
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn accepted_name_is_normalized() {
        let name = elicit_session_name(|| {
            std::future::ready(ElicitNameOutcome::Accepted(
                "Invoice Processing".to_string(),
            ))
        })
        .await;
        assert_eq!(name.as_deref(), Some("invoice-processing"));
    }

    #[tokio::test(start_paused = true)]
    async fn accepted_name_normalizing_to_empty_is_dropped() {
        let mut calls = 0;
        let name = elicit_session_name(|| {
            calls += 1;
            std::future::ready(ElicitNameOutcome::Accepted("!!!".to_string()))
        })
        .await;
        assert_eq!(name, None);
        assert_eq!(calls, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_never_retries() {
        let mut calls = 0;
        let name = elicit_session_name(|| {
            calls += 1;
            std::future::ready(ElicitNameOutcome::TimedOut)
        })
        .await;
        assert_eq!(name, None);
        assert_eq!(calls, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn decline_never_retries() {
        let mut calls = 0;
        let name = elicit_session_name(|| {
            calls += 1;
            std::future::ready(ElicitNameOutcome::NoName)
        })
        .await;
        assert_eq!(name, None);
        assert_eq!(calls, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn transport_failure_retries_once_then_accepts() {
        let mut calls = 0;
        let name = elicit_session_name(|| {
            calls += 1;
            std::future::ready(if calls == 1 {
                ElicitNameOutcome::Failed("no stream yet".to_string())
            } else {
                ElicitNameOutcome::Accepted("flight search".to_string())
            })
        })
        .await;
        assert_eq!(name.as_deref(), Some("flight-search"));
        assert_eq!(calls, 2);
    }

    #[tokio::test(start_paused = true)]
    async fn two_transport_failures_give_up() {
        let mut calls = 0;
        let name = elicit_session_name(|| {
            calls += 1;
            std::future::ready(ElicitNameOutcome::Failed("still no stream".to_string()))
        })
        .await;
        assert_eq!(name, None);
        assert_eq!(calls, 2);
    }
}
