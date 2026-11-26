//! Native feature gate for a server that ships ahead of the browser.
//!
//! claw-server is updated over the air independently of the native BrowserOS/Neo
//! browser, so a new server routinely drives an older browser. CDP surface that a
//! BrowserOS patch release added (custom methods, or new routing behavior behind an
//! existing parameter) must therefore be checked against the connected browser
//! before it is advertised to agents or sent.
//!
//! The key is the **native BrowserOS version**: the browser's compiled-in
//! `version_info::GetBrowserOSVersionNumber()` (from `resources/BROWSEROS_VERSION`), which
//! the browser writes into the server's launch `config.json` as
//! `instance.browseros_version`. It is deliberately not the OTA server version
//! (`CARGO_PKG_VERSION`), the Chromium version (two BrowserOS releases can share one
//! Chromium yet differ in our patches), a build checkout's stamp, a dev-mode flag, or the
//! legacy `browserContextId` directory labels. An identity that is missing, blank or
//! malformed is unknown, and unknown supports no feature in the table; ordinary
//! browsing never consults this gate.
//!
//! A host can also withhold a feature the browser implements, for example because the
//! user's plan does not include it ([`WithheldFeatures`]). Withholding goes through the same
//! gate, so discovery, typed calls and raw CDP all refuse it alike; only the error differs,
//! telling the agent it is a plan matter rather than a browser upgrade.
//!
//! Adding a native feature: land the Chromium patch, add a [`NativeFeature`] variant and
//! one `FEATURES` row whose minimum is the first `BROWSEROS_VERSION` that ships it, list
//! any raw CDP methods or (method, parameter) pairs it introduced, then tag the tools that
//! need it (`ToolMetadata::requires` in browseros-mcp). Hiding, refusal and raw guards
//! follow from the row; code that adds a new option to an existing operation calls
//! [`NativeCapabilities::require`] or [`NativeCapabilities::supports`] for that option only.

use crate::CoreError;
use serde_json::Value;
use std::fmt;

/// A native capability that only some BrowserOS releases implement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeFeature {
    /// PR #2908: saved-profile discovery/loading, window/tab identity sidecars, and
    /// profile-scoped routing for History and tab/window creation.
    ProfileTargeting,
    /// Saved-password inventory, explicit storage and acknowledged native filling, served
    /// by BrowserOS's own `Passwords` CDP domain.
    Credentials,
    /// Saved payment card metadata, card saves and native card filling, served by
    /// BrowserOS's own `Payments` CDP domain.
    PaymentCards,
}

impl NativeFeature {
    /// Every feature, for hosts that publish a flag per feature (the run runtime).
    pub const ALL: &'static [Self] = &[
        Self::ProfileTargeting,
        Self::Credentials,
        Self::PaymentCards,
    ];

    /// Stable camelCase key for machine-readable surfaces such as run's feature flags.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::ProfileTargeting => "profileTargeting",
            Self::Credentials => "credentials",
            Self::PaymentCards => "paymentCards",
        }
    }

    /// The feature named by [`Self::key`], for hosts that pass features by name.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|feature| feature.key() == key)
    }

    const fn bit(self) -> u8 {
        match self {
            Self::ProfileTargeting => 1,
            Self::Credentials => 1 << 1,
            Self::PaymentCards => 1 << 2,
        }
    }

    /// Why a host withheld this feature, in words an agent can act on.
    #[must_use]
    pub const fn withheld_reason(self) -> &'static str {
        match self {
            Self::ProfileTargeting => {
                "choosing a browser profile is part of ultrafast, which is off on this computer"
            }
            Self::Credentials => "saved passwords are not part of this computer's plan",
            Self::PaymentCards => "saved payment cards are not part of this computer's plan",
        }
    }

    /// The first native BrowserOS release that ships this feature.
    #[must_use]
    pub fn min_browseros(self) -> BrowserOsVersion {
        self.spec().min_browseros
    }

    fn spec(self) -> &'static FeatureSpec {
        FEATURES
            .iter()
            .find(|spec| spec.feature == self)
            .unwrap_or_else(|| unreachable!("every NativeFeature has a FEATURES row"))
    }
}

/// Native features a host withholds even though the browser implements them. Set per admitted
/// call (see `BrowserView::capture_withholding`), so a plan change applies from the next call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct WithheldFeatures(u8);

impl WithheldFeatures {
    #[must_use]
    pub const fn none() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn with(self, feature: NativeFeature) -> Self {
        Self(self.0 | feature.bit())
    }

