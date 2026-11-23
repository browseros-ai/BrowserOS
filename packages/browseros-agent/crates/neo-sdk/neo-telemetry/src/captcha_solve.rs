//! One CAPTCHA solve, as the server hands it to telemetry.

use crate::label::{CaptchaFamily, CaptchaProvider, CaptchaReason, CaptchaVerdict};

/// A solve that returned an outcome ([`crate::RunTelemetry::captcha_solved`]). A plain data bag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptchaSolve {
    /// The CAPTCHA it found.
    pub family: CaptchaFamily,
    /// How it ended.
    pub verdict: CaptchaVerdict,
    /// Why.
    pub reason: CaptchaReason,
    /// What it cost and who was paid; `None` for a free solve.
    pub spend: Option<CaptchaSpend>,
}

/// A solve's cost. A plain data bag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptchaSpend {
    /// Who it is booked to.
    pub provider: CaptchaProvider,
    /// In US dollars. Only a finite amount above 0 is counted.
    pub usd: f64,
}
