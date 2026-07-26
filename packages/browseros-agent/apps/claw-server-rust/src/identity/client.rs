use crate::ids::ProfileId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
    pub title: Option<String>,
}

/// The profile fields needed to resolve an MCP client without coupling identity to storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileView {
    pub id: ProfileId,
    pub slug: String,
    pub name: String,
}

/// Profile and display identity resolved from MCP client metadata.
/// Per-conversation ownership uses `ConversationIdentity` even when this
/// resolves the same profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ClientIdentity {
    Profile {
        profile_id: ProfileId,
        slug: String,
        label: String,
    },
    Ephemeral {
        slug: String,
        label: String,
    },
}

impl ClientIdentity {
    #[must_use]
    pub fn resolve(client_info: &ClientInfo, profiles: &[ProfileView]) -> Self {
        let client_slug = slugify_client_name(&client_info.name).unwrap_or_else(|| "agent".into());
        if let Some(profile) = profiles.iter().find(|profile| {
            profile.slug == client_slug
                || names_match(profile.name.as_str(), client_info.name.as_str())
        }) {
            return Self::Profile {
                profile_id: profile.id.clone(),
                slug: profile.slug.clone(),
                label: profile.name.clone(),
            };
        }
        let label = clean_label(&client_info.name).unwrap_or_else(|| client_slug.clone());
        Self::Ephemeral {
            slug: client_slug,
            label,
        }
    }

    #[must_use]
    pub fn slug(&self) -> &str {
        match self {
            Self::Profile { slug, .. } | Self::Ephemeral { slug, .. } => slug,
        }
    }

    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Profile { label, .. } | Self::Ephemeral { label, .. } => label,
        }
    }

    #[must_use]
    pub fn profile_id(&self) -> Option<&ProfileId> {
        match self {
            Self::Profile { profile_id, .. } => Some(profile_id),
            Self::Ephemeral { .. } => None,
        }
    }
}

#[must_use]
pub fn slugify_client_name(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut pending_dash = false;
    for ch in raw.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(ch);
            if out.len() >= 64 {
                break;
            }
        } else {
            pending_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn clean_label(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn names_match(profile_name: &str, client_name: &str) -> bool {
    slugify_client_name(profile_name).as_deref() == slugify_client_name(client_name).as_deref()
}

#[cfg(test)]
mod tests {
    use super::{ClientIdentity, ClientInfo, ProfileView, slugify_client_name};
    use crate::ids::ProfileId;

    fn profile() -> ProfileView {
        ProfileView {
            id: ProfileId::new("p1"),
            name: "Finance Ops".to_string(),
            slug: "finance-ops".to_string(),
        }
    }

    #[test]
    fn slugify_matches_ts_identity_rules() {
        assert_eq!(
            slugify_client_name("Cowork . Finance ops").as_deref(),
            Some("cowork-finance-ops")
        );
        assert_eq!(slugify_client_name("..."), None);
    }

    #[test]
    fn resolves_profile_when_client_name_matches_slug() {
        let resolved = ClientIdentity::resolve(
            &ClientInfo {
                name: "finance ops".to_string(),
                version: "1".to_string(),
                title: None,
            },
            &[profile()],
        );
        match resolved {
            ClientIdentity::Profile {
                profile_id, slug, ..
            } => {
                assert_eq!(profile_id.as_str(), "p1");
                assert_eq!(slug, "finance-ops");
            }
            ClientIdentity::Ephemeral { .. } => panic!("expected profile"),
        }
    }

    #[test]
    fn falls_back_to_ephemeral_for_unknown_client() {
        let resolved = ClientIdentity::resolve(
            &ClientInfo {
                name: "Other".to_string(),
                version: "1".to_string(),
                title: None,
            },
            &[profile()],
        );
        match resolved {
            ClientIdentity::Ephemeral { slug, .. } => assert_eq!(slug, "other"),
            ClientIdentity::Profile { .. } => panic!("expected ephemeral"),
        }
    }

    #[test]
    fn empty_client_name_uses_agent_slug() {
        let resolved = ClientIdentity::resolve(
            &ClientInfo {
                name: "...".to_string(),
                version: "1".to_string(),
                title: None,
            },
            &[],
        );

        assert_eq!(resolved.slug(), "agent");
    }
}