    #[must_use]
    pub const fn contains(self, feature: NativeFeature) -> bool {
        self.0 & feature.bit() != 0
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Each withheld feature, in [`NativeFeature::ALL`] order.
    pub fn features(self) -> impl Iterator<Item = NativeFeature> {
        NativeFeature::ALL
            .iter()
            .copied()
            .filter(move |feature| self.contains(*feature))
    }
}

impl fmt::Display for NativeFeature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.spec().label)
    }
}

/// A native BrowserOS release version such as `0.51.1.3`, compared numerically part by
/// part (`0.51.1.10` is newer than `0.51.1.9`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BrowserOsVersion([u32; 4]);

impl BrowserOsVersion {
    #[must_use]
    pub const fn new(major: u32, minor: u32, build: u32, patch: u32) -> Self {
        Self([major, minor, build, patch])
    }

    /// Accepts two to four dot-separated decimal parts; missing trailing parts are zero,
    /// matching the three-part display form signing uses when the patch is zero.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let parts = raw.trim().split('.').collect::<Vec<_>>();
        if !(2..=4).contains(&parts.len()) {
            return None;
        }
        let mut version = [0; 4];
        for (slot, part) in version.iter_mut().zip(parts) {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            *slot = part.parse().ok()?;
        }
        Some(Self(version))
    }
}

impl fmt::Display for BrowserOsVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [major, minor, build, patch] = self.0;
        write!(f, "{major}.{minor}.{build}.{patch}")
    }
}

/// One row of the gate: the first native release with the feature and the raw CDP it owns.
struct FeatureSpec {
    feature: NativeFeature,
    label: &'static str,
    min_browseros: BrowserOsVersion,
    /// Raw methods refused while the feature is unsupported.
    raw_methods: &'static [&'static str],
    /// Raw (method, parameter) pairs refused while unsupported. Pairs, never bare parameter
    /// names: stock CDP takes `browserContextId` for off-the-record contexts on many methods,
    /// and those must keep working. Requests that omit the parameter are unaffected.
    raw_params: &'static [(&'static str, &'static str)],
}

/// The only place feature requirements live. Keep rows in release order.
const FEATURES: &[FeatureSpec] = &[
    FeatureSpec {
        feature: NativeFeature::ProfileTargeting,
        label: "profile targeting",
        min_browseros: BrowserOsVersion::new(0, 51, 1, 3),
        raw_methods: &["Browser.getProfiles", "Browser.loadProfile"],
        raw_params: &[
            ("History.search", "browserContextId"),
            ("History.getRecent", "browserContextId"),
            ("History.deleteUrl", "browserContextId"),
            ("History.deleteRange", "browserContextId"),
            // Declared before #2908, but older natives accepted and ignored it and opened in the
            // last-active window. Honoring the selected profile is the new behavior gated here.
            ("Browser.createTab", "browserContextId"),
            ("Browser.createWindow", "browserContextId"),
        ],
    },
    FeatureSpec {
        feature: NativeFeature::Credentials,
        label: "credentials",
        // 0.51.2.1 served these commands from upstream's Autofill domain; 0.51.2.7 moved
        // them, wire shapes unchanged, into BrowserOS's own Passwords domain.
        min_browseros: BrowserOsVersion::new(0, 51, 2, 7),
        raw_methods: &[
            "Passwords.getContext",
            "Passwords.listCredentials",
            "Passwords.addCredential",
            "Passwords.fillCredential",
            "Passwords.cancelOperation",
        ],
        raw_params: &[],
    },
    FeatureSpec {
        feature: NativeFeature::PaymentCards,
        label: "payment cards",
        min_browseros: BrowserOsVersion::new(0, 51, 2, 7),
        raw_methods: &[
            "Payments.getContext",
            "Payments.listCards",
            "Payments.addCard",
            "Payments.fillCard",
            "Payments.cancelOperation",
        ],
        raw_params: &[],
    },
];

/// The connected browser's native identity, fixed for one server process.
///
/// In production the browser that launches claw-server writes this identity into the launch
/// config, and the server is assumed to keep talking to that parent for its lifetime; a
/// different browser comes with a relaunched server and its own config. The default is
/// unknown, which supports no table feature.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NativeCapabilities {
    browseros: Option<BrowserOsVersion>,
    withheld: WithheldFeatures,
}

impl NativeCapabilities {
    /// Lenient by design: blank or malformed input yields unknown rather than an error, so a
    /// bad identity can only hide features, never stop the server.
    #[must_use]
    pub fn from_browseros_version(raw: Option<&str>) -> Self {
        Self {
            browseros: raw.and_then(BrowserOsVersion::parse),
            withheld: WithheldFeatures::none(),
        }
    }

