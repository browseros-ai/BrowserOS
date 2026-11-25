//! How much of a page's address an op span may carry: the span privacy dial.

use crate::TelemetryError;

/// How much of a page's address an op span may carry (`CLAW_TELEMETRY_SITE_DETAIL`). Spans
/// only: no metric ever carries a URL or a host, whatever this says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SiteDetail {
    /// No URL, host name, title, input or cookie anywhere. The default.
    #[default]
    None,
    /// `url.domain` only: the host name.
    Host,
    /// `url.domain` and `url.full`, with userinfo, query string and fragment stripped.
    Url,
}

impl SiteDetail {
    /// The environment variable that sets it.
    pub const ENV: &'static str = "CLAW_TELEMETRY_SITE_DETAIL";

    /// The spelling `CLAW_TELEMETRY_SITE_DETAIL` takes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Host => "host",
            Self::Url => "url",
        }
    }

    /// Reads `CLAW_TELEMETRY_SITE_DETAIL`'s value: `none`, `host` or `url`, exactly. Anything
    /// else fails startup, since a typo here would silently widen or narrow what spans carry.
    pub fn parse(value: &str) -> Result<Self, TelemetryError> {
        [Self::None, Self::Host, Self::Url]
            .into_iter()
            .find(|detail| detail.as_str() == value)
            .ok_or_else(|| TelemetryError::InvalidEnv {
                name: Self::ENV,
                reason: format!("`{value}` is not one of none, host, url"),
            })
    }
}
