//! Product names shared by local tab presentation and analytics alias recognition.
//! Reported names remain intact on sessions; these pure lookups never define
//! conversation identity, ownership, or permission.

use super::slugify_client_name;

// Keep full wire aliases here. Tab naming also derives their old bounded forms
// from this table because those titles can survive a server upgrade.
pub(crate) const CLIENT_ALIASES: [(&str, &str); 4] = [
    ("codex-mcp-client", "codex"),
    ("codex-posthog-dashboard", "codex"),
    ("codex-browserclaw", "codex"),
    ("browserclaw-claude-desktop-wrapper", "claude-desktop"),
];

/// Canonical analytics spelling for established wire aliases. Unknown values
/// are returned intact so the existing long-tail analytics contract survives.
#[must_use]
pub fn canonical_client_name(slug: &str) -> &str {
    CLIENT_ALIASES
        .iter()
        .find_map(|(alias, canonical)| (*alias == slug).then_some(*canonical))
        .unwrap_or(slug)
}

/// Recognizes a client product for display. Exact normalized names prevent a
/// worker nickname such as `cld-messaging` from being guessed to be Claude.
/// Display intentionally combines Claude products while analytics keeps them distinct.
#[must_use]
pub fn client_product_prefix(name: &str) -> Option<&'static str> {
    let slug = slugify_client_name(name)?;
    match canonical_client_name(unversioned_client_slug(&slug)) {
        "claude" | "claude-code" | "claude-desktop" | "claude-ai" => Some("claude"),
        "codex" => Some("codex"),
        "cursor" => Some("cursor"),
        "opencode" | "open-code" => Some("opencode"),
        "antigravity" => Some("antigravity"),
        "vscode" | "vs-code" | "visual-studio-code" | "vscode-insiders" | "vs-code-insiders" => {
            Some("vscode")
        }
        "zed" => Some("zed"),
        _ => None,
    }
}

/// Removes only trailing numeric version segments. Interior digits and all-digit
/// names remain intact, so unknown products retain their existing display spelling.
#[must_use]
pub(crate) fn unversioned_client_slug(slug: &str) -> &str {
    let mut end = slug.len();
    for segment in slug.rsplit('-') {
        if segment.is_empty() || !segment.bytes().all(|byte| byte.is_ascii_digit()) {
            break;
        }
        end = end.saturating_sub(segment.len() + 1);
    }
    if end == 0 { slug } else { &slug[..end] }
}
