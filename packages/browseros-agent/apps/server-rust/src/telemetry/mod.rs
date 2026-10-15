//! Process telemetry: file logging (`<executionDir>/browseros-server.log`, rotated; P3), Sentry
//! for startup failures and panics, PostHog metrics with the TS event names (P13).

pub mod logging;
pub mod metrics;
pub mod sentry;
