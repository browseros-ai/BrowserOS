//! Prometheus text exposition 0.0.4 from one collection of the pipeline's manual reader, for
//! `GET /v1/metrics`.
//!
//! Our own renderer, not a third-party exporter: the subset is small (u64 counters, two i64
//! up-down counters, f64 histograms; no units, scope labels or `target_info`), the
//! catalog that built the instruments (`instruments::CATALOG`) supplies every `# HELP` / `# TYPE`
//! line and the family order, and an OpenTelemetry bump never waits on another crate's release.
//!
//! Deterministic: families in catalog order, series sorted by their label text, labels sorted by
//! name. Every family gets its `# HELP` / `# TYPE` lines even before it has a sample, so a
//! consumer sees the whole catalog from the first scrape.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::{Display, Write as _};

use opentelemetry::KeyValue;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, Metric, MetricData, ResourceMetrics};

use crate::instruments::{CATALOG, MetricSpec};

/// The exposition being written: one family at a time, in catalog order.
#[derive(Debug, Default)]
pub(crate) struct Prometheus {
    text: String,
}

/// One series' rendered lines, kept with its label text so a family's series can be sorted.
struct Series {
    labels: String,
    lines: String,
}

impl Prometheus {
    /// The exposition of `collected`: the catalog's families, each with whatever data points the
    /// collection holds for it. A metric the catalog does not name is left out.
    pub(crate) fn render(collected: &ResourceMetrics) -> String {
        let mut metrics: HashMap<&str, &Metric> = HashMap::new();
        for scope in collected.scope_metrics() {
            for metric in scope.metrics() {
                metrics.insert(metric.name(), metric);
            }
        }
        let mut exposition = Self::default();
        for spec in CATALOG {
            exposition.family(spec, metrics.get(spec.name).map(|metric| metric.data()));
        }
        exposition.text
    }

    fn family(&mut self, spec: &MetricSpec, data: Option<&AggregatedMetrics>) {
        let name = spec.name;
        let _ = writeln!(self.text, "# HELP {name} {}", escape_help(spec.help));
        let _ = writeln!(self.text, "# TYPE {name} {}", spec.kind.as_str());
        let mut series: Vec<Series> = match data {
            Some(AggregatedMetrics::U64(MetricData::Sum(sum))) => sum
                .data_points()
                .map(|point| sample(name, point.attributes(), point.value()))
                .collect(),
            Some(AggregatedMetrics::I64(MetricData::Sum(sum))) => sum
                .data_points()
                .map(|point| sample(name, point.attributes(), point.value()))
                .collect(),
            Some(AggregatedMetrics::F64(MetricData::Histogram(histogram))) => histogram
                .data_points()
                .map(|point| {
                    let labels = label_pairs(point.attributes());
                    let mut buckets = Vec::new();
                    let mut below = 0;
                    for (bound, count) in point.bounds().zip(point.bucket_counts()) {
                        below += count;
                        buckets.push((float(bound), below));
                    }
                    // OpenTelemetry's counts are per bucket, with one more for the overflow;
                    // Prometheus' are cumulative and end at `+Inf`, which is every observation.
                    buckets.push(("+Inf".to_owned(), point.count()));
                    let mut lines = String::new();
                    for (bound, below) in buckets {
                        let mut with_le = labels.clone();
                        with_le.push((Cow::Borrowed("le"), bound));
                        let _ = writeln!(lines, "{name}_bucket{} {below}", label_text(&with_le));
                    }
                    let text = label_text(&labels);
                    let _ = writeln!(lines, "{name}_sum{text} {}", float(point.sum()));
                    let _ = writeln!(lines, "{name}_count{text} {}", point.count());
                    Series {
                        labels: text,
                        lines,
                    }
                })
                .collect(),
            // The catalog builds only the shapes above; anything else is not ours to render.
            Some(_) | None => Vec::new(),
        };
        series.sort_by(|left, right| left.labels.cmp(&right.labels));
        for series in series {
            self.text.push_str(&series.lines);
        }
    }
}

/// A counter's or a gauge's one line.
fn sample<'a, T: Display>(
    name: &str,
    attributes: impl Iterator<Item = &'a KeyValue>,
    value: T,
) -> Series {
    let labels = label_text(&label_pairs(attributes));
    let lines = format!("{name}{labels} {value}\n");
    Series { labels, lines }
}

