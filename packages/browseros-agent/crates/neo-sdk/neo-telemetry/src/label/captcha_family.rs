//! Which kind of CAPTCHA a solve met, as a label.

/// The CAPTCHA a solve found: the `neo.captcha.family` of `neo_captcha_solves_total`. Mirrors
/// neo-captcha's `CaptchaKind`, plus [`CaptchaFamily::None`] for a page with none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CaptchaFamily {
    /// reCAPTCHA v2.
    RecaptchaV2,
    /// reCAPTCHA v3.
    RecaptchaV3,
    /// Cloudflare Turnstile.
    Turnstile,
    /// Cloudflare's full-page challenge.
    CloudflareChallenge,
    /// hCaptcha.
    Hcaptcha,
    /// GeeTest.
    Geetest,
    /// AWS WAF.
    AwsWaf,
    /// Arkose Labs.
    Arkose,
    /// `PerimeterX` (HUMAN)'s press-and-hold check, which only a person can pass.
    Perimeterx,
    /// A challenge Neo detected and cannot name.
    Unknown,
    /// The solve found no CAPTCHA.
    None,
}

impl CaptchaFamily {
    /// Every value, in wire order.
    pub const ALL: [Self; 11] = [
        Self::RecaptchaV2,
        Self::RecaptchaV3,
        Self::Turnstile,
        Self::CloudflareChallenge,
        Self::Hcaptcha,
        Self::Geetest,
        Self::AwsWaf,
        Self::Arkose,
        Self::Perimeterx,
        Self::Unknown,
        Self::None,
    ];

    /// The wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RecaptchaV2 => "recaptcha_v2",
            Self::RecaptchaV3 => "recaptcha_v3",
            Self::Turnstile => "turnstile",
            Self::CloudflareChallenge => "cloudflare_challenge",
            Self::Hcaptcha => "hcaptcha",
            Self::Geetest => "geetest",
            Self::AwsWaf => "aws_waf",
            Self::Arkose => "arkose",
            Self::Perimeterx => "perimeterx",
            Self::Unknown => "unknown",
            Self::None => "none",
        }
    }
}
