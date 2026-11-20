//! What the telemetry tests read Neo's output with, from outside the crate that writes it: the
//! Prometheus text the host API serves, parsed ([`Scrape`]), and an OTLP/HTTP collector in this
//! process that decodes what Neo pushes to it ([`MockCollector`]). `tests.rs` starts the whole
//! app on the fake browser and checks both doors at once, so every slice of the telemetry work
//! (startup config, run and op hooks, trace context, the scrape route, the push) is exercised
//! together rather than against a double.
//!
//! ```text
//!  App (real wiring, FakeCdp) ─┬─ host API socket ─ GET /v1/metrics ─▶ Scrape
//!                              └─ OTLP/HTTP (protobuf | JSON, TCP | Unix socket) ─▶ MockCollector
//! ```

mod mock_collector;
mod scrape;
mod tests;

pub(crate) use mock_collector::{ExportedSpan, MockCollector, Wire};
pub(crate) use scrape::Scrape;
