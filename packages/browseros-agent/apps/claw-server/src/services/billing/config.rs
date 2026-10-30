//! Which billing gateway this build talks to and which licence signing keys it trusts.
//!
//! Chosen when the server is built, never read from the environment, so a shipped build cannot
//! be pointed at another gateway or made to trust another key.

use super::licence::decode_base64url;
use ed25519_dalek::VerifyingKey;
use url::Url;

/// A gateway address and the `(kid, public key)` pairs it signs licences with.
type Gateway = (&'static str, &'static [(&'static str, &'static str)]);

/// The local billing gateway's compose stack, used in dev mode.
const LOCAL_GATEWAY: &str = "http://localhost:8787";

/// The public half of the local billing gateway's dev-only signing key (raw 32 bytes, base64url).
const DEV_KEYS: &[(&str, &str)] = &[("dev-1", "tHSf_0xpRZQb_Zelg55jNqn1GmyWbV6cm2ZSi248AUk")];

/// The staging gateway, for builds with the `billing-staging` feature. Billing stays off in such
/// a build while this is `None`.
const STAGING_GATEWAY: Option<Gateway> = None;

/// The production gateway. `None` until launch, so release builds are Free.
const RELEASE_GATEWAY: Option<Gateway> = None;

/// Key ids that only local and test gateways sign with.
const NON_RELEASE_KID_PREFIXES: &[&str] = &["dev-", "test-"];

#[derive(Clone, Debug)]
pub struct BillingConfig {
    pub gateway: Url,
    pub keys: Vec<(String, VerifyingKey)>,
    /// Staging and release builds: licences signed with a dev or test key are refused.
    pub release: bool,
}

impl BillingConfig {
    /// The gateway this build uses, or `None` when it has no billing.
    #[must_use]
    pub fn for_build(dev_mode: bool) -> Option<Self> {
        if cfg!(feature = "billing-staging") {
            return STAGING_GATEWAY.and_then(|gateway| from_constants(gateway, true));
        }
        if dev_mode {
            from_constants((LOCAL_GATEWAY, DEV_KEYS), false)
        } else {
            RELEASE_GATEWAY.and_then(|gateway| from_constants(gateway, true))
        }
    }

    /// The key a licence signed as `kid` must verify against, if this build trusts it.
    #[must_use]
    pub fn trusted_key(&self, kid: &str) -> Option<&VerifyingKey> {
        if self.release && is_non_release_kid(kid) {
            return None;
        }
        self.keys
            .iter()
            .find(|(trusted, _)| trusted == kid)
            .map(|(_, key)| key)
    }
}

#[must_use]
pub fn is_non_release_kid(kid: &str) -> bool {
    NON_RELEASE_KID_PREFIXES
        .iter()
        .any(|prefix| kid.starts_with(prefix))
}

/// A raw 32-byte Ed25519 public key in base64url.
#[must_use]
pub fn parse_public_key(encoded: &str) -> Option<VerifyingKey> {
    let bytes: [u8; 32] = decode_base64url(encoded)?.try_into().ok()?;
    VerifyingKey::from_bytes(&bytes).ok()
}

fn from_constants((gateway, keys): Gateway, release: bool) -> Option<BillingConfig> {
    let gateway = Url::parse(gateway).ok()?;
    let keys = keys
        .iter()
        .filter(|(kid, _)| !(release && is_non_release_kid(kid)))
        .filter_map(|(kid, key)| Some(((*kid).to_string(), parse_public_key(key)?)))
        .collect::<Vec<_>>();
    if keys.is_empty() {
        tracing::error!("billing has no usable licence key; billing is off");
        return None;
    }
    Some(BillingConfig {
        gateway,
        keys,
        release,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_compiled_in_key_parses() {
        for (kid, key) in DEV_KEYS {
            assert!(parse_public_key(key).is_some(), "{kid}");
        }
    }

    #[cfg(not(feature = "billing-staging"))]
    #[test]
    fn dev_mode_uses_the_local_gateway_and_trusts_only_dev_1() {
        let Some(config) = BillingConfig::for_build(true) else {
            panic!("dev mode has billing");
        };
        assert_eq!(config.gateway.as_str(), "http://localhost:8787/");
        assert!(!config.release);
        assert!(config.trusted_key("dev-1").is_some());
        assert!(config.trusted_key("prod-2026-10").is_none());
    }

    #[cfg(not(feature = "billing-staging"))]
    #[test]
    fn release_builds_have_no_billing_until_launch() {
        assert!(BillingConfig::for_build(false).is_none());
    }

    #[test]
    fn a_release_config_never_trusts_dev_or_test_keys() {
        let key = parse_public_key(DEV_KEYS[0].1);
        let Some(key) = key else {
            panic!("dev key parses");
        };
        let config = BillingConfig {
            gateway: Url::parse(LOCAL_GATEWAY).unwrap_or_else(|error| panic!("{error}")),
            keys: vec![
                ("dev-1".to_string(), key),
                ("test-1".to_string(), key),
                ("prod-1".to_string(), key),
            ],
            release: true,
        };
        assert!(config.trusted_key("dev-1").is_none());
        assert!(config.trusted_key("test-1").is_none());
        assert!(config.trusted_key("prod-1").is_some());
        assert!(from_constants((LOCAL_GATEWAY, DEV_KEYS), true).is_none());
    }
}
