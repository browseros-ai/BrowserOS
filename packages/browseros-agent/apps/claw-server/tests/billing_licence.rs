//! Licence verification: which signed licences this server believes, and what it reads from
//! them. A licence that fails any check is not used, so the plan reads as none.

#[path = "support/fake_gateway.rs"]
mod fake_gateway;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use claw_server::services::billing::{
    BillingConfig, Status,
    licence::{LicenceError, MAX_CLOCK_SKEW_MS, SignedLicence, verify},
};
use ed25519_dalek::SigningKey;
use fake_gateway::{TEST_KID, sign, signing_key};
use serde_json::{Value, json};

const INSTALL: &str = "8f3e2b14-6c0a-4d9e-b5a7-1c2d3e4f5a60";
const NOW: i64 = 1_793_145_600_000;
const HOUR_MS: i64 = 60 * 60 * 1000;

fn config(kid: &str, release: bool) -> anyhow::Result<BillingConfig> {
    Ok(BillingConfig {
        gateway: url::Url::parse("http://127.0.0.1:1")?,
        keys: vec![(kid.to_string(), signing_key().verifying_key())],
        release,
    })
}

fn payload() -> Value {
    json!({
        "v": 3,
        "kid": TEST_KID,
        "installId": INSTALL,
        "app": "claw",
        "plan": "pro",
        "features": ["claw.ultrafast"],
        "status": "active",
        "periodEnd": NOW + 30 * 24 * HOUR_MS,
        "endsAt": null,
        "graceUntil": null,
        "issuedAt": NOW,
        "expiresAt": NOW + 72 * HOUR_MS,
    })
}

fn signed_with(key: &SigningKey, kid: &str, payload: &Value) -> SignedLicence {
    let (licence, signature) = sign(key, payload);
    SignedLicence {
        licence,
        signature,
        kid: kid.to_string(),
    }
}

fn signed(payload: &Value) -> SignedLicence {
    signed_with(&signing_key(), TEST_KID, payload)
}

fn with(field: &str, value: Value) -> Value {
    let mut payload = payload();
    payload[field] = value;
    payload
}

fn check(
    licence: &SignedLicence,
) -> Result<claw_server::services::billing::licence::Licence, LicenceError> {
    verify(
        licence,
        INSTALL,
        &config(TEST_KID, false).map_err(|_| LicenceError::Malformed)?,
        NOW,
        None,
    )
}

#[test]
fn a_good_licence_reads_its_plan() -> anyhow::Result<()> {
    let licence = check(&signed(&payload())).map_err(|error| anyhow::anyhow!("{error}"))?;
    assert_eq!(licence.status, Status::Active);
    assert_eq!(licence.plan.as_deref(), Some("pro"));
    assert_eq!(licence.features, ["claw.ultrafast"]);
    assert_eq!(licence.issued_at, NOW);
    assert_eq!(licence.expires_at, NOW + 72 * HOUR_MS);
    Ok(())
}

#[test]
fn a_tampered_byte_is_refused() -> anyhow::Result<()> {
    let mut licence = signed(&payload());
    let mut bytes = URL_SAFE_NO_PAD.decode(&licence.licence)?;
    let Some(position) = bytes.iter().position(|byte| *byte == b'a') else {
        anyhow::bail!("payload has an a");
    };
    bytes[position] = b'b';
    licence.licence = URL_SAFE_NO_PAD.encode(bytes);
    assert_eq!(check(&licence), Err(LicenceError::BadSignature));

    let mut signature = signed(&payload());
    signature.signature = URL_SAFE_NO_PAD.encode([0_u8; 64]);
    assert_eq!(check(&signature), Err(LicenceError::BadSignature));
    Ok(())
}

#[test]
fn an_unknown_kid_or_another_key_is_refused() {
    let licence = signed_with(&signing_key(), "test-2", &with("kid", json!("test-2")));
    assert_eq!(check(&licence), Err(LicenceError::UnknownKey));

    let stranger = SigningKey::from_bytes(&[9; 32]);
    assert_eq!(
        check(&signed_with(&stranger, TEST_KID, &payload())),
        Err(LicenceError::BadSignature)
    );
}

#[test]
fn a_payload_naming_another_kid_is_refused() {
    let licence = signed(&with("kid", json!("prod-2026-10")));
    assert_eq!(check(&licence), Err(LicenceError::WrongKid));
}

