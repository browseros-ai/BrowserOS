//! Why a CAPTCHA solve ended as it did, as a label.

/// Why a solve ended: the `neo.captcha.reason` of `neo_captcha_reasons_total`. Mirrors neo-captcha's
/// `SolveReason`, spelled as the script's `reason` field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CaptchaReason {
    /// The widget passed on its own while Neo waited.
    AutoPassed,
    /// Neo clicked the checkbox and it passed.
    CheckboxPassed,
    /// Neo answered the image challenge and it passed.
    TilesAccepted,
    /// Neo applied a provider's token and the page changed.
    TokenAccepted,
    /// Neo applied a token; the page has to submit it.
    AwaitingSubmit,
    /// No CAPTCHA Neo knows was on the page.
    NoCaptcha,
    /// The widget already reported itself complete.
    AlreadyPassed,
    /// Only a passive script or badge, no visible widget.
    NotBlocking,
    /// A CAPTCHA Neo cannot solve.
    UnsupportedType,
    /// Neo could not scan the whole page.
    ScanIncomplete,
    /// Neo acted and could not confirm the result.
    AcceptanceUnconfirmed,
    /// The provider failed.
    ProviderError,
    /// A browser call failed.
    BrowserError,
    /// The solve ran out of time.
    DeadlineExceeded,
    /// The run stopped during the solve.
    CallerCancelled,
    /// The tab closed during the solve.
    TabClosed,
    /// The page navigated during the solve.
    Navigated,
    /// No configured provider solves this CAPTCHA.
    NoProvider,
    /// The provider was unavailable.
    ProviderUnavailable,
    /// Every image round failed.
    TilesExhausted,
    /// The challenge did not pass.
    ChallengeNotPassed,
    /// This Neo's CAPTCHA budget is used up.
    SpendLimitReached,
}

impl CaptchaReason {
    /// Every value, in wire order.
    pub const ALL: [Self; 22] = [
        Self::AutoPassed,
        Self::CheckboxPassed,
        Self::TilesAccepted,
        Self::TokenAccepted,
        Self::AwaitingSubmit,
        Self::NoCaptcha,
        Self::AlreadyPassed,
        Self::NotBlocking,
        Self::UnsupportedType,
        Self::ScanIncomplete,
        Self::AcceptanceUnconfirmed,
        Self::ProviderError,
        Self::BrowserError,
        Self::DeadlineExceeded,
        Self::CallerCancelled,
        Self::TabClosed,
        Self::Navigated,
        Self::NoProvider,
        Self::ProviderUnavailable,
        Self::TilesExhausted,
        Self::ChallengeNotPassed,
        Self::SpendLimitReached,
    ];

    /// The wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AutoPassed => "auto_passed",
            Self::CheckboxPassed => "checkbox_passed",
            Self::TilesAccepted => "tiles_accepted",
            Self::TokenAccepted => "token_accepted",
            Self::AwaitingSubmit => "awaiting_submit",
            Self::NoCaptcha => "no_captcha",
            Self::AlreadyPassed => "already_passed",
            Self::NotBlocking => "not_blocking",
            Self::UnsupportedType => "unsupported_type",
            Self::ScanIncomplete => "scan_incomplete",
            Self::AcceptanceUnconfirmed => "acceptance_unconfirmed",
            Self::ProviderError => "provider_error",
            Self::BrowserError => "browser_error",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::CallerCancelled => "caller_cancelled",
            Self::TabClosed => "tab_closed",
            Self::Navigated => "navigated",
            Self::NoProvider => "no_provider",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::TilesExhausted => "tiles_exhausted",
            Self::ChallengeNotPassed => "challenge_not_passed",
            Self::SpendLimitReached => "spend_limit_reached",
        }
    }
}
