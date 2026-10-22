//! Transport-independent password operations. Native owns secret storage, site
//! matching and opaque handles; adapters receive metadata and acknowledged fills.
//! Requests intentionally have no Debug implementation because they may hold secrets.
use crate::{BrowserProfileId, CoreError};
use browseros_cdp::CdpError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CredentialInfo {
    pub credential_id: String,
    pub username: String,
    pub site: String,
    pub stores: Vec<String>,
}

#[derive(Default)]
pub struct CredentialListOptions {
    pub query: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

pub enum CredentialSource {
    Saved { credential_id: String },
    Provided { username: String, password: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CredentialInventory {
    pub browser_profile_id: BrowserProfileId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    pub credentials: Vec<CredentialInfo>,
    pub total: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CredentialAddOutcome {
    pub browser_profile_id: BrowserProfileId,
    pub origin: String,
    pub status: String,
    pub credential: CredentialInfo,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CredentialFillOutcome {
    pub browser_profile_id: BrowserProfileId,
    pub origin: String,
    pub status: String,
    pub filled_fields: Vec<String>,
}

/// A fixed diagnostic vocabulary: native/transport exception text is never
/// reflected into a password-bearing tool result or server diagnostic.
#[derive(Clone, Debug, Serialize)]
pub struct CredentialFailure {
    pub code: &'static str,
    pub message: &'static str,
}
impl std::fmt::Display for CredentialFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message)
    }
}
impl std::error::Error for CredentialFailure {}
impl CredentialFailure {
    #[must_use]
    pub fn new(code: &str) -> Self {
        let (code, message) = match code {
            "not_supported" => (
                "not_supported",
                "Credentials require a newer native browser.",
            ),
            "page_out_of_scope" => (
                "page_out_of_scope",
                "This page is outside the selected browser target.",
            ),
            "page_changed" => (
                "page_changed",
                "The page changed. Find the current page and retry.",
            ),
            "credential_not_found" => (
                "credential_not_found",
                "Credential not found for this site and profile. List accounts for this page.",
            ),
            "credential_conflict" => (
                "credential_conflict",
                "This site and username already have a different saved password.",
            ),
            "no_login_form" => (
                "no_login_form",
                "No unambiguous main-frame login form was found.",
            ),
            "no_fillable_fields" => (
                "no_fillable_fields",
                "No login fields acknowledged filling.",
            ),
            "operation_not_allowed" => (
                "operation_not_allowed",
                "The selected profile, page or native password policy does not allow this operation.",
            ),
            "reauth_required" => (
                "reauth_required",
                "Native device reauthentication is required. Complete filling in the browser.",
            ),
            "store_unavailable" => (
                "store_unavailable",
                "The native password store is unavailable.",
            ),
            "stale_cursor" => (
                "stale_cursor",
                "The credential inventory or listing context changed. Restart listing without cursor.",
            ),
            "busy" => (
                "busy",
                "Another credential operation is in progress. Retry after it completes.",
            ),
            "timeout" => ("timeout", "The credential operation timed out."),
            "operation_cancelled" => (
                "operation_cancelled",
                "The credential operation was cancelled.",
            ),
            "save_outcome_unknown" => (
                "save_outcome_unknown",
                "The save outcome is unknown. List accounts before retrying.",
            ),
            _ => ("store_error", "The native credential operation failed."),
        };
        Self { code, message }
    }
}
impl From<CoreError> for CredentialFailure {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::NativeFeatureUnavailable { .. } => Self::new("not_supported"),
            CoreError::Target(_) => Self::new("page_out_of_scope"),
            CoreError::Cdp(CdpError::Protocol { message, .. }) => Self::new(
                message
                    .strip_prefix("credentials:")
                    .unwrap_or("store_error"),
            ),
            _ => Self::new("store_error"),
        }
    }
}
