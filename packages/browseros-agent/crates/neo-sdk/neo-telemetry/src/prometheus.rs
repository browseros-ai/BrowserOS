//! Prometheus text exposition 0.0.4 over the pipeline's manual reader, ordered by the instrument
//! table (`instruments.rs`), for `GET /v1/metrics`.
//!
//! Owned by the metrics-and-spans work. A renderer of our own (`ResourceMetrics`' accessors are
//! public under `experimental_metrics_custom_reader`), so an SDK bump never waits on a
//! third-party exporter. `# HELP` / `# TYPE` per family; a counter name ends in `_total` once; a
//! histogram renders `_bucket{le=..}` up to `+Inf`, `_sum` and `_count`; no `target_info`, no
//! scope labels.
