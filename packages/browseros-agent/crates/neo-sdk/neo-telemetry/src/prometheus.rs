//! Prometheus text exposition 0.0.4 from one collection of the pipeline's manual reader, for
//! `GET /v1/metrics`.
//!
//! Our own renderer, not a third-party exporter: the subset is small (u64 counters, one f64
//! counter, two i64 up-down counters, f64 histograms; no units, scope labels or `target_info`), the
//! catalog that built the instruments (`instruments::CATALOG`) supplies every `# HELP` / `# TYPE`
//! line and the family order, and an OpenTelemetry bump never waits on another crate's release.
//!
//! Deterministic: families in catalog order, series sorted by their label text, labels sorted by
//! name. Every family gets its `# HELP` / `# TYPE` lines even before it has a sample, so a
//! consumer sees the whole catalog from the first scrape.
//!
//! The instruments carry OpenTelemetry names and attribute keys (`neo.run.duration`,
//! `neo.run.status`), which the OTLP door sends as they are. This text derives its names from them
//! by OpenTelemetry's Prometheus compatibility rules (`neo_run_duration_seconds`,
//! `neo_run_status`), the translation a collector's Prometheus exporter and Prometheus' own OTLP
//! receiver apply to a push, so a dashboard reads the same names whichever door fed it.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::{Display, Write as _};

use opentelemetry::KeyValue;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, Metric, MetricData, ResourceMetrics};

use crate::instruments::{CATALOG, Kind, MetricSpec};

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
            exposition.family(spec, metrics.get(spec.otel).map(|metric| metric.data()));
        }
        exposition.text
    }

    fn family(&mut self, spec: &MetricSpec, data: Option<&AggregatedMetrics>) {
        let name = family_name(spec);
        let name = name.as_str();
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
            Some(AggregatedMetrics::F64(MetricData::Sum(sum))) => sum
                .data_points()
                .map(|point| sample(name, point.attributes(), float(point.value())))
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

/// A family's Prometheus name, derived from its OpenTelemetry name as OpenTelemetry's Prometheus
/// compatibility rules (and Prometheus' own OTLP receiver) derive it: the unit's word follows the
/// name unless it already ends with it (`s` is `seconds`; an annotation such as `{run}` names what
/// is counted and adds nothing), a counter (a monotonic sum) ends in `_total`, every character the
/// format does not allow becomes `_`, and runs of `_` are one. An up-down counter is a gauge, so
/// it gets no `_total`.
pub(crate) fn family_name(spec: &MetricSpec) -> String {
    let mut name = spec.otel.to_owned();
    if let Some(word) = unit_word(spec.unit)
        && !name.ends_with(&format!("_{word}"))
    {
        name.push('_');
        name.push_str(&word);
    }
    if spec.kind == Kind::Counter && !name.ends_with("_total") {
        name.push_str("_total");
    }
    sanitized(&name, |character| {
        character.is_ascii_alphanumeric() || matches!(character, '_' | ':')
    })
}

/// The words a UCUM unit adds to a name (`s` is `seconds`, `By/s` is `bytes_per_second`), or
/// `None` for a unit that adds nothing: none, or only an annotation in braces such as `{run}`.
fn unit_word(unit: &str) -> Option<String> {
    let unit = without_annotations(unit);
    let (main, per) = unit.split_once('/').unwrap_or((&unit, ""));
    let word = |part: &str| unit_name(part).unwrap_or(part).to_owned();
    match (main.is_empty(), per.is_empty()) {
        (true, true) => None,
        (false, true) => Some(word(main)),
        (true, false) => Some(format!("per_{}", per_unit_name(per))),
        (false, false) => Some(format!("{}_per_{}", word(main), per_unit_name(per))),
    }
}

/// `unit` without its `{...}` annotations.
fn without_annotations(unit: &str) -> String {
    let mut out = String::with_capacity(unit.len());
    let mut depth = 0_usize;
    for character in unit.chars() {
        match character {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            other if depth == 0 => out.push(other),
            _ => {}
        }
    }
    out
}

/// The spec's table of UCUM units Prometheus spells out.
fn unit_name(unit: &str) -> Option<&'static str> {
    Some(match unit {
        "d" => "days",
        "h" => "hours",
        "min" => "minutes",
        "s" => "seconds",
        "ms" => "milliseconds",
        "us" => "microseconds",
        "ns" => "nanoseconds",
        "By" => "bytes",
        "KiBy" => "kibibytes",
        "MiBy" => "mebibytes",
        "GiBy" => "gibibytes",
        "TiBy" => "tebibytes",
        "kBy" => "kilobytes",
        "MBy" => "megabytes",
        "GBy" => "gigabytes",
        "TBy" => "terabytes",
        "m" => "meters",
        "V" => "volts",
        "A" => "amperes",
        "J" => "joules",
        "W" => "watts",
        "g" => "grams",
        "Cel" => "celsius",
        "Hz" => "hertz",
        "%" => "percent",
        _ => return None,
    })
}