/// A data point's labels, sorted by name, values as text.
fn label_pairs<'a>(attributes: impl Iterator<Item = &'a KeyValue>) -> Vec<(Cow<'a, str>, String)> {
    let mut pairs: Vec<(Cow<'a, str>, String)> = attributes
        .map(|attribute| {
            (
                label_name(attribute.key.as_str()),
                attribute.value.as_str().into_owned(),
            )
        })
        .collect();
    pairs.sort_by(|left, right| left.0.cmp(&right.0));
    pairs
}

/// A label name as the text format allows it, `[a-zA-Z_][a-zA-Z0-9_]*`. The catalog's names
/// already are; this keeps the text well-formed should the SDK add one of its own, such as the
/// `otel.metric.overflow` series it opens past its cardinality limit (2000 per metric, far above
/// the closed label sets).
fn label_name(key: &str) -> Cow<'_, str> {
    let allowed = |character: char| character.is_ascii_alphanumeric() || character == '_';
    let leading_digit = key.starts_with(|character: char| character.is_ascii_digit());
    if !key.is_empty() && !leading_digit && key.chars().all(allowed) {
        return Cow::Borrowed(key);
    }
    let mut name = String::with_capacity(key.len() + 1);
    if key.is_empty() || leading_digit {
        name.push('_');
    }
    name.extend(
        key.chars()
            .map(|character| if allowed(character) { character } else { '_' }),
    );
    Cow::Owned(name)
}

