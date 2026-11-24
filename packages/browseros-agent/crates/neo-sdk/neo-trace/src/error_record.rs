//! Who caused a failure, as a record keeps it. The owner is the party whose behavior must change
//! to fix it. That is the whole point of claw-sdk's first goal: tell Neo bugs from script bugs,
//! host decisions and what the website did.

use crate::OperationId;
use serde::de::value::StringDeserializer;
use serde::{Deserialize, Deserializer, Serialize};

/// Whose behavior must change to fix a failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum ErrorOwner {
    /// Neo itself, the browser or the OS: a lost connection, a closed target, a limit.
    Neo,
    /// The agent's own code: a syntax error, `invalid_input`, `strict_mode_violation`,
    /// `no_match`, `method_unavailable`.
    Script,
    /// What the page did: a failed navigation, an element that never became actionable.
    Website,
    /// The host's rules, services or owner, including `owner_has_control`.
    Host,
}

impl ErrorOwner {
    /// The stored and wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Neo => "neo",
            Self::Script => "script",
            Self::Website => "website",
            Self::Host => "host",
        }
    }
}

/// Additive machine-readable detail; the primary error code remains backward compatible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    /// The native target's renderer crashed.
    TargetCrashed,
}

impl DiagnosticCode {
    /// The additive diagnostic's wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TargetCrashed => "target_crashed",
        }
    }

    /// Keep stored errors readable after rollback when a newer writer added a diagnostic.
    /// Unknown strings become absent; malformed types still fail deserialization. Known names
    /// use the enum's deserializer so typed and stored names stay in sync.
    pub fn deserialize_optional<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Self>, D::Error> {
        let name = Option::<String>::deserialize(deserializer)?;
        Ok(name.and_then(|name| Self::deserialize(StringDeserializer::<D::Error>::new(name)).ok()))
    }
}

/// A failure as it is stored: bounded on the writer before it reaches disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorRecord {
    /// Who must change to fix it.
    pub owner: ErrorOwner,
    /// A stable snake_case code, e.g. `owner_has_control`.
    pub code: String,
    /// Optional detail retained in full and summary views, without changing the primary code.
    #[serde(
        default,
        deserialize_with = "DiagnosticCode::deserialize_optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub diagnostic_code: Option<DiagnosticCode>,
    /// The human message.
    pub message: String,
    /// The host's own code when `code` is `host_error`, e.g. `needs_owner`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_code: Option<String>,
    /// True only when nothing was sent or the operation was a read.
    pub retryable: bool,
    /// On a run's error, the operation it came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<OperationId>,
    /// A `covered` pointer action's cover, as its error named it: its ref, role, name, kind and
    /// controls (neo-playwright's action/cover.rs). The cover's own snapshot lines (`block`),
    /// which only the script's error shows, are not kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::{DiagnosticCode, ErrorOwner};

    #[test]
    fn old_error_records_need_no_diagnostic_or_migration() -> Result<(), serde_json::Error> {
        let old: super::ErrorRecord = serde_json::from_str(
            r#"{"owner":"neo","code":"internal_error","message":"old failure","retryable":true}"#,
        )?;
        assert_eq!(old.diagnostic_code, None);
        assert!(serde_json::to_value(old)?.get("diagnosticCode").is_none());
        Ok(())
    }

    #[test]
    fn stored_error_records_ignore_unknown_diagnostics() -> Result<(), serde_json::Error> {
        for (diagnostic, expected) in [
            (Some("target_crashed"), Some(DiagnosticCode::TargetCrashed)),
            (None, None),
            (Some("future_diagnostic"), None),
        ] {
            let stored = serde_json::json!({
                "owner": "neo",
                "code": "internal_error",
                "diagnosticCode": diagnostic,
                "message": "stored failure",
                "retryable": false,
            });
            let record: super::ErrorRecord = serde_json::from_str(&stored.to_string())?;
            assert_eq!(record.owner, ErrorOwner::Neo);
            assert_eq!(record.code, "internal_error");
            assert_eq!(record.diagnostic_code, expected);
            assert_eq!(record.message, "stored failure");
            assert!(!record.retryable);
            assert_eq!(
                serde_json::to_value(record)?.get("diagnosticCode"),
                expected
                    .map(|code| serde_json::json!(code.as_str()))
                    .as_ref()
            );
        }
        Ok(())
    }

    #[test]
    fn stored_error_records_reject_malformed_diagnostics() {
        for diagnostic in [
            serde_json::json!(true),
            serde_json::json!(42),
            serde_json::json!([]),
            serde_json::json!({ "target_crashed": null }),
        ] {
            let stored = serde_json::json!({
                "owner": "neo",
                "code": "internal_error",
                "diagnosticCode": diagnostic,
                "message": "stored failure",
                "retryable": false,
            });
            assert!(
                serde_json::from_str::<super::ErrorRecord>(&stored.to_string()).is_err(),
                "malformed diagnostic should be rejected: {diagnostic}"
            );
        }
    }

    #[test]
    fn an_owner_serializes_as_its_stored_name() -> Result<(), serde_json::Error> {
        for owner in [
            ErrorOwner::Neo,
            ErrorOwner::Script,
            ErrorOwner::Website,
            ErrorOwner::Host,
        ] {
            assert_eq!(serde_json::to_value(owner)?, owner.as_str());
        }
        Ok(())
    }
}
