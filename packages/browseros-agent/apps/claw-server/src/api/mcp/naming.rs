//! Presents session identities as bounded product/task titles and recognizes old
//! titles for reconnect hints. These names never grant ownership or replace the
//! session's original agent and conversation identities.

use crate::{
    identity::{client_product_prefix, slugify_client_name, unversioned_client_slug},
    services::sessions::Session,
};

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

const PREFIX_WORD_LIMIT: usize = 3;
const PREFIX_MAX_LEN: usize = 20;

/// The standard product prefix when recognized, otherwise the bounded client slug.
#[must_use]
pub fn client_prefix_from_slug(slug: &str) -> &str {
    client_product_prefix(slug).unwrap_or_else(|| legacy_client_prefix(slug))
}

/// Retains the previous title spelling for unknown clients and reconnect discovery.
/// Keep this separate from product recognition: old open groups outlive server updates.
fn legacy_client_prefix(slug: &str) -> &str {
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        return "agent";
    }
    let named = unversioned_client_slug(trimmed);
    let capped = &named[..cap_segments(named)];
    if capped.is_empty() { "agent" } else { capped }
}

/// Reported product metadata wins for presentation; the original agent slug stays
/// available for custom names and keeps owning conversations, claims, and colors.
fn session_client_prefix(session: &Session) -> &str {
    client_product_prefix(session.client_name())
        .unwrap_or_else(|| client_prefix_from_slug(session.agent().slug()))
}

/// Recognizes possible earlier tasks by canonical or historical title prefix.
/// This is a discovery hint only: a shared product name never establishes ownership.
#[must_use]
pub fn is_session_group_candidate(session: &Session, title: &str) -> bool {
    let Some((prefix, _)) = title.split_once('/') else {
        return false;
    };
    let expected = session_client_prefix(session);
    prefix == expected
        || client_product_prefix(prefix) == Some(expected)
        || prefix == legacy_client_prefix(session.agent().slug())
        || slugify_client_name(session.client_name())
            .is_some_and(|slug| prefix == legacy_client_prefix(&slug))
        // The old 20-character cap truncated our established Desktop wrapper
        // alias before its product suffix, so recognition of the full alias alone
        // cannot recover those still-open groups.
        || (expected == "claude" && prefix == "browserclaw-claude")
}

/// Byte length of `slug` kept under both caps, never splitting a segment except when
/// the first one alone already busts the budget.
fn cap_segments(slug: &str) -> usize {
    let mut end = 0;
    let mut kept = 0;
    let mut offset = 0;
    for segment in slug.split('-') {
        let start = offset;
        offset += segment.len() + 1;
        if segment.is_empty() {
            continue;
        }
        if kept == PREFIX_WORD_LIMIT {
            break;
        }
        let candidate = start + segment.len();
        if kept > 0 && candidate > PREFIX_MAX_LEN {
            break;
        }
        end = candidate;
        kept += 1;
    }
    if end > PREFIX_MAX_LEN {
        return floor_char_boundary(slug, PREFIX_MAX_LEN);
    }
    end
}

/// Largest index at or below `max` that lands on a char boundary. The slug is ASCII
/// by construction, but this is a public entry point and a mid-codepoint slice panics.
fn floor_char_boundary(text: &str, max: usize) -> usize {
    if max >= text.len() {
        return text.len();
    }
    text.char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= max)
        .last()
        .unwrap_or(0)
}

/// Builds the BrowserOS tab-group title for a named MCP session.
#[must_use]
pub fn build_session_group_title(prefix: &str, small_name: &str) -> String {
    format!("{prefix}/{small_name}")
}

/// Formats every session title (creation, rename reply, and reminder) through
/// the same product-resolution rules, including before a browser is connected.
#[must_use]
pub fn session_group_title(session: &Session, label: &str) -> String {
    build_session_group_title(session_client_prefix(session), label)
}

