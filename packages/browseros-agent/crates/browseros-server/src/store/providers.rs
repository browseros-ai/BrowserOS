//! LLM provider rows (`kind = 'llm'`) in the `providers` table, and the one list of provider
//! types.
//!
//! A row is read three ways, as three types, so a handler can never return a key: `ProviderRow`
//! for any reader (flags, no secrets), `ProviderEdit` for a save, and `ProviderConfig` for an
//! outbound model call only. `ProviderDraft` is the settings form's unsaved provider, for the
//! connection test. Coding-agent rows share the table (`kind = 'acp'`) and the single default
//! row; `Agents` owns their writes, and the default pointer may point at a row of either kind.

use super::{Edit, Secret, SecretEdit, Store, StoreError};
use crate::turn::AgentKind;

pub struct Providers<'a>(
    #[expect(dead_code, reason = "read once chunk `settings` writes the queries")]
    pub(super)  &'a Store,
);

/// The one list of provider types. `model` matches on it exhaustively, and the contract's
/// enum is derived from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    Anthropic,
    Openai,
    Google,
    Openrouter,
    Fireworks,
    Azure,
    Bedrock,
    OpenaiCompatible,
    Ollama,
    Lmstudio,
    Moonshot,
    ChatgptPro,
}

impl ProviderKind {
    pub const ALL: [ProviderKind; 12] = [
        ProviderKind::Anthropic,
        ProviderKind::Openai,
        ProviderKind::Google,
        ProviderKind::Openrouter,
        ProviderKind::Fireworks,
        ProviderKind::Azure,
        ProviderKind::Bedrock,
        ProviderKind::OpenaiCompatible,
        ProviderKind::Ollama,
        ProviderKind::Lmstudio,
        ProviderKind::Moonshot,
        ProviderKind::ChatgptPro,
    ];

    /// The `type` column's spelling, which is also the contract's.
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Openai => "openai",
            ProviderKind::Google => "google",
            ProviderKind::Openrouter => "openrouter",
            ProviderKind::Fireworks => "fireworks",
            ProviderKind::Azure => "azure",
            ProviderKind::Bedrock => "bedrock",
            ProviderKind::OpenaiCompatible => "openai-compatible",
            ProviderKind::Ollama => "ollama",
            ProviderKind::Lmstudio => "lmstudio",
            ProviderKind::Moonshot => "moonshot",
            ProviderKind::ChatgptPro => "chatgpt-pro",
        }
    }

    /// `None` for a type this server no longer runs (`github-copilot`, `qwen-code`): such rows
    /// stay in the table and are left out of every list.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

/// Which kind of chat target a listed row is, with its type. Rows of a type this server does
/// not run are never listed, so the type is always a known one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Llm(ProviderKind),
    Acp(AgentKind),
}

/// A row as any reader may see it: `has_*` flags, never a secret.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderRow {
    pub id: String,
    pub kind: RowKind,
    pub name: String,
    pub model_id: Option<String>,
    pub reasoning_effort: Option<String>,
    pub reasoning_summary: Option<String>,
    pub is_default: bool,
    pub base_url: Option<String>,
    /// Sent with every model call; a value may hold the literal `{{conversationId}}`.
    pub headers: Option<Vec<(String, String)>>,
    pub supports_images: bool,
    pub context_window: Option<i64>,
    /// Stored and returned; no model call uses it (as today).
    pub temperature: f64,
    pub resource_name: Option<String>,
    pub region: Option<String>,
    pub working_directory: Option<String>,
    pub custom_config: Option<String>,
    pub has_api_key: bool,
    pub has_access_key_id: bool,
    pub has_secret_access_key: bool,
    pub has_session_token: bool,
    /// Epoch milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
}

/// A create-or-update of one LLM provider row. Required columns are plain values; a nullable
/// column is an `Edit` (absent keeps, `null` clears); a column with a default is an `Option`
/// (absent keeps on update, takes the default on insert). A save never moves the default row
/// and never rewrites `created_at` of an existing row.
#[derive(Clone)]
pub struct ProviderEdit {
    pub kind: ProviderKind,
    pub name: String,
    pub model_id: String,
    pub context_window: i64,
    pub base_url: Edit<String>,
    pub headers: Edit<Vec<(String, String)>>,
    pub supports_images: Option<bool>,
    pub temperature: Option<f64>,
    pub api_key: SecretEdit,
    pub access_key_id: SecretEdit,
    pub secret_access_key: SecretEdit,
    pub session_token: SecretEdit,
    pub resource_name: Edit<String>,
    pub region: Edit<String>,
    pub reasoning_effort: Edit<String>,
    pub reasoning_summary: Edit<String>,
    /// Honoured on insert only, so re-importing a provider keeps when it was first made.
    pub created_at: Option<i64>,
}