    /// The same browser with `withheld` refused on top of what it lacks. The reported identity
    /// and native support stay as they are; see [`Self::implements`].
    #[must_use]
    pub const fn withholding(mut self, withheld: WithheldFeatures) -> Self {
        self.withheld = withheld;
        self
    }

    #[must_use]
    pub const fn withheld(&self) -> WithheldFeatures {
        self.withheld
    }

    /// Whether the browser itself implements `feature`, whatever the host withholds. Routing
    /// that only the browser decides (the Default profile on a capable native) reads this.
    #[must_use]
    pub fn implements(&self, feature: NativeFeature) -> bool {
        self.browseros
            .is_some_and(|version| version >= feature.spec().min_browseros)
    }

    #[must_use]
    pub const fn browseros_version(&self) -> Option<BrowserOsVersion> {
        self.browseros
    }

    #[must_use]
    pub fn supports(&self, feature: NativeFeature) -> bool {
        self.implements(feature) && !self.withheld.contains(feature)
    }

    /// Fails before an operation that needs `feature` is sent to the browser. A browser that
    /// lacks it reports the upgrade it needs first; withholding only narrows a capable one.
    pub fn require(&self, feature: NativeFeature) -> Result<(), CoreError> {
        if !self.implements(feature) {
            return Err(CoreError::NativeFeatureUnavailable {
                feature,
                required: feature.spec().min_browseros,
                reported: self.browseros,
            });
        }
        if self.withheld.contains(feature) {
            return Err(CoreError::FeatureWithheld { feature });
        }
        Ok(())
    }

    /// Refuses only raw CDP that a table row lists for an unsupported feature. Everything
    /// else is stock or pre-existing CDP and passes through; this is not a classifier for
    /// arbitrary methods.
    pub fn check_raw(&self, method: &str, params: &Value) -> Result<(), CoreError> {
        for spec in self.unsupported() {
            let gated_param = spec.raw_params.iter().any(|(gated, param)| {
                *gated == method && params.get(*param).is_some_and(|value| !value.is_null())
            });
            if spec.raw_methods.contains(&method) || gated_param {
                return self.require(spec.feature);
            }
        }
        Ok(())
    }

    /// [`Self::check_raw`] for pre-serialized parameters. Unparseable JSON passes through
    /// unchanged; the browser rejects it on its own.
    pub fn check_raw_json(&self, method: &str, params_json: &str) -> Result<(), CoreError> {
        let listed = self.unsupported().any(|spec| {
            spec.raw_methods.contains(&method)
                || spec.raw_params.iter().any(|(gated, _)| *gated == method)
        });
        if !listed {
            return Ok(());
        }
        match serde_json::from_str::<Value>(params_json) {
            Ok(params) => self.check_raw(method, &params),
            Err(_) => self.check_raw(method, &Value::Null),
        }
    }

    fn unsupported(&self) -> impl Iterator<Item = &'static FeatureSpec> + '_ {
        FEATURES.iter().filter(|spec| !self.supports(spec.feature))
    }
}

