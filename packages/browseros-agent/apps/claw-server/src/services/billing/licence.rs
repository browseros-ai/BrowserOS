//! Decoding and verifying the billing gateway's signed licence (payload v3).
//!
//! Pure: no clock, no disk, no network. A licence that fails any check is simply not used, so
//! editing a saved licence can only make the plan read as none.

use super::config::BillingConfig;
use base64::{
    Engine as _, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use ed25519_dalek::Signature;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The app every licence this server accepts must be issued for.
pub const APP: &str = "claw";

/// The only payload version this build reads.
pub const VERSION: i64 = 3;

/// How far ahead of this computer's clock a licence may claim to have been issued.
pub const MAX_CLOCK_SKEW_MS: i64 = 10 * 60 * 1000;

const BASE64URL: GeneralPurpose = GeneralPurpose::new(
    &alphabet::URL_SAFE,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// What the gateway returns and what is saved to disk, unchanged, so it can be checked again
/// on every start: the base64url payload, its Ed25519 signature and the signing key's id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedLicence {
    pub licence: String,
    pub signature: String,
    pub kid: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    None,
    Active,
    Ending,
    PaymentFailed,
    Unpaid,
}

/// A verified licence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Licence {
    pub plan: Option<String>,
    /// Empty whenever the status is none or unpaid.
    pub features: Vec<String>,
    pub status: Status,
    pub period_end: Option<i64>,
    pub ends_at: Option<i64>,
    pub grace_until: Option<i64>,
    pub issued_at: i64,
    pub expires_at: i64,
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum LicenceError {
    #[error("signed with a key this build does not know")]
    UnknownKey,
    #[error("not base64url")]
    NotBase64,
    #[error("bad signature")]
    BadSignature,
    #[error("unreadable payload")]
    Malformed,
    #[error("unsupported payload version")]
    WrongVersion,
    #[error("payload names a different key")]
    WrongKid,
    #[error("issued for another app")]
    WrongApp,
    #[error("issued for another install")]
    WrongInstall,
    #[error("issued in the future")]
    FromTheFuture,
    #[error("expired")]
    Expired,
    #[error("older than the saved licence")]
    Older,
}

#[derive(Deserialize)]
struct Version {
    v: i64,
}

/// Unknown fields are ignored, so a newer gateway can add some without breaking this build.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Payload {
    kid: String,
    install_id: String,
    app: String,
    plan: Option<String>,
    #[serde(default)]
    features: Vec<String>,
    status: String,
    #[serde(default)]
    period_end: Option<i64>,
    #[serde(default)]
    ends_at: Option<i64>,
    #[serde(default)]
    grace_until: Option<i64>,
    issued_at: i64,
    expires_at: i64,
}

/// Verifies `signed` for this install.
///
/// `stored_issued_at` is the saved licence's issue time: a licence never steps back to an older
/// one, which keeps a replayed reply from undoing a newer state.
pub fn verify(
    signed: &SignedLicence,
    install_id: &str,
    config: &BillingConfig,
    now_ms: i64,
    stored_issued_at: Option<i64>,
) -> Result<Licence, LicenceError> {
    let key = config
        .trusted_key(&signed.kid)
        .ok_or(LicenceError::UnknownKey)?;
    let bytes = decode_base64url(&signed.licence).ok_or(LicenceError::NotBase64)?;
    let signature = decode_base64url(&signed.signature).ok_or(LicenceError::NotBase64)?;
    let signature = Signature::from_slice(&signature).map_err(|_| LicenceError::BadSignature)?;
    key.verify_strict(&bytes, &signature)
        .map_err(|_| LicenceError::BadSignature)?;

    let version: Version = serde_json::from_slice(&bytes).map_err(|_| LicenceError::Malformed)?;
    if version.v != VERSION {
        return Err(LicenceError::WrongVersion);
    }
    let payload: Payload = serde_json::from_slice(&bytes).map_err(|_| LicenceError::Malformed)?;
    if payload.kid != signed.kid {
        return Err(LicenceError::WrongKid);
    }
    if payload.app != APP {
        return Err(LicenceError::WrongApp);
    }
    if payload.install_id != install_id {
        return Err(LicenceError::WrongInstall);
    }
    if payload.issued_at > now_ms.saturating_add(MAX_CLOCK_SKEW_MS) {
        return Err(LicenceError::FromTheFuture);
    }
    if now_ms >= payload.expires_at {
        return Err(LicenceError::Expired);
    }
    if stored_issued_at.is_some_and(|stored| payload.issued_at < stored) {
        return Err(LicenceError::Older);
    }

    let status = parse_status(&payload.status);
    let features = match status {
        Status::Active | Status::Ending | Status::PaymentFailed => payload.features,
        Status::None | Status::Unpaid => Vec::new(),
    };
    Ok(Licence {
        plan: payload.plan.filter(|_| status != Status::None),
        features,
        status,
        period_end: payload.period_end,
        ends_at: payload.ends_at,
        grace_until: payload.grace_until,
        issued_at: payload.issued_at,
        expires_at: payload.expires_at,
    })
}

/// A status this build does not know unlocks nothing.
fn parse_status(raw: &str) -> Status {
    match raw {
        "active" => Status::Active,
        "ending" => Status::Ending,
        "paymentFailed" => Status::PaymentFailed,
        "unpaid" => Status::Unpaid,
        _ => Status::None,
    }
}

/// base64url, with or without padding.
#[must_use]
pub fn decode_base64url(encoded: &str) -> Option<Vec<u8>> {
    BASE64URL.decode(encoded.trim()).ok()
}