/// For outbound requests only. A different type from `ProviderRow`, so a handler cannot return a key.
pub struct ProviderConfig {
    pub id: String,
    pub kind: ProviderKind,
    pub model: String,
    pub base_url: Option<String>,
    pub headers: Vec<(String, String)>,
    pub context_window: usize,
    pub supports_images: bool,
    pub reasoning_effort: Option<String>,
    pub reasoning_summary: Option<String>,
    pub resource_name: Option<String>,
    pub credentials: Credentials,
}

/// What a provider call authenticates with.
pub enum Credentials {
    None,
    ApiKey(Secret),
    Aws {
        access_key_id: Secret,
        secret_access_key: Secret,
        session_token: Option<Secret>,
        region: String,
    },
    /// The ChatGPT sign-in: tokens live in `oauth_tokens`, read by `model::chatgpt`.
    SignIn,
}

impl Providers<'_> {
    /// Rows of both kinds (the extension filters), without secrets. Rows of a type this server
    /// no longer runs are left out.
    pub async fn list(&self) -> Result<Vec<ProviderRow>, StoreError> {
        Err(StoreError::NotImplemented("Providers::list"))
    }

    /// The selected row, of either kind, or `None` when there is none or it is of a type this
    /// server no longer runs.
    pub async fn default(&self) -> Result<Option<ProviderRow>, StoreError> {
        Err(StoreError::NotImplemented("Providers::default"))
    }

    /// Either kind. The lookup, the clear and the set are one transaction (the partial unique
    /// index allows one default row, so the old one is cleared first).
    pub async fn set_default(&self, _id: &str) -> Result<Option<ProviderRow>, StoreError> {
        Err(StoreError::NotImplemented("Providers::set_default"))
    }

    /// LLM rows only: an id that belongs to an `acp` row is refused. Written as INSERT … ON
    /// CONFLICT DO UPDATE, never REPLACE: a replace deletes the row first, and the retained
    /// `scheduled_jobs.provider_id` has ON DELETE SET NULL.
    pub async fn save(&self, _id: &str, _edit: ProviderEdit) -> Result<ProviderRow, StoreError> {
        Err(StoreError::NotImplemented("Providers::save"))
    }

    /// LLM rows only. Deleting one sets `provider_id` to NULL in any retained scheduled job
    /// (the old server's schema does this, and the old server deletes the same way).
    pub async fn delete(&self, _id: &str) -> Result<bool, StoreError> {
        Err(StoreError::NotImplemented("Providers::delete"))
    }

    /// The row with its credentials, for an outbound model call only. `None` id = the default row.
    pub async fn config(&self, _id: Option<&str>) -> Result<Option<ProviderConfig>, StoreError> {
        Err(StoreError::NotImplemented("Providers::config"))
    }
}

/// A provider as typed into the settings form, for `POST /test-provider`.
#[derive(Clone)]
pub struct ProviderDraft {
    pub kind: ProviderKind,
    pub model: String,
    pub base_url: Option<String>,
    pub headers: Vec<(String, String)>,
    pub resource_name: Option<String>,
    pub region: Option<String>,
    pub api_key: Option<Secret>,
    pub access_key_id: Option<Secret>,
    pub secret_access_key: Option<Secret>,
    pub session_token: Option<Secret>,
}

/// Why a draft cannot be tested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftError {
    /// A credential is missing and the saved row named for borrowing is of another type.
    TypeMismatch,
    /// A credential this provider type needs is missing; the name is the form field's.
    MissingCredential(&'static str),
}

impl std::fmt::Display for DraftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DraftError::TypeMismatch => {
                f.write_str("The saved provider is of another type, so its key cannot be used.")
            }
            DraftError::MissingCredential(field) => write!(f, "{field} is required."),
        }
    }
}

/// The context window a test call is resolved with when no saved row gives one; a one-line
/// prompt never comes near it.
const DRAFT_CONTEXT_WINDOW: usize = 128_000;