impl fmt::Display for NativeCapabilities {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.browseros {
            Some(version) => write!(f, "BrowserOS {version}"),
            None => f.write_str("unknown BrowserOS version"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn native(raw: &str) -> NativeCapabilities {
        NativeCapabilities::from_browseros_version(Some(raw))
    }

    #[test]
    fn profile_targeting_starts_at_native_0_51_1_3() {
        assert!(!native("0.51.1.2").supports(NativeFeature::ProfileTargeting));
        assert!(native("0.51.1.3").supports(NativeFeature::ProfileTargeting));
        assert!(native("0.52.0.0").supports(NativeFeature::ProfileTargeting));
    }

    #[test]
    fn a_withheld_feature_is_refused_as_a_plan_matter_on_a_capable_native() {
        let withheld = WithheldFeatures::none().with(NativeFeature::ProfileTargeting);
        let caps = native("0.53.0.1").withholding(withheld);
        assert!(caps.implements(NativeFeature::ProfileTargeting));
        assert!(!caps.supports(NativeFeature::ProfileTargeting));
        assert!(caps.supports(NativeFeature::Credentials));
        assert!(matches!(
            caps.require(NativeFeature::ProfileTargeting),
            Err(CoreError::FeatureWithheld {
                feature: NativeFeature::ProfileTargeting
            })
        ));
        let refused = caps
            .check_raw("Browser.getProfiles", &json!({}))
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default();
        assert!(
            refused.starts_with(
                "plan_feature_unavailable: choosing a browser profile is part of ultrafast"
            ),
            "{refused}"
        );
        assert!(
            caps.check_raw("History.getRecent", &json!({"browserContextId": "c"}))
                .is_err()
        );
        assert!(caps.check_raw("Browser.getTabs", &json!({})).is_ok());
        // An older native still reports the upgrade it needs, not the plan.
        assert!(matches!(
            native("0.51.1.2")
                .withholding(withheld)
                .require(NativeFeature::ProfileTargeting),
            Err(CoreError::NativeFeatureUnavailable { .. })
        ));
    }

    #[test]
    fn versions_compare_numerically_not_lexically() {
        assert!(native("0.51.1.10").supports(NativeFeature::ProfileTargeting));
        assert!(native("0.51.10").supports(NativeFeature::ProfileTargeting));
        assert!(!native("0.9.9.9").supports(NativeFeature::ProfileTargeting));
        assert_eq!(
            BrowserOsVersion::parse("0.51.1"),
            Some(BrowserOsVersion::new(0, 51, 1, 0))
        );
    }

    #[test]
    fn missing_blank_or_malformed_identity_is_unknown() {
        for raw in [
            None,
            Some(""),
            Some("  "),
            Some("abc"),
            Some("0.51.x.3"),
            Some("0.51.1.3.1"),
            Some("51"),
            Some("0.51.-1.3"),
            Some("0..1.3"),
        ] {
            let caps = NativeCapabilities::from_browseros_version(raw);
            assert_eq!(caps.browseros_version(), None, "{raw:?}");
            assert!(!caps.supports(NativeFeature::ProfileTargeting), "{raw:?}");
        }
    }

    #[test]
    fn refusal_names_requirement_and_reported_identity() {
        let old = native("0.51.1.2").require(NativeFeature::ProfileTargeting);
        let message = old.err().map(|err| err.to_string()).unwrap_or_default();
        assert!(
            message.starts_with("native_feature_unavailable: "),
            "{message}"
        );
        assert!(message.contains("profile targeting needs BrowserOS 0.51.1.3 or newer"));
        assert!(message.contains("0.51.1.2"), "{message}");

        let unknown = NativeCapabilities::default().require(NativeFeature::ProfileTargeting);
        let message = unknown.err().map(|err| err.to_string()).unwrap_or_default();
        assert!(message.contains("version is unknown"), "{message}");
        assert!(
            native("0.51.1.3")
                .require(NativeFeature::ProfileTargeting)
                .is_ok()
        );
    }

    #[test]
    fn raw_guard_refuses_listed_methods_and_parameter_pairs_only() {
        let old = native("0.51.1.2");
        for (method, params) in [
            ("Browser.getProfiles", json!({})),
            ("Browser.loadProfile", json!({"browserProfileId":"p"})),
            (
                "History.search",
                json!({"query":"x","browserContextId":"c"}),
            ),
            ("History.getRecent", json!({"browserContextId":"c"})),
            (
                "History.deleteUrl",
                json!({"url":"u","browserContextId":"c"}),
            ),
            (
                "History.deleteRange",
                json!({"startTime":0,"endTime":1,"browserContextId":"c"}),
            ),
            (
                "Browser.createTab",
                json!({"url":"about:blank","browserContextId":"c"}),
            ),
            ("Browser.createWindow", json!({"browserContextId":"c"})),
        ] {
            assert!(old.check_raw(method, &params).is_err(), "{method} {params}");
        }
        for (method, params) in [
            ("History.getRecent", json!({"maxResults":5})),
            ("Browser.createTab", json!({"url":"about:blank"})),
            ("Browser.createWindow", json!({})),
            (
                "Target.createTarget",
                json!({"url":"about:blank","browserContextId":"otr"}),
            ),
            ("Storage.getCookies", json!({"browserContextId":"otr"})),
            ("Browser.getTabs", json!({})),
            ("Browser.notARealMethod", json!({"browserContextId":"c"})),
        ] {
            assert!(old.check_raw(method, &params).is_ok(), "{method} {params}");
        }
        let current = native("0.51.1.3");
        assert!(current.check_raw("Browser.getProfiles", &json!({})).is_ok());
        assert!(
            current
                .check_raw(
                    "Browser.createTab",
                    &json!({"url":"u","browserContextId":"c"})
                )
                .is_ok()
        );
    }

    #[test]
    fn raw_json_guard_matches_value_guard() {
        let unknown = NativeCapabilities::default();
        assert!(unknown.check_raw_json("Browser.getProfiles", "{}").is_err());
        assert!(
            unknown
                .check_raw_json("History.getRecent", r#"{"browserContextId":"c"}"#)
                .is_err()
        );
        assert!(
            unknown
                .check_raw_json("History.getRecent", r#"{"maxResults":1}"#)
                .is_ok()
        );
        assert!(
            unknown
                .check_raw_json("Target.createTarget", r#"{"browserContextId":"otr"}"#)
                .is_ok()
        );
    }
}
