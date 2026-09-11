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

const PREFIX_WORD_LIMIT: usize = 3;
const PREFIX_MAX_LEN: usize = 20;

/// Returns the client namespace used before the session label.
///
/// Keeps whole slug segments so a two-word product name survives intact:
/// truncating at the first segment would render `claude-code` and `claude-desktop`
/// as the same `claude` chip, which defeats the point of naming the owner. Bounded
/// by a word count and a character budget so a long name cannot dominate the tab
/// strip. Every step cuts from the right, so the result is always a prefix of the
/// input and the borrow is preserved.
#[must_use]
pub fn client_prefix_from_slug(slug: &str) -> &str {
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        return "agent";
    }
    let named = &trimmed[..strip_trailing_version(trimmed)];
    let capped = &named[..cap_segments(named)];
    if capped.is_empty() { "agent" } else { capped }
}

/// Byte length of `slug` with a trailing run of all-digit segments removed. Some
/// clients fold the version into the name, so `Claude Code 1.2.3` slugifies to
/// `claude-code-1-2-3`. Only a trailing run is dropped, which is what keeps the
/// interior digits of `gpt-4-turbo` intact. An all-digit slug is left whole rather
/// than reduced to nothing.
fn strip_trailing_version(slug: &str) -> usize {
    let mut end = slug.len();
    for segment in slug.rsplit('-') {
        if segment.is_empty() || !segment.bytes().all(|byte| byte.is_ascii_digit()) {
            break;
        }
        end = end.saturating_sub(segment.len() + 1);
    }
    if end == 0 { slug.len() } else { end }
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
        assert_eq!(
            desired_group_title(&session).await,
            "claude-code/agile-alpaca"
        );
        session.rename("flight-search".to_string()).await;
        assert_eq!(
            desired_group_title(&session).await,
            "claude-code/flight-search"
        );
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
        assert_eq!(client_prefix_from_slug("claude-code"), "claude-code");
        assert_eq!(client_prefix_from_slug("cursor"), "cursor");
        assert_eq!(
            client_prefix_from_slug("cowork-finance-ops"),
            "cowork-finance-ops"
        );
    }

    #[test]
    fn client_prefix_no_longer_collides_distinct_products() {
        assert_ne!(
            client_prefix_from_slug("claude-code"),
            client_prefix_from_slug("claude-desktop")
        );
    }

    #[test]
    fn client_prefix_drops_trailing_version_but_keeps_interior_digits() {
        assert_eq!(client_prefix_from_slug("claude-code-1-2-3"), "claude-code");
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
