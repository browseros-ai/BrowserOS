//! Who was paid for a CAPTCHA solve, as a label.

/// The provider a solve's spend is booked to: the `neo.captcha.provider` of
/// `neo_captcha_spend_total`, spelled as the script's `provider` field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CaptchaProvider {
    /// `CapSolver`.
    Capsolver,
    /// 2Captcha.
    TwoCaptcha,
    /// A cost reported without a provider, kept rather than dropped.
    Unknown,
}

impl CaptchaProvider {
    /// The wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Capsolver => "capsolver",
            Self::TwoCaptcha => "2captcha",
            Self::Unknown => "unknown",
        }
    }
}