/// Tab-group title the orchestrator should apply for this session right now.
pub async fn desired_group_title(session: &Session) -> String {
    session_group_title(session, &session.label().await)
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
            "Claude Code".to_string(),
            tokio::time::Instant::now(),
        );
        assert_eq!(desired_group_title(&session).await, "claude/agile-alpaca");
        session.rename("flight-search".to_string()).await;
        assert_eq!(desired_group_title(&session).await, "claude/flight-search");
    }

    #[test]
    fn known_products_have_standard_prefixes_without_guessing_worker_names() {
        for (raw, expected) in [
            ("Claude Code", "claude"),
            ("claude-desktop", "claude"),
            ("browserclaw-claude-desktop-wrapper", "claude"),
            ("codex-mcp-client", "codex"),
            ("codex-posthog-dashboard", "codex"),
            ("Codex BrowserClaw", "codex"),
            ("Cursor", "cursor"),
            ("OpenCode", "opencode"),
            ("Antigravity", "antigravity"),
            ("VS Code", "vscode"),
            ("vscode-insiders", "vscode"),
            ("Zed", "zed"),
            ("claude-code-1-2-3", "claude"),
            ("cld-messaging", "cld-messaging"),
            ("claude-code-router", "claude-code-router"),
            ("cowork-finance-ops", "cowork-finance-ops"),
        ] {
            assert_eq!(client_prefix_from_slug(raw), expected, "{raw}");
        }
    }

    #[tokio::test]
    async fn product_titles_and_legacy_candidates_preserve_conversation_identity() {
        let session = Session::new(
            SessionId::new("s1"),
            ClientIdentity::Ephemeral {
                slug: "cld-messaging".to_string(),
                label: "cld-messaging".to_string(),
            },
            ConversationIdentity::new("cld-messaging", "tidy-seal".to_string()),
            "Claude Code".to_string(),
            tokio::time::Instant::now(),
        );
        assert_eq!(desired_group_title(&session).await, "claude/tidy-seal");
        assert_eq!(session.agent().slug(), "cld-messaging");
        assert_eq!(session.convo_id().as_str(), "cld-messaging-tidy-seal");
        for title in [
            "claude/task",
            "claude-code/task",
            "cld-messaging/task",
            "browserclaw-claude/task",
        ] {
            assert!(is_session_group_candidate(&session, title), "{title}");
        }
        for title in [
            "codex/task",
            "claude-extra/task",
            "claude",
            "cld-other/task",
        ] {
            assert!(!is_session_group_candidate(&session, title), "{title}");
        }
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
    fn client_prefix_keeps_whole_multi_word_names() {
        assert_eq!(client_prefix_from_slug("claude-code"), "claude");
        assert_eq!(client_prefix_from_slug("cursor"), "cursor");
        assert_eq!(
            client_prefix_from_slug("cowork-finance-ops"),
            "cowork-finance-ops"
        );
    }

    #[test]
    fn claude_products_share_the_approved_display_prefix() {
        assert_eq!(
            client_prefix_from_slug("claude-code"),
            client_prefix_from_slug("claude-desktop")
        );
    }

    #[test]
    fn client_prefix_drops_trailing_version_but_keeps_interior_digits() {
        assert_eq!(client_prefix_from_slug("claude-code-1-2-3"), "claude");
        assert_eq!(client_prefix_from_slug("gpt-4-turbo"), "gpt-4-turbo");
        assert_eq!(client_prefix_from_slug("1-2-3"), "1-2-3");
    }

    #[test]
    fn client_prefix_is_bounded() {
        assert_eq!(client_prefix_from_slug("a-b-c-d-e"), "a-b-c");
        assert_eq!(
            client_prefix_from_slug("alpha-betagammadeltaepsilonzeta"),
            "alpha"
        );
        assert_eq!(
            client_prefix_from_slug("averyveryverylongsinglesegmentname"),
            "averyveryverylongsin"
        );
    }

    #[test]
    fn client_prefix_falls_back_when_nothing_survives() {
        assert_eq!(client_prefix_from_slug(""), "agent");
        assert_eq!(client_prefix_from_slug("---"), "agent");
    }

    #[test]
    fn group_title_combines_prefix_and_name() {
        assert_eq!(
            build_session_group_title("claude", "invoice-processing"),
            "claude/invoice-processing"
        );
    }
}
