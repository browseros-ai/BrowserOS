//! Process telemetry: file logging (`<executionDir>/browseros-server.log`, rotated), Sentry for
//! startup failures, panics and unhandled request errors, and PostHog metrics with the TS event
//! names.

pub mod logging;
pub mod metrics;
pub mod sentry;