/// `{name="value",…}`, or nothing for a series without labels.
fn label_text(pairs: &[(Cow<'_, str>, String)]) -> String {
    if pairs.is_empty() {
        return String::new();
    }
    let mut text = String::from("{");
    for (index, (name, value)) in pairs.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        let _ = write!(text, "{name}=\"{}\"", escape_label_value(value));
    }
    text.push('}');
    text
}

/// A label value as the text format quotes it: backslash, double quote and line feed escaped.
/// Every value is a closed `&'static str` today; this keeps the text well-formed whatever a
/// future label holds.
fn escape_label_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// `# HELP` text: backslash and line feed escaped (a quote needs none there).
fn escape_help(help: &str) -> String {
    help.replace('\\', "\\\\").replace('\n', "\\n")
}

/// A float as the text format spells it: `+Inf`, `-Inf`, `NaN`, else the shortest decimal that
/// reads back exactly (`0.005`, `1`, `2.5`).
fn float(value: f64) -> String {
    if value.is_nan() {
        "NaN".to_owned()
    } else if value.is_infinite() {
        if value.is_sign_positive() {
            "+Inf"
        } else {
            "-Inf"
        }
        .to_owned()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;
    use std::time::Duration;

    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    use opentelemetry_sdk::metrics::reader::MetricReader as _;

    use super::*;
    use crate::instruments::{CodeLabel, Instruments, OpLabel, OpenTabs};
    use crate::label::{
        CloseReason, ErrorCode, ErrorLabel, ErrorOwner, OpOutcome, RunStatus, TabOrigin,
    };
    use crate::pipeline::ScrapeReader;
    use crate::{OpenTabCounts, Vocabulary};

    fn rendered(record: impl FnOnce(&Instruments, &OpenTabs)) -> Result<String, Box<dyn Error>> {
        let reader = ScrapeReader::new();
        let provider = SdkMeterProvider::builder()
            .with_reader(reader.clone())
            .build();
        let open_tabs = Arc::new(OpenTabs::default());
        let instruments = Instruments::new(&provider.meter("test"), Arc::clone(&open_tabs));
        record(&instruments, &open_tabs);
        let mut collected = ResourceMetrics::default();
        reader.collect(&mut collected)?;
        Ok(Prometheus::render(&collected))
    }

    #[test]
    fn a_fresh_pipeline_lists_every_family_and_its_two_unlabelled_zeros()
    -> Result<(), Box<dyn Error>> {
        let text = rendered(|_, _| {})?;
        let help: Vec<&str> = text
            .lines()
            .filter_map(|line| line.strip_prefix("# HELP "))
            .filter_map(|line| line.split(' ').next())
            .collect();
        let catalog: Vec<&str> = CATALOG.iter().map(|spec| spec.name).collect();
        assert_eq!(
            help, catalog,
            "every family is listed, in catalog order, before any data"
        );
        let samples: Vec<&str> = text.lines().filter(|line| !line.starts_with('#')).collect();
        assert_eq!(
            samples,
            vec!["neo_runs_in_flight 0", "neo_renderer_crashes_total 0"],
            "only the unlabelled series exist before anything happened; no source, no tab gauge"
        );
        Ok(())
    }

    /// The exposition `renders_the_golden_exposition` records: one of each kind, in catalog order.
    const GOLDEN: &str = "\
# HELP neo_runs_total Runs that ended, and calls refused before they became a run, by status and by who must fix the failure.
# TYPE neo_runs_total counter
neo_runs_total{error_owner=\"none\",status=\"ok\"} 1
# HELP neo_runs_in_flight Runs admitted and not yet ended.
# TYPE neo_runs_in_flight gauge
neo_runs_in_flight 1
# HELP neo_run_duration_seconds How long each run took, from admission to its end, by status.
# TYPE neo_run_duration_seconds histogram
neo_run_duration_seconds_bucket{status=\"ok\",le=\"0.1\"} 0
neo_run_duration_seconds_bucket{status=\"ok\",le=\"0.25\"} 0
neo_run_duration_seconds_bucket{status=\"ok\",le=\"0.5\"} 1
neo_run_duration_seconds_bucket{status=\"ok\",le=\"1\"} 1
neo_run_duration_seconds_bucket{status=\"ok\",le=\"2.5\"} 1
neo_run_duration_seconds_bucket{status=\"ok\",le=\"5\"} 1
neo_run_duration_seconds_bucket{status=\"ok\",le=\"10\"} 1
neo_run_duration_seconds_bucket{status=\"ok\",le=\"30\"} 1
neo_run_duration_seconds_bucket{status=\"ok\",le=\"60\"} 1
neo_run_duration_seconds_bucket{status=\"ok\",le=\"120\"} 1
neo_run_duration_seconds_bucket{status=\"ok\",le=\"300\"} 1
neo_run_duration_seconds_bucket{status=\"ok\",le=\"600\"} 1
neo_run_duration_seconds_bucket{status=\"ok\",le=\"900\"} 1
neo_run_duration_seconds_bucket{status=\"ok\",le=\"+Inf\"} 1
neo_run_duration_seconds_sum{status=\"ok\"} 0.3
neo_run_duration_seconds_count{status=\"ok\"} 1
# HELP neo_operations_total Operations settled, by op (unregistered names count as unknown), status and who must fix the failure.
# TYPE neo_operations_total counter
neo_operations_total{error_owner=\"website\",op=\"page.goto\",status=\"error\"} 1
# HELP neo_operation_duration_seconds How long each operation took, by op and status.
# TYPE neo_operation_duration_seconds histogram
neo_operation_duration_seconds_bucket{op=\"page.goto\",status=\"error\",le=\"0.005\"} 0
neo_operation_duration_seconds_bucket{op=\"page.goto\",status=\"error\",le=\"0.01\"} 0
neo_operation_duration_seconds_bucket{op=\"page.goto\",status=\"error\",le=\"0.025\"} 1
neo_operation_duration_seconds_bucket{op=\"page.goto\",status=\"error\",le=\"0.05\"} 1
neo_operation_duration_seconds_bucket{op=\"page.goto\",status=\"error\",le=\"0.1\"} 1
neo_operation_duration_seconds_bucket{op=\"page.goto\",status=\"error\",le=\"0.25\"} 1
neo_operation_duration_seconds_bucket{op=\"page.goto\",status=\"error\",le=\"0.5\"} 1
neo_operation_duration_seconds_bucket{op=\"page.goto\",status=\"error\",le=\"1\"} 1
neo_operation_duration_seconds_bucket{op=\"page.goto\",status=\"error\",le=\"2.5\"} 1
neo_operation_duration_seconds_bucket{op=\"page.goto\",status=\"error\",le=\"5\"} 1
neo_operation_duration_seconds_bucket{op=\"page.goto\",status=\"error\",le=\"10\"} 1
neo_operation_duration_seconds_bucket{op=\"page.goto\",status=\"error\",le=\"30\"} 1
neo_operation_duration_seconds_bucket{op=\"page.goto\",status=\"error\",le=\"60\"} 1
neo_operation_duration_seconds_bucket{op=\"page.goto\",status=\"error\",le=\"120\"} 1
neo_operation_duration_seconds_bucket{op=\"page.goto\",status=\"error\",le=\"+Inf\"} 1
neo_operation_duration_seconds_sum{op=\"page.goto\",status=\"error\"} 0.02
neo_operation_duration_seconds_count{op=\"page.goto\",status=\"error\"} 1
# HELP neo_errors_total Failures by who must fix them and their code, each counted once where it is attributed.
# TYPE neo_errors_total counter
neo_errors_total{error_code=\"no_match\",error_owner=\"website\"} 1
# HELP neo_tabs_open Tabs open now, by who opened them.
# TYPE neo_tabs_open gauge
neo_tabs_open{origin=\"agent\"} 2
neo_tabs_open{origin=\"found\"} 0
neo_tabs_open{origin=\"neo\"} 1
# HELP neo_tabs_opened_total Tabs Neo registered, by who opened them.
# TYPE neo_tabs_opened_total counter
neo_tabs_opened_total{origin=\"agent\"} 1
# HELP neo_tabs_closed_total Tabs closed, by who opened them and why they closed.
# TYPE neo_tabs_closed_total counter
neo_tabs_closed_total{close_reason=\"script\",origin=\"agent\"} 1
# HELP neo_tab_lifetime_seconds How long each closed tab was open, by who opened it and why it closed.
# TYPE neo_tab_lifetime_seconds histogram
neo_tab_lifetime_seconds_bucket{close_reason=\"script\",origin=\"agent\",le=\"1\"} 0
neo_tab_lifetime_seconds_bucket{close_reason=\"script\",origin=\"agent\",le=\"10\"} 0
neo_tab_lifetime_seconds_bucket{close_reason=\"script\",origin=\"agent\",le=\"30\"} 0
neo_tab_lifetime_seconds_bucket{close_reason=\"script\",origin=\"agent\",le=\"60\"} 1
neo_tab_lifetime_seconds_bucket{close_reason=\"script\",origin=\"agent\",le=\"300\"} 1
neo_tab_lifetime_seconds_bucket{close_reason=\"script\",origin=\"agent\",le=\"900\"} 1
neo_tab_lifetime_seconds_bucket{close_reason=\"script\",origin=\"agent\",le=\"1800\"} 1
neo_tab_lifetime_seconds_bucket{close_reason=\"script\",origin=\"agent\",le=\"3600\"} 1
neo_tab_lifetime_seconds_bucket{close_reason=\"script\",origin=\"agent\",le=\"7200\"} 1
neo_tab_lifetime_seconds_bucket{close_reason=\"script\",origin=\"agent\",le=\"14400\"} 1
neo_tab_lifetime_seconds_bucket{close_reason=\"script\",origin=\"agent\",le=\"43200\"} 1
neo_tab_lifetime_seconds_bucket{close_reason=\"script\",origin=\"agent\",le=\"86400\"} 1
neo_tab_lifetime_seconds_bucket{close_reason=\"script\",origin=\"agent\",le=\"+Inf\"} 1
neo_tab_lifetime_seconds_sum{close_reason=\"script\",origin=\"agent\"} 45
neo_tab_lifetime_seconds_count{close_reason=\"script\",origin=\"agent\"} 1
# HELP neo_renderer_crashes_total Renderer crashes, whether or not an op was using the tab.
# TYPE neo_renderer_crashes_total counter
neo_renderer_crashes_total 1
";

    #[test]
    fn renders_the_golden_exposition() -> Result<(), Box<dyn Error>> {
        let vocabulary = Vocabulary::new(["page.goto"], ["no_match"]);
        let text = rendered(|instruments, open_tabs| {
            instruments.run_started();
            instruments.run_started();
            instruments.run_ended(RunStatus::Ok, None, Duration::from_millis(300));
            instruments.op_settled(
                OpLabel::of(Some(&vocabulary), "page.goto"),
                OpOutcome::Error,
                Some(ErrorLabel {
                    owner: ErrorOwner::Website,
                    code: ErrorCode::new("no_match"),
                }),
                Duration::from_millis(20),
            );
            instruments.error_counted(
                ErrorOwner::Website,
                CodeLabel::of(Some(&vocabulary), ErrorCode::new("no_match")),
            );
            instruments.tab_opened(TabOrigin::Agent);
            instruments.tab_closed(
                TabOrigin::Agent,
                CloseReason::Script,
                Duration::from_secs(45),
            );
            instruments.renderer_crashed();
            open_tabs.set(Arc::new(|| OpenTabCounts {
                agent: 2,
                found: 0,
                neo: 1,
            }));
        })?;
        assert_eq!(
            text, GOLDEN,
            "the exposition is exactly the text format, in catalog order"
        );
        Ok(())
    }

    #[test]
    fn series_are_sorted_and_histograms_are_cumulative() -> Result<(), Box<dyn Error>> {
        let text = rendered(|instruments, _| {
            for (status, seconds) in [
                (RunStatus::Timeout, 1000.0),
                (RunStatus::Error, 0.05),
                (RunStatus::Ok, 3.0),
                (RunStatus::Ok, 0.2),
                (RunStatus::Ok, 7.0),
            ] {
                instruments.run_started();
                instruments.run_ended(status, None, Duration::from_secs_f64(seconds));
            }
        })?;
        let runs: Vec<&str> = text
            .lines()
            .filter(|line| line.starts_with("neo_runs_total"))
            .collect();
        assert_eq!(
            runs,
            vec![
                "neo_runs_total{error_owner=\"none\",status=\"error\"} 1",
                "neo_runs_total{error_owner=\"none\",status=\"ok\"} 3",
                "neo_runs_total{error_owner=\"none\",status=\"timeout\"} 1",
            ],
            "series sort by their label text, whatever order they were recorded in"
        );
        let ok_buckets: Vec<u64> = text
            .lines()
            .filter(|line| line.starts_with("neo_run_duration_seconds_bucket{status=\"ok\""))
            .filter_map(|line| line.rsplit(' ').next())
            .map(str::parse)
            .collect::<Result<_, _>>()?;
        assert!(
            ok_buckets.windows(2).all(|pair| pair[0] <= pair[1]),
            "bucket counts never go down: they are cumulative: {ok_buckets:?}"
        );
        assert_eq!(
            ok_buckets.last(),
            Some(&3),
            "`+Inf` holds every observation of the series"
        );
        assert!(
            text.contains("neo_run_duration_seconds_bucket{status=\"timeout\",le=\"900\"} 0\n")
                && text.contains(
                    "neo_run_duration_seconds_bucket{status=\"timeout\",le=\"+Inf\"} 1\n"
                ),
            "a run longer than the last bound counts only in `+Inf`"
        );
        Ok(())
    }

    #[test]
    fn every_label_name_is_one_the_text_format_accepts() {
        for name in ["status", "error_owner", "op"] {
            assert_eq!(label_name(name), name, "the catalog's names pass unchanged");
        }
        assert_eq!(
            label_name("otel.metric.overflow"),
            "otel_metric_overflow",
            "the SDK's own overflow label would otherwise break the exposition"
        );
        assert_eq!(
            label_name("9lives"),
            "_9lives",
            "a name never starts with a digit"
        );
        assert_eq!(label_name(""), "_", "a name is never empty");
    }

    #[test]
    fn label_values_and_help_are_escaped_and_floats_spelled_as_the_format_wants() {
        assert_eq!(
            escape_label_value("a\\b\"c\nd"),
            "a\\\\b\\\"c\\nd",
            "backslash, quote and line feed are escaped in a label value"
        );
        assert_eq!(
            escape_help("line\\one\nline \"two\""),
            "line\\\\one\\nline \"two\"",
            "help escapes backslash and line feed, never quotes"
        );
        for (value, spelled) in [
            (0.005, "0.005"),
            (1.0, "1"),
            (2.5, "2.5"),
            (86400.0, "86400"),
            (f64::INFINITY, "+Inf"),
            (f64::NEG_INFINITY, "-Inf"),
            (f64::NAN, "NaN"),
        ] {
            assert_eq!(
                float(value),
                spelled,
                "{value} as the text format spells it"
            );
        }
    }
}
