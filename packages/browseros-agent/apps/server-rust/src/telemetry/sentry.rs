//! Sentry for startup failures and panics only (claw's minimal config; `request-type=schedule`
//! tag on chat errors). Implemented by P13.

/// Keeps the client alive for the process lifetime; dropping it flushes.
#[derive(Debug, Default)]
pub struct SentryGuard {
    _private: (),
}

/// S0 stub: Sentry disabled.
pub fn init() -> SentryGuard {
    SentryGuard::default()
}
