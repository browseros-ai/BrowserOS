//! The pinned OpenTelemetry SDK still has the shapes Neo's Prometheus scrape is built on.
//!
//! A version pin kept as a test, so an OpenTelemetry bump that changes one fails here first: a
//! meter provider read through a shared manual reader, and names, values, histogram bounds and an
//! observable up-down counter read back through `ResourceMetrics`' public accessors, as
//! `prometheus.rs` reads them. The OTLP push door has tests of its own (`otlp.rs`,
//! `transport.rs`, `tests/push_shutdown.rs`).

use std::error::Error;
use std::sync::{Arc, Weak};
use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry::metrics::MeterProvider;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData, ResourceMetrics};
use opentelemetry_sdk::metrics::reader::MetricReader;
use opentelemetry_sdk::metrics::{
    InstrumentKind, ManualReader, Pipeline, SdkMeterProvider, Temporality,
};

/// The scrape's reader, shared: the meter provider owns one clone, the renderer collects
/// through another, as `pipeline.rs`'s `ScrapeReader` does.
#[derive(Clone, Debug)]
struct SharedReader(Arc<ManualReader>);

impl MetricReader for SharedReader {
    fn register_pipeline(&self, pipeline: Weak<Pipeline>) {
        self.0.register_pipeline(pipeline);
    }

    fn collect(&self, metrics: &mut ResourceMetrics) -> OTelSdkResult {
        self.0.collect(metrics)
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.0.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.0.shutdown_with_timeout(timeout)
    }

    fn temporality(&self, kind: InstrumentKind) -> Temporality {
        self.0.temporality(kind)
    }
}

#[test]
fn the_scrape_reads_the_same_meter_provider_through_a_shared_manual_reader()
-> Result<(), Box<dyn Error>> {
    let reader = SharedReader(Arc::new(
        ManualReader::builder()
            .with_temporality(Temporality::Cumulative)
            .build(),
    ));
    let provider = SdkMeterProvider::builder()
        .with_resource(
            Resource::builder_empty()
                .with_service_name("browseros-neo")
                .build(),
        )
        .with_reader(reader.clone())
        .build();
    let meter = provider.meter("neo-telemetry");
    meter
        .u64_counter("neo_spike_total")
        .build()
        .add(2, &[KeyValue::new("status", "ok")]);
    meter
        .f64_histogram("neo_spike_duration_seconds")
        .with_unit("s")
        .with_boundaries(vec![0.1, 1.0])
        .build()
        .record(0.5, &[]);
    let _open = meter
        .i64_observable_up_down_counter("neo_spike_open")
        .with_callback(|observer| observer.observe(3, &[KeyValue::new("origin", "agent")]))
        .build();

    let mut collected = ResourceMetrics::default();
    reader.collect(&mut collected)?;
    let mut seen = Vec::new();
    for scope in collected.scope_metrics() {
        for metric in scope.metrics() {
            let summary = match metric.data() {
                AggregatedMetrics::U64(MetricData::Sum(sum)) => sum
                    .data_points()
                    .map(|point| format!("{} {}", metric.name(), point.value()))
                    .collect::<Vec<_>>()
                    .join(","),
                AggregatedMetrics::I64(MetricData::Sum(sum)) => sum
                    .data_points()
                    .map(|point| format!("{} {}", metric.name(), point.value()))
                    .collect::<Vec<_>>()
                    .join(","),
                AggregatedMetrics::F64(MetricData::Histogram(histogram)) => histogram
                    .data_points()
                    .map(|point| {
                        let bounds: Vec<f64> = point.bounds().collect();
                        let counts: Vec<u64> = point.bucket_counts().collect();
                        format!("{} {bounds:?} {counts:?} {}", metric.name(), point.count())
                    })
                    .collect::<Vec<_>>()
                    .join(","),
                _ => format!("{} other", metric.name()),
            };
            seen.push(summary);
        }
    }
    seen.sort();
    assert_eq!(
        seen,
        vec![
            "neo_spike_duration_seconds [0.1, 1.0] [0, 1, 0] 1".to_owned(),
            "neo_spike_open 3".to_owned(),
            "neo_spike_total 2".to_owned(),
        ],
        "the renderer reads names, values, bounds and bucket counts from the reader, and the \
         observable up-down counter is read at collection time"
    );
    provider.shutdown()?;
    Ok(())
}
