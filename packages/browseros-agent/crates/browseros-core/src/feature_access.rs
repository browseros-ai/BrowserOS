//! Request-scoped feature access: live grants narrow immutable browser compatibility.
//! Hosts resolve grants once at admission. Keeping this value in BrowserView makes typed
//! calls, raw CDP and every primitive of a run agree even when access changes mid-call.
use crate::{CoreError, NativeCapabilities, NativeFeature, capabilities};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Product access supplied by the host. Today the server persists a local override; a future
/// authenticated entitlement resolver can supply the same grants without changing consumers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureFlags {
    pub pro: bool,
}

/// Immutable effective access for one discovery or execution request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FeatureAccess {
    native: NativeCapabilities,
    flags: FeatureFlags,
}

impl FeatureAccess {
    #[must_use]
    pub const fn new(native: NativeCapabilities, flags: FeatureFlags) -> Self {
        Self { native, flags }
    }

    #[must_use]
    pub const fn flags(self) -> FeatureFlags {
        self.flags
    }

    #[must_use]
    pub fn supports(self, feature: NativeFeature) -> bool {
        self.require(feature).is_ok()
    }

    pub fn require(self, feature: NativeFeature) -> Result<(), CoreError> {
        // A grant cannot manufacture native support; retain the actionable upgrade error.
        self.native.require(feature)?;
        // Paid-feature policy lives here, not in the native protocol compatibility table.
        if matches!(feature, NativeFeature::ProfileTargeting) && !self.flags.pro {
            return Err(CoreError::ProFeatureDisabled { feature });
        }
        Ok(())
    }

    pub fn check_raw(self, method: &str, params: &Value) -> Result<(), CoreError> {
        capabilities::check_raw(method, params, |feature| self.require(feature))
    }

    pub fn check_raw_json(self, method: &str, params: &str) -> Result<(), CoreError> {
        capabilities::check_raw_json(method, params, |feature| self.require(feature))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pro_narrows_compatibility_and_leaves_credentials_independent() {
        let native = NativeCapabilities::from_browseros_version(Some("0.51.2.1"));
        assert!(native.supports(NativeFeature::ProfileTargeting));
        let free = FeatureAccess::new(native, FeatureFlags::default());
        assert!(matches!(
            free.require(NativeFeature::ProfileTargeting),
            Err(CoreError::ProFeatureDisabled { .. })
        ));
        assert!(free.supports(NativeFeature::Credentials));
        for (method, params) in [
            ("Browser.getProfiles", json!({})),
            ("Browser.loadProfile", json!({"browserProfileId":"p"})),
            ("History.getRecent", json!({"browserContextId":"c"})),
            ("Browser.createTab", json!({"browserContextId":"c"})),
            ("Browser.createWindow", json!({"browserContextId":"c"})),
        ] {
            assert!(free.check_raw(method, &params).is_err(), "{method}");
            assert!(
                free.check_raw_json(method, &params.to_string()).is_err(),
                "{method}"
            );
        }
        assert!(free.check_raw("Browser.getTabs", &json!({})).is_ok());
        assert!(
            free.check_raw("Target.createTarget", &json!({"browserContextId":"otr"}))
                .is_ok()
        );
        assert!(
            free.check_raw_json("Browser.getProfiles", "invalid")
                .is_err()
        );
        let pro = FeatureAccess::new(native, FeatureFlags { pro: true });
        assert!(pro.supports(NativeFeature::ProfileTargeting));
        for raw in [None, Some("0.51.1.2")] {
            let access = FeatureAccess::new(
                NativeCapabilities::from_browseros_version(raw),
                FeatureFlags { pro: true },
            );
            assert!(matches!(
                access.require(NativeFeature::ProfileTargeting),
                Err(CoreError::NativeFeatureUnavailable { .. })
            ));
        }
    }
}
