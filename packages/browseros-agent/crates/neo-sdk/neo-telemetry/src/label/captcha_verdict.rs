//! How a CAPTCHA solve ended, as a label.

/// How a solve ended: the `neo.captcha.verdict` of `neo_captcha_solves_total`. Mirrors neo-captcha's
/// `SolveStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CaptchaVerdict {
    /// The widget produced a new response.
    Solved,
    /// A token was applied and the page still has to submit it.
    TokenApplied,
    /// Nothing blocked the page.
    AlreadyClear,
    /// A CAPTCHA Neo cannot solve.
    Unsupported,
    /// Neo tried and did not get through.
    Failed,
    /// The run stopped, the tab closed or the page navigated during the solve.
    Cancelled,
    /// Only a person can go on.
    NeedsHuman,
}

impl CaptchaVerdict {
    /// Every value, in wire order.
    pub const ALL: [Self; 7] = [
        Self::Solved,
        Self::TokenApplied,
        Self::AlreadyClear,
        Self::Unsupported,
        Self::Failed,
        Self::Cancelled,
        Self::NeedsHuman,
    ];

    /// The wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Solved => "solved",
            Self::TokenApplied => "token_applied",
            Self::AlreadyClear => "already_clear",
            Self::Unsupported => "unsupported",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::NeedsHuman => "needs_human",
        }
    }
}
