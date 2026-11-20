//! What Neo's telemetry is configured with, read once from the environment at startup.

use crate::{OtlpConfig, SiteDetail, TelemetryError};

/// Neo's telemetry settings, read once at startup and never changed while Neo runs (they are
/// not on `/v1/settings`). [`Default`] is everything off: no URL detail on spans and no push.
/// The Prometheus scrape needs no setting: it is served whenever the host API listener is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TelemetryConfig {
    /// `CLAW_TELEMETRY_SITE_DETAIL`: how much of a page's address an op span carries.
    pub site_detail: SiteDetail,
    /// The standard `OTEL_*` variables: the push door, the resource and the sampler.
    pub otlp: OtlpConfig,
}

impl TelemetryConfig {
    /// Reads every telemetry variable through `lookup` (the process environment, or a test's
    /// map), so a malformed value fails startup instead of quietly running with a default. An
    /// unset or blank variable keeps its default.
    pub fn from_env(lookup: &dyn Fn(&str) -> Option<String>) -> Result<Self, TelemetryError> {
        let site_detail = match lookup(SiteDetail::ENV) {
            Some(value) if !value.trim().is_empty() => SiteDetail::parse(value.trim())?,
            Some(_) | None => SiteDetail::default(),
        };
        Ok(Self {
            site_detail,
            otlp: OtlpConfig::from_env(lookup)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::error::Error;

    use super::*;

    fn read(vars: &[(&str, &str)]) -> Result<TelemetryConfig, TelemetryError> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        TelemetryConfig::from_env(&|name| vars.get(name).cloned())
    }

    #[test]
    fn nothing_set_is_everything_off() -> Result<(), Box<dyn Error>> {
        assert_eq!(
            read(&[])?,
            TelemetryConfig::default(),
            "an empty environment must start telemetry as the default: no URL detail, no push"
        );
        assert_eq!(
            read(&[(SiteDetail::ENV, "  ")])?.site_detail,
            SiteDetail::None,
            "a blank value is unset, as every other Neo variable reads it"
        );
        Ok(())
    }

    #[test]
    fn site_detail_reads_each_of_its_three_values() -> Result<(), Box<dyn Error>> {
        for (value, detail) in [
            ("none", SiteDetail::None),
            ("host", SiteDetail::Host),
            ("url", SiteDetail::Url),
            (" url ", SiteDetail::Url),
        ] {
            assert_eq!(
                read(&[(SiteDetail::ENV, value)])?.site_detail,
                detail,
                "`{value}` names a level of the dial"
            );
        }
        Ok(())
    }

    #[test]
    fn site_detail_refuses_anything_else() {
        for value in ["URL", "full", "hosts", "domain", "0"] {
            let error = read(&[(SiteDetail::ENV, value)]);
            assert!(
                matches!(
                    error,
                    Err(TelemetryError::InvalidEnv {
                        name: SiteDetail::ENV,
                        ..
                    })
                ),
                "`{value}` must fail startup, naming the variable: {error:?}"
            );
        }
    }
}
