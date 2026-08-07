use crate::services::sessions::Session;

const SMALL_NAME_WORD_LIMIT: usize = 3;
const SMALL_NAME_MAX_LEN: usize = 32;

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

/// Tab-group title the orchestrator should apply for this session right now.
pub async fn desired_group_title(session: &Session) -> String {
    build_session_group_title(
        client_prefix_from_slug(session.agent().slug()),
        &session.label().await,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        identity::{ClientIdentity, ConversationIdentity},
        ids::SessionId,
    };

    #[tokio::test]
    async fn desired_group_title_uses_label_when_named() {
        let session = Session::new(
            SessionId::new("s1"),
            ClientIdentity::Ephemeral {
                slug: "claude-code".to_string(),
                label: "Claude Code".to_string(),
            },
            ConversationIdentity::new("claude-code", "agile-alpaca".to_string()),
            "Codex".to_string(),
            tokio::time::Instant::now(),
        );
        assert_eq!(desired_group_title(&session).await, "claude/agile-alpaca");
        session.rename("flight-search".to_string()).await;
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
}
