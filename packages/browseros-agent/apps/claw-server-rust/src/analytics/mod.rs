pub mod events;
mod service;
mod state;

use serde_json::Value;

pub use service::AnalyticsService;
pub use state::TelemetryState;

use events::EventDefinition;

/**
 * Product-event boundary used by lifecycle services. Definitions are opaque,
 * and delivery implementations own validation so producers never depend on
 * PostHog or its event model.
 */
pub trait AnalyticsSink: Send + Sync {
    fn capture(&self, event: EventDefinition, properties: Value);
}

#[derive(Debug, Default)]
pub struct NoopAnalyticsSink;

impl AnalyticsSink for NoopAnalyticsSink {
    fn capture(&self, _event: EventDefinition, _properties: Value) {}
}
