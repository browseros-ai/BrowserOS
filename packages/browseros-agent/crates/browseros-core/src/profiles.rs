//! Native catalog identities and pure selection. Catalog discovery and loading are separate
//! from host-owned task binding; resolving a selector never loads or creates a profile.

use crate::{BrowserContextId, BrowserProfileId, TargetError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserProfileInfo {
    pub browser_profile_id: BrowserProfileId,
    pub name: String,
    pub directory: String,
    pub loaded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_context_id: Option<BrowserContextId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadProfileOutcome {
    pub profile: BrowserProfileInfo,
    /// True only when this request initiated the native load; joining an existing load is false.
    pub loaded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileSelector {
    Id(BrowserProfileId),
    Name(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileCandidate {
    pub browser_profile_id: BrowserProfileId,
    pub name: String,
    pub directory: String,
}

/// A sidecar parsed from native JSON, independently of the generated protocol snapshot.
/// The legacy browserContextId directory label is deliberately absent from this type.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileMeta {
    pub browser_profile_id: Option<BrowserProfileId>,
    pub live_browser_context_id: Option<BrowserContextId>,
}

pub fn resolve_profile(
    profiles: &[BrowserProfileInfo],
    selector: &ProfileSelector,
) -> Result<BrowserProfileInfo, TargetError> {
    let matches = profiles
        .iter()
        .filter(|profile| match selector {
            ProfileSelector::Id(id) => &profile.browser_profile_id == id,
            ProfileSelector::Name(name) => &profile.name == name,
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [profile] => Ok((*profile).clone()),
        [] => Err(TargetError::ProfileNotFound(match selector {
            ProfileSelector::Id(id) => id.to_string(),
            ProfileSelector::Name(name) => name.clone(),
        })),
        _ => Err(TargetError::ProfileAmbiguous {
            name: match selector {
                ProfileSelector::Id(id) => id.to_string(),
                ProfileSelector::Name(name) => name.clone(),
            },
            candidates: matches
                .into_iter()
                .map(|profile| ProfileCandidate {
                    browser_profile_id: profile.browser_profile_id.clone(),
                    name: profile.name.clone(),
                    directory: profile.directory.clone(),
                })
                .collect(),
        }),
    }
}

/// Directory of the browser's original profile. The native catalog reports each profile's
/// directory as its path base name, which survives renames; display names can be renamed or
/// duplicated (another profile may be named "Default") and never select the default.
pub const DEFAULT_PROFILE_DIRECTORY: &str = "Default";

/// The profile a task uses when it has no explicit browser target: for now the original
/// Chromium profile. This is the one place the default is chosen, so a later user setting
/// replaces this rule rather than its callers. List order, last-used and focus never count.
#[must_use]
pub fn default_profile(profiles: &[BrowserProfileInfo]) -> Option<&BrowserProfileInfo> {
    profiles
        .iter()
        .find(|profile| profile.directory == DEFAULT_PROFILE_DIRECTORY)
}