/// A rate's time unit in the singular, as Prometheus spells it after `per_`.
fn per_unit_name(unit: &str) -> &str {
    match unit {
        "s" => "second",
        "m" => "minute",
        "h" => "hour",
        "d" => "day",
        "w" => "week",
        "mo" => "month",
        "y" => "year",
        other => unit_name(other).unwrap_or(other),
    }
}

/// `text` with every character `allowed` rejects as `_`, and runs of `_` as one.
fn sanitized(text: &str, allowed: impl Fn(char) -> bool) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        let character = if allowed(character) { character } else { '_' };
        if !(character == '_' && out.ends_with('_')) {
            out.push(character);
        }
    }
    out
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

/// A label name as the text format allows it, `[a-zA-Z_][a-zA-Z0-9_]*`, translated from an
/// attribute key by the same compatibility rules as the family names: every other character is
/// `_` (`neo.op.name` is `neo_op_name`), runs of `_` are one, and a key that starts with a digit
/// gets a `key_` prefix, as Prometheus' OTLP receiver spells it. Any key works, so an attribute
/// the SDK adds on its own, such as the `otel.metric.overflow` series it opens past its
/// cardinality limit (2000 per metric, far above the closed label sets), keeps the text
/// well-formed.
pub(crate) fn label_name(key: &str) -> Cow<'_, str> {
    let allowed = |character: char| character.is_ascii_alphanumeric() || character == '_';
    let leading_digit = key.starts_with(|character: char| character.is_ascii_digit());
    if !key.is_empty() && !leading_digit && !key.contains("__") && key.chars().all(allowed) {
        return Cow::Borrowed(key);
    }
    if key.is_empty() {
        return Cow::Borrowed("_");
    }
    let name = sanitized(key, allowed);
    Cow::Owned(if leading_digit {
        format!("key_{name}")
    } else {
        name
    })
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
        CaptchaFamily, CaptchaProvider, CaptchaReason, CaptchaVerdict, CloseReason, ErrorCode,
        ErrorLabel, ErrorOwner, HttpStatus, NetError, OpOutcome, RunStatus, TabOrigin,
    };
    use crate::pipeline::ScrapeReader;
    use crate::{CaptchaSolve, CaptchaSpend, Navigated, OpenTabCounts, Vocabulary};

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
        let catalog: Vec<String> = CATALOG.iter().map(|spec| family_name(spec)).collect();
        assert_eq!(
            help, catalog,
            "every family is listed, in catalog order, before any data"
        );
        let samples: Vec<&str> = text.lines().filter(|line| !line.starts_with('#')).collect();
        assert_eq!(
            samples,
            vec!["neo_run_active 0", "neo_renderer_crashes_total 0"],
            "only the unlabelled series exist before anything happened; no source, no tab gauge"
        );
        Ok(())
    }

    /// The exposition `renders_the_golden_exposition` records: one of each kind, in catalog order.
    /// A navigation without a status is a series without `http_response_status_code`, and the f64 spend
    /// counter renders as the shortest decimal that reads back.
    const GOLDEN: &str = "\
# HELP neo_runs_total Runs that ended, and calls refused before they became a run, by status and by who must fix the failure.
# TYPE neo_runs_total counter
neo_runs_total{neo_error_owner=\"none\",neo_run_status=\"ok\"} 1
# HELP neo_run_active Runs admitted and not yet ended.
# TYPE neo_run_active gauge
neo_run_active 1
# HELP neo_run_duration_seconds How long each run took, from admission to its end, by status.
# TYPE neo_run_duration_seconds histogram
neo_run_duration_seconds_bucket{neo_run_status=\"ok\",le=\"0.1\"} 0
neo_run_duration_seconds_bucket{neo_run_status=\"ok\",le=\"0.25\"} 0
neo_run_duration_seconds_bucket{neo_run_status=\"ok\",le=\"0.5\"} 1
neo_run_duration_seconds_bucket{neo_run_status=\"ok\",le=\"1\"} 1
neo_run_duration_seconds_bucket{neo_run_status=\"ok\",le=\"2.5\"} 1
neo_run_duration_seconds_bucket{neo_run_status=\"ok\",le=\"5\"} 1
neo_run_duration_seconds_bucket{neo_run_status=\"ok\",le=\"10\"} 1
neo_run_duration_seconds_bucket{neo_run_status=\"ok\",le=\"30\"} 1
neo_run_duration_seconds_bucket{neo_run_status=\"ok\",le=\"60\"} 1
neo_run_duration_seconds_bucket{neo_run_status=\"ok\",le=\"120\"} 1
neo_run_duration_seconds_bucket{neo_run_status=\"ok\",le=\"300\"} 1
neo_run_duration_seconds_bucket{neo_run_status=\"ok\",le=\"600\"} 1
neo_run_duration_seconds_bucket{neo_run_status=\"ok\",le=\"900\"} 1
neo_run_duration_seconds_bucket{neo_run_status=\"ok\",le=\"+Inf\"} 1
neo_run_duration_seconds_sum{neo_run_status=\"ok\"} 0.3
neo_run_duration_seconds_count{neo_run_status=\"ok\"} 1
# HELP neo_operations_total Operations settled, by op (unregistered names count as unknown), status and who must fix the failure.
# TYPE neo_operations_total counter
neo_operations_total{neo_error_owner=\"website\",neo_op_name=\"page.goto\",neo_op_outcome=\"error\"} 1
# HELP neo_operation_duration_seconds How long each operation took, by op and status.
# TYPE neo_operation_duration_seconds histogram
neo_operation_duration_seconds_bucket{neo_op_name=\"page.goto\",neo_op_outcome=\"error\",le=\"0.005\"} 0
neo_operation_duration_seconds_bucket{neo_op_name=\"page.goto\",neo_op_outcome=\"error\",le=\"0.01\"} 0
neo_operation_duration_seconds_bucket{neo_op_name=\"page.goto\",neo_op_outcome=\"error\",le=\"0.025\"} 1
neo_operation_duration_seconds_bucket{neo_op_name=\"page.goto\",neo_op_outcome=\"error\",le=\"0.05\"} 1
neo_operation_duration_seconds_bucket{neo_op_name=\"page.goto\",neo_op_outcome=\"error\",le=\"0.1\"} 1
neo_operation_duration_seconds_bucket{neo_op_name=\"page.goto\",neo_op_outcome=\"error\",le=\"0.25\"} 1
neo_operation_duration_seconds_bucket{neo_op_name=\"page.goto\",neo_op_outcome=\"error\",le=\"0.5\"} 1
neo_operation_duration_seconds_bucket{neo_op_name=\"page.goto\",neo_op_outcome=\"error\",le=\"1\"} 1
neo_operation_duration_seconds_bucket{neo_op_name=\"page.goto\",neo_op_outcome=\"error\",le=\"2.5\"} 1
neo_operation_duration_seconds_bucket{neo_op_name=\"page.goto\",neo_op_outcome=\"error\",le=\"5\"} 1
neo_operation_duration_seconds_bucket{neo_op_name=\"page.goto\",neo_op_outcome=\"error\",le=\"10\"} 1
neo_operation_duration_seconds_bucket{neo_op_name=\"page.goto\",neo_op_outcome=\"error\",le=\"30\"} 1
neo_operation_duration_seconds_bucket{neo_op_name=\"page.goto\",neo_op_outcome=\"error\",le=\"60\"} 1
neo_operation_duration_seconds_bucket{neo_op_name=\"page.goto\",neo_op_outcome=\"error\",le=\"120\"} 1
neo_operation_duration_seconds_bucket{neo_op_name=\"page.goto\",neo_op_outcome=\"error\",le=\"+Inf\"} 1
neo_operation_duration_seconds_sum{neo_op_name=\"page.goto\",neo_op_outcome=\"error\"} 0.02
neo_operation_duration_seconds_count{neo_op_name=\"page.goto\",neo_op_outcome=\"error\"} 1
# HELP neo_errors_total Failures by who must fix them and their code, each counted once where it is attributed.
# TYPE neo_errors_total counter
neo_errors_total{error_type=\"no_match\",neo_error_owner=\"website\"} 1
# HELP neo_tab_count Tabs open now, by who opened them.
# TYPE neo_tab_count gauge
neo_tab_count{neo_tab_origin=\"agent\"} 2
neo_tab_count{neo_tab_origin=\"found\"} 0
neo_tab_count{neo_tab_origin=\"neo\"} 1
# HELP neo_tab_opened_total Tabs Neo registered, by who opened them.
# TYPE neo_tab_opened_total counter
neo_tab_opened_total{neo_tab_origin=\"agent\"} 1
# HELP neo_tab_closed_total Tabs closed, by who opened them and why they closed.
# TYPE neo_tab_closed_total counter
neo_tab_closed_total{neo_tab_close_reason=\"script\",neo_tab_origin=\"agent\"} 1
# HELP neo_tab_duration_seconds How long each closed tab was open, by who opened it and why it closed.
# TYPE neo_tab_duration_seconds histogram
neo_tab_duration_seconds_bucket{neo_tab_close_reason=\"script\",neo_tab_origin=\"agent\",le=\"1\"} 0
neo_tab_duration_seconds_bucket{neo_tab_close_reason=\"script\",neo_tab_origin=\"agent\",le=\"10\"} 0
neo_tab_duration_seconds_bucket{neo_tab_close_reason=\"script\",neo_tab_origin=\"agent\",le=\"30\"} 0
neo_tab_duration_seconds_bucket{neo_tab_close_reason=\"script\",neo_tab_origin=\"agent\",le=\"60\"} 1
neo_tab_duration_seconds_bucket{neo_tab_close_reason=\"script\",neo_tab_origin=\"agent\",le=\"300\"} 1
neo_tab_duration_seconds_bucket{neo_tab_close_reason=\"script\",neo_tab_origin=\"agent\",le=\"900\"} 1
neo_tab_duration_seconds_bucket{neo_tab_close_reason=\"script\",neo_tab_origin=\"agent\",le=\"1800\"} 1
neo_tab_duration_seconds_bucket{neo_tab_close_reason=\"script\",neo_tab_origin=\"agent\",le=\"3600\"} 1
neo_tab_duration_seconds_bucket{neo_tab_close_reason=\"script\",neo_tab_origin=\"agent\",le=\"7200\"} 1
neo_tab_duration_seconds_bucket{neo_tab_close_reason=\"script\",neo_tab_origin=\"agent\",le=\"14400\"} 1
neo_tab_duration_seconds_bucket{neo_tab_close_reason=\"script\",neo_tab_origin=\"agent\",le=\"43200\"} 1
neo_tab_duration_seconds_bucket{neo_tab_close_reason=\"script\",neo_tab_origin=\"agent\",le=\"86400\"} 1
neo_tab_duration_seconds_bucket{neo_tab_close_reason=\"script\",neo_tab_origin=\"agent\",le=\"+Inf\"} 1
neo_tab_duration_seconds_sum{neo_tab_close_reason=\"script\",neo_tab_origin=\"agent\"} 45
neo_tab_duration_seconds_count{neo_tab_close_reason=\"script\",neo_tab_origin=\"agent\"} 1
# HELP neo_renderer_crashes_total Renderer crashes, whether or not an op was using the tab.
# TYPE neo_renderer_crashes_total counter
neo_renderer_crashes_total 1
# HELP neo_navigations_total Navigations sent to the browser that ended, by the HTTP status their document arrived with, when one was read.
# TYPE neo_navigations_total counter
neo_navigations_total 1
neo_navigations_total{http_response_status_code=\"404\"} 1
# HELP neo_navigation_errors_total Navigations that failed on the network, by the browser's net error.
# TYPE neo_navigation_errors_total counter
neo_navigation_errors_total{error_type=\"name_not_resolved\"} 1
# HELP neo_captcha_solves_total CAPTCHA solves that returned an outcome, by the CAPTCHA found and how the solve ended.
# TYPE neo_captcha_solves_total counter
neo_captcha_solves_total{neo_captcha_family=\"turnstile\",neo_captcha_verdict=\"solved\"} 1
# HELP neo_captcha_reasons_total CAPTCHA solves that returned an outcome, by why the solve ended as it did.
# TYPE neo_captcha_reasons_total counter
neo_captcha_reasons_total{neo_captcha_reason=\"token_accepted\"} 1
# HELP neo_captcha_spend_total US dollars CAPTCHA providers charged, by the provider each solve booked it to.
# TYPE neo_captcha_spend_total counter
neo_captcha_spend_total{neo_captcha_provider=\"2captcha\"} 0.003
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
            instruments.navigated(Navigated {
                status: HttpStatus::new(404),
                error: None,
            });
            instruments.navigated(Navigated {
                status: None,
                error: Some(NetError::NameNotResolved),
            });
            instruments.captcha_solved(CaptchaSolve {
                family: CaptchaFamily::Turnstile,
                verdict: CaptchaVerdict::Solved,
                reason: CaptchaReason::TokenAccepted,
                spend: Some(CaptchaSpend {
                    provider: CaptchaProvider::TwoCaptcha,
                    usd: 0.003,
                }),
            });
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
                "neo_runs_total{neo_error_owner=\"none\",neo_run_status=\"error\"} 1",
                "neo_runs_total{neo_error_owner=\"none\",neo_run_status=\"ok\"} 3",
                "neo_runs_total{neo_error_owner=\"none\",neo_run_status=\"timeout\"} 1",
            ],
            "series sort by their label text, whatever order they were recorded in"
        );
        let ok_buckets: Vec<u64> = text
            .lines()
            .filter(|line| {
                line.starts_with("neo_run_duration_seconds_bucket{neo_run_status=\"ok\"")
            })
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
            text.contains(
                "neo_run_duration_seconds_bucket{neo_run_status=\"timeout\",le=\"900\"} 0\n"
            ) && text.contains(
                "neo_run_duration_seconds_bucket{neo_run_status=\"timeout\",le=\"+Inf\"} 1\n"
            ),
            "a run longer than the last bound counts only in `+Inf`"
        );
        Ok(())
    }

    #[test]
    fn every_label_name_is_the_attribute_key_translated() {
        for (key, name) in [
            ("neo.run.status", "neo_run_status"),
            ("neo.op.outcome", "neo_op_outcome"),
            ("neo.error.owner", "neo_error_owner"),
            ("error.type", "error_type"),
            ("neo.op.name", "neo_op_name"),
            ("http.response.status_code", "http_response_status_code"),
            ("neo.tab.close_reason", "neo_tab_close_reason"),
            ("neo.captcha.family", "neo_captcha_family"),
        ] {
            assert_eq!(label_name(key), name, "{key}: dots become underscores");
        }
        assert_eq!(
            label_name("otel.metric.overflow"),
            "otel_metric_overflow",
            "the SDK's own overflow label would otherwise break the exposition"
        );
        assert_eq!(label_name("a..b__c"), "a_b_c", "a run of `_` is one");
        assert_eq!(
            label_name("9lives"),
            "key_9lives",
            "a name never starts with a digit"
        );
        assert_eq!(label_name("plain"), "plain", "a valid name is kept");
        assert_eq!(label_name(""), "_", "a name is never empty");
    }

    #[test]
    fn family_names_follow_the_compatibility_rules() {
        let spec = |otel, kind, unit| MetricSpec {
            otel,
            help: "",
            kind,
            unit,
            buckets: &[],
        };
        for (otel, kind, unit, name) in [
            ("neo.runs", Kind::Counter, "{run}", "neo_runs_total"),
            ("neo.run.active", Kind::Gauge, "{run}", "neo_run_active"),
            (
                "neo.run.duration",
                Kind::Histogram,
                "s",
                "neo_run_duration_seconds",
            ),
            (
                "neo.captcha.spend",
                Kind::Counter,
                "{USD}",
                "neo_captcha_spend_total",
            ),
            ("a.b_total", Kind::Counter, "{thing}", "a_b_total"),
            ("a.wait_seconds", Kind::Histogram, "s", "a_wait_seconds"),
            ("a.size", Kind::Gauge, "By", "a_size_bytes"),
            ("a.io", Kind::Counter, "By/s", "a_io_bytes_per_second_total"),
            ("a.odd", Kind::Gauge, "{x}m{y}", "a_odd_meters"),
            ("a-b..c", Kind::Gauge, "", "a_b_c"),
        ] {
            assert_eq!(
                family_name(&spec(otel, kind, unit)),
                name,
                "{otel} ({unit}): unit word, then `_total` for a counter, never twice"
            );
        }
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