impl ProviderDraft {
    /// Fills a blank credential from `saved`. A saved row of another provider type lends
    /// nothing (an openrouter key is never sent as an azure one). Whenever any credential is
    /// borrowed, the saved row's base URL, resource name and region replace the typed ones: a
    /// saved secret is never sent to an address the caller chose. Pure.
    pub fn resolve(self, saved: Option<ProviderConfig>) -> Result<ProviderConfig, DraftError> {
        let mismatched = saved.as_ref().is_some_and(|saved| saved.kind != self.kind);
        let lender = saved.as_ref().filter(|saved| saved.kind == self.kind);
        let (saved_key, saved_aws) = match lender.map(|saved| &saved.credentials) {
            Some(Credentials::ApiKey(key)) => (Some(key.clone()), None),
            Some(Credentials::Aws {
                access_key_id,
                secret_access_key,
                session_token,
                region,
            }) => (
                None,
                Some((
                    access_key_id.clone(),
                    secret_access_key.clone(),
                    session_token.clone(),
                    region.clone(),
                )),
            ),
            _ => (None, None),
        };
        let mut borrowed = false;
        let mut take = |typed: Option<Secret>, stored: Option<Secret>| match typed {
            Some(secret) => Some(secret),
            None => {
                borrowed |= stored.is_some();
                stored
            }
        };
        let api_key = take(self.api_key, saved_key);
        let access_key_id = take(
            self.access_key_id,
            saved_aws.as_ref().map(|aws| aws.0.clone()),
        );
        let secret_access_key = take(
            self.secret_access_key,
            saved_aws.as_ref().map(|aws| aws.1.clone()),
        );
        let session_token = take(
            self.session_token,
            saved_aws.as_ref().and_then(|aws| aws.2.clone()),
        );

        let (base_url, resource_name, region) = match lender {
            Some(saved) if borrowed => (
                saved.base_url.clone(),
                saved.resource_name.clone(),
                saved_aws.as_ref().map(|aws| aws.3.clone()),
            ),
            _ => (self.base_url, self.resource_name, self.region),
        };
        let missing = |field: &'static str| {
            if mismatched {
                DraftError::TypeMismatch
            } else {
                DraftError::MissingCredential(field)
            }
        };
        let credentials = match self.kind {
            ProviderKind::ChatgptPro => Credentials::SignIn,
            ProviderKind::Bedrock => Credentials::Aws {
                access_key_id: access_key_id.ok_or_else(|| missing("accessKeyId"))?,
                secret_access_key: secret_access_key.ok_or_else(|| missing("secretAccessKey"))?,
                session_token,
                region: region.ok_or(DraftError::MissingCredential("region"))?,
            },
            ProviderKind::OpenaiCompatible | ProviderKind::Ollama | ProviderKind::Lmstudio => {
                api_key.map_or(Credentials::None, Credentials::ApiKey)
            }
            ProviderKind::Anthropic
            | ProviderKind::Openai
            | ProviderKind::Google
            | ProviderKind::Openrouter
            | ProviderKind::Fireworks
            | ProviderKind::Azure
            | ProviderKind::Moonshot => {
                Credentials::ApiKey(api_key.ok_or_else(|| missing("apiKey"))?)
            }
        };
        Ok(ProviderConfig {
            id: lender.map_or_else(|| "draft".to_owned(), |saved| saved.id.clone()),
            kind: self.kind,
            model: self.model,
            base_url,
            headers: self.headers,
            context_window: lender.map_or(DRAFT_CONTEXT_WINDOW, |saved| saved.context_window),
            supports_images: lender.is_some_and(|saved| saved.supports_images),
            reasoning_effort: lender.and_then(|saved| saved.reasoning_effort.clone()),
            reasoning_summary: lender.and_then(|saved| saved.reasoning_summary.clone()),
            resource_name,
            credentials,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(kind: ProviderKind) -> ProviderDraft {
        ProviderDraft {
            kind,
            model: "m".into(),
            base_url: Some("https://typed.example".into()),
            headers: Vec::new(),
            resource_name: None,
            region: None,
            api_key: None,
            access_key_id: None,
            secret_access_key: None,
            session_token: None,
        }
    }

    fn saved(kind: ProviderKind, credentials: Credentials) -> ProviderConfig {
        ProviderConfig {
            id: "saved".into(),
            kind,
            model: "m".into(),
            base_url: Some("https://saved.example".into()),
            headers: Vec::new(),
            context_window: 8_000,
            supports_images: true,
            reasoning_effort: None,
            reasoning_summary: None,
            resource_name: None,
            credentials,
        }
    }

    #[test]
    fn a_borrowed_key_goes_only_to_the_saved_address() -> Result<(), DraftError> {
        let config = draft(ProviderKind::Openrouter).resolve(Some(saved(
            ProviderKind::Openrouter,
            Credentials::ApiKey(Secret::new("sk-saved")),
        )))?;
        assert_eq!(config.base_url.as_deref(), Some("https://saved.example"));
        assert!(
            matches!(config.credentials, Credentials::ApiKey(key) if key.expose() == "sk-saved")
        );
        Ok(())
    }

    #[test]
    fn a_typed_key_keeps_the_typed_address() -> Result<(), DraftError> {
        let mut typed = draft(ProviderKind::Openrouter);
        typed.api_key = Some(Secret::new("sk-typed"));
        let config = typed.resolve(Some(saved(
            ProviderKind::Openrouter,
            Credentials::ApiKey(Secret::new("sk-saved")),
        )))?;
        assert_eq!(config.base_url.as_deref(), Some("https://typed.example"));
        Ok(())
    }

    #[test]
    fn a_row_of_another_type_lends_nothing() {
        let result = draft(ProviderKind::Azure).resolve(Some(saved(
            ProviderKind::Openrouter,
            Credentials::ApiKey(Secret::new("sk-saved")),
        )));
        assert_eq!(result.err(), Some(DraftError::TypeMismatch));
        assert_eq!(
            draft(ProviderKind::Azure).resolve(None).err(),
            Some(DraftError::MissingCredential("apiKey"))
        );
    }
}