#[test]
fn a_release_build_refuses_dev_and_test_kids() -> anyhow::Result<()> {
    for kid in ["dev-1", "test-1"] {
        let licence = signed_with(&signing_key(), kid, &with("kid", json!(kid)));
        assert_eq!(
            verify(&licence, INSTALL, &config(kid, true)?, NOW, None),
            Err(LicenceError::UnknownKey),
            "{kid}"
        );
        assert!(verify(&licence, INSTALL, &config(kid, false)?, NOW, None).is_ok());
    }
    let licence = signed_with(&signing_key(), "prod-1", &with("kid", json!("prod-1")));
    assert!(verify(&licence, INSTALL, &config("prod-1", true)?, NOW, None).is_ok());
    Ok(())
}

#[test]
fn another_app_or_install_is_refused() {
    assert_eq!(
        check(&signed(&with("app", json!("other")))),
        Err(LicenceError::WrongApp)
    );
    assert_eq!(
        check(&signed(&with(
            "installId",
            json!("2a9b7c6d-1e0f-4a3b-8c5d-6e7f8091a2b3")
        ))),
        Err(LicenceError::WrongInstall)
    );
}

#[test]
fn only_payload_v3_is_read() {
    let mut v2 = payload();
    v2["v"] = json!(2);
    v2["emailHint"] = json!("s***@example.com");
    assert_eq!(check(&signed(&v2)), Err(LicenceError::WrongVersion));
    assert_eq!(
        check(&signed(&json!({"v": 3, "kid": TEST_KID}))),
        Err(LicenceError::Malformed)
    );
}

#[test]
fn unknown_fields_do_not_break_an_older_build() {
    assert!(check(&signed(&with("somethingNew", json!({"x": 1})))).is_ok());
}

#[test]
fn an_expired_licence_is_refused() {
    let mut expired = payload();
    expired["issuedAt"] = json!(NOW - 73 * HOUR_MS);
    expired["expiresAt"] = json!(NOW - HOUR_MS);
    assert_eq!(check(&signed(&expired)), Err(LicenceError::Expired));
    let mut edge = payload();
    edge["expiresAt"] = json!(NOW);
    assert_eq!(check(&signed(&edge)), Err(LicenceError::Expired));
}

#[test]
fn a_licence_issued_more_than_ten_minutes_ahead_is_refused() {
    let mut skewed = payload();
    skewed["issuedAt"] = json!(NOW + MAX_CLOCK_SKEW_MS);
    assert!(check(&signed(&skewed)).is_ok());
    skewed["issuedAt"] = json!(NOW + MAX_CLOCK_SKEW_MS + 1);
    assert_eq!(check(&signed(&skewed)), Err(LicenceError::FromTheFuture));
}

#[test]
fn a_licence_older_than_the_saved_one_is_refused() -> anyhow::Result<()> {
    let config = config(TEST_KID, false)?;
    let licence = signed(&payload());
    assert_eq!(
        verify(&licence, INSTALL, &config, NOW, Some(NOW + 1)),
        Err(LicenceError::Older)
    );
    assert!(verify(&licence, INSTALL, &config, NOW, Some(NOW)).is_ok());
    Ok(())
}

#[test]
fn unpaid_and_none_unlock_nothing() -> anyhow::Result<()> {
    let unpaid = check(&signed(&with("status", json!("unpaid"))))
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    assert_eq!(unpaid.status, Status::Unpaid);
    assert_eq!(unpaid.plan.as_deref(), Some("pro"));
    assert!(unpaid.features.is_empty());

    let unknown = check(&signed(&with("status", json!("trialing"))))
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    assert_eq!(unknown.status, Status::None);
    assert!(unknown.features.is_empty());
    Ok(())
}

#[test]
fn the_local_gateways_dev_key_is_the_one_compiled_in() -> anyhow::Result<()> {
    let Some(config) = BillingConfig::for_build(true) else {
        if cfg!(feature = "billing-staging") {
            return Ok(());
        }
        anyhow::bail!("dev mode has billing");
    };
    let Some(key) = config.trusted_key("dev-1") else {
        anyhow::bail!("dev mode trusts dev-1");
    };
    assert_eq!(
        URL_SAFE_NO_PAD.encode(key.as_bytes()),
        "tHSf_0xpRZQb_Zelg55jNqn1GmyWbV6cm2ZSi248AUk"
    );
    Ok(())
}
