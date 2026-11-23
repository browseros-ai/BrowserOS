//! The live pipeline through the public facade only: each metric's Prometheus text after its facade calls, closed label sets, in-flight across every way a run ends, the open-tabs gauge, both spans, the privacy dial and the shutdown.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::{fmt, thread};

use neo_telemetry::label::{
    CloseReason, Delivery, ErrorCode, ErrorLabel, ErrorOwner, OpOutcome, RunStatus, TabOrigin,
};
use neo_telemetry::{
    CapturedKind, CapturedSpan, OpCounting, OpSettled, OpSite, OpenTabCounts, Probe, RunFacts,
    RunVerdict, SiteDetail, SpanStatus, SpanValue, Telemetry, TelemetryConfig, TraceContext,
    Vocabulary,
};

/// The server's tables, as a test binds them.
const OPS: [&str; 4] = [
    "page.goto",
    "locator.click",
    "page.close",
    "context.newPage",
];
const CODES: [&str; 7] = [
    "no_match",
    "script_error",
    "run_timeout",
    "run_cancelled",
    "browser_unavailable",
    "internal_error",
    "not_awaited",
];

/// A caller's sampled trace, and the same trace marked "not sampled".
const PARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
const UNSAMPLED: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00";

const OK: RunVerdict = RunVerdict {
    status: RunStatus::Ok,
    error: None,
};

fn live(detail: SiteDetail) -> (Telemetry, Probe) {
    let (telemetry, probe) = Telemetry::in_memory(&TelemetryConfig {
        site_detail: detail,
        ..TelemetryConfig::default()
    });
    telemetry.bind_vocabulary(Vocabulary::new(OPS, CODES));
    (telemetry, probe)
}

fn facts(run_id: &dyn fmt::Display) -> RunFacts<'_> {
    RunFacts {
        run_id,
        task_id: None,
        parent: None,
    }
}

fn settled<'a>(op: &'static str, op_id: &'a dyn fmt::Display) -> OpSettled<'a> {
    OpSettled {
        op,
        raw_name: None,
        op_id,
        site: None,
        tab_id: None,
        started: Instant::now(),
        outcome: OpOutcome::Ok,
        delivery: Delivery::Done,
        counting: OpCounting::Counted,
        error: None,
        url: None,
    }
}

const fn failure(owner: ErrorOwner, code: &'static str) -> ErrorLabel {
    ErrorLabel {
        owner,
        code: ErrorCode::new(code),
    }
}

fn ago(elapsed: Duration) -> Result<Instant, String> {
    Instant::now()
        .checked_sub(elapsed)
        .ok_or_else(|| format!("the clock cannot go {elapsed:?} back"))
}

/// Telemetry pushing to a collector that takes connections and never answers, with one
/// finished run waiting to leave on each signal. Keep the listener: dropping it refuses
/// connections, and a refused export ends at once.
fn pushing_to_a_hung_collector() -> Result<(Telemetry, TcpListener), Box<dyn Error>> {
    // Bound and never accepted: the kernel takes the connection and the request; nobody answers.
    let collector = TcpListener::bind("127.0.0.1:0")?;
    let endpoint = format!("http://{}", collector.local_addr()?);
    let config = TelemetryConfig::from_env(&|name| match name {
        "OTEL_EXPORTER_OTLP_ENDPOINT" => Some(endpoint.clone()),
        // Nothing leaves on a timer first, so the run waits for the last flush.
        "OTEL_BSP_SCHEDULE_DELAY" | "OTEL_METRIC_EXPORT_INTERVAL" => Some("600000".to_owned()),
        _ => None,
    })?;
    let telemetry = Telemetry::new(&config)?;
    telemetry.start_run(facts(&"run-1")).finish(OK);
    Ok((telemetry, collector))
}

/// One sample line of the exposition.
#[derive(Debug)]
struct Sample {
    name: String,
    labels: BTreeMap<String, String>,
    value: String,
}

/// An exposition, parsed strictly: every line is `# HELP`, `# TYPE` or a sample whose family
/// was typed before it, or the parse fails.
#[derive(Debug, Default)]
struct Exposition {
    types: BTreeMap<String, String>,
    help: BTreeSet<String>,
    samples: Vec<Sample>,
}

impl Exposition {
    fn parse(text: &str) -> Result<Self, String> {
        let mut exposition = Self::default();
        for line in text.lines() {
            if let Some(help) = line.strip_prefix("# HELP ") {
                let (family, _) = help
                    .split_once(' ')
                    .ok_or_else(|| format!("help without text: {line}"))?;
                exposition.help.insert(family.to_owned());
            } else if let Some(typed) = line.strip_prefix("# TYPE ") {
                let (family, kind) = typed
                    .split_once(' ')
                    .ok_or_else(|| format!("type without kind: {line}"))?;
                if !matches!(kind, "counter" | "gauge" | "histogram") {
                    return Err(format!("unknown type: {line}"));
                }
                exposition.types.insert(family.to_owned(), kind.to_owned());
            } else {
                let sample = parse_sample(line)?;
                let family = family_of(&sample.name, &exposition.types)
                    .ok_or_else(|| format!("sample before its # TYPE: {line}"))?;
                if !exposition.help.contains(family) {
                    return Err(format!("sample before its # HELP: {line}"));
                }
                exposition.samples.push(sample);
            }
        }
        Ok(exposition)
    }

    /// The value of the series `name` with exactly `labels`.
    fn value(&self, name: &str, labels: &[(&str, &str)]) -> Option<&str> {
        let wanted: BTreeMap<String, String> = labels
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        self.samples
            .iter()
            .find(|sample| sample.name == name && sample.labels == wanted)
            .map(|sample| sample.value.as_str())
    }

    /// Every sample line named `name`.
    fn named(&self, name: &str) -> Vec<&Sample> {
        self.samples
            .iter()
            .filter(|sample| sample.name == name)
            .collect()
    }
}

/// The family a sample belongs to: its own name, or a histogram's without the suffix.
fn family_of<'a>(name: &str, types: &'a BTreeMap<String, String>) -> Option<&'a str> {
    if let Some((family, _)) = types.get_key_value(name) {
        return Some(family);
    }
    ["_bucket", "_sum", "_count"].iter().find_map(|suffix| {
        let family = name.strip_suffix(suffix)?;
        let (family, kind) = types.get_key_value(family)?;
        (kind == "histogram").then_some(family.as_str())
    })
}

fn parse_sample(line: &str) -> Result<Sample, String> {
    let (series, value) = line
        .rsplit_once(' ')
        .ok_or_else(|| format!("no value: {line}"))?;
    if !matches!(value, "+Inf" | "-Inf" | "NaN") && value.parse::<f64>().is_err() {
        return Err(format!("not a number: {line}"));
    }
    let (name, labels) = match series.split_once('{') {
        Some((name, labels)) => {
            let inner = labels
                .strip_suffix('}')
                .ok_or_else(|| format!("unclosed labels: {line}"))?;
            (name, parse_labels(inner)?)
        }
        None => (series, BTreeMap::new()),
    };
    if !is_name(name, ":") {
        return Err(format!("bad metric name: {line}"));
    }
    Ok(Sample {
        name: name.to_owned(),
        labels,
        value: value.to_owned(),
    })
}

fn parse_labels(inner: &str) -> Result<BTreeMap<String, String>, String> {
    let mut labels = BTreeMap::new();
    let mut chars = inner.chars().peekable();
    while chars.peek().is_some() {
        let key: String = chars
            .by_ref()
            .take_while(|character| *character != '=')
            .collect();
        if !is_name(&key, "") || chars.next() != Some('"') {
            return Err(format!("bad label start in {inner}"));
        }
        let mut value = String::new();
        loop {
            match chars.next() {
                Some('\\') => match chars.next() {
                    Some('\\') => value.push('\\'),
                    Some('"') => value.push('"'),
                    Some('n') => value.push('\n'),
                    other => return Err(format!("bad escape {other:?} in {inner}")),
                },
                Some('"') => break,
                Some(character) => value.push(character),
                None => return Err(format!("unterminated label value in {inner}")),
            }
        }
        if labels.insert(key, value).is_some() {
            return Err(format!("a label repeats in {inner}"));
        }
        match chars.next() {
            Some(',') | None => {}
            Some(other) => return Err(format!("`{other}` after a label in {inner}")),
        }
    }
    Ok(labels)
}

/// `[a-zA-Z_<extra>][a-zA-Z0-9_<extra>]*`.
fn is_name(name: &str, extra: &str) -> bool {
    let mut chars = name.chars();
    let allowed = |character: char| character == '_' || extra.contains(character);
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || allowed(first))
        && chars.all(|rest| rest.is_ascii_alphanumeric() || allowed(rest))
}

fn scrape(telemetry: &Telemetry) -> Result<Exposition, String> {
    Exposition::parse(&telemetry.render_prometheus())
}

fn the_span<'a>(spans: &'a [CapturedSpan], name: &str) -> Result<&'a CapturedSpan, String> {
    match spans
        .iter()
        .filter(|span| span.name == name)
        .collect::<Vec<_>>()[..]
    {
        [span] => Ok(span),
        ref other => Err(format!("expected one {name} span, got {}", other.len())),
    }
}

/// Whether `span` is an op's span, by the attribute Neo classifies its spans with.
fn is_op(span: &CapturedSpan) -> bool {
    span.attributes.get("neo.span.type") == Some(&text("op"))
}

fn text(value: &str) -> SpanValue {
    SpanValue::Str(value.to_owned())
}

fn attributes(pairs: &[(&str, SpanValue)]) -> BTreeMap<String, SpanValue> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

/// One series `every_metric_reports_its_facade_calls` expects, with its exact labels.
struct Expected {
    series: &'static str,
    labels: &'static [(&'static str, &'static str)],
    value: &'static str,
}

/// What `every_metric_reports_its_facade_calls` expects after its calls.
const EXPECTED: [Expected; 19] = [
    Expected {
        series: "neo_runs_total",
        labels: &[("status", "error"), ("error_owner", "website")],
        value: "1",
    },
    Expected {
        series: "neo_runs_total",
        labels: &[("status", "error"), ("error_owner", "neo")],
        value: "1",
    },
    Expected {
        series: "neo_runs_in_flight",
        labels: &[],
        value: "1",
    },
    Expected {
        series: "neo_run_duration_seconds_count",
        labels: &[("status", "error")],
        value: "1",
    },
    Expected {
        series: "neo_operations_total",
        labels: &[
            ("op", "page.goto"),
            ("status", "ok"),
            ("error_owner", "none"),
        ],
        value: "1",
    },
    Expected {
        series: "neo_operations_total",
        labels: &[
            ("op", "locator.click"),
            ("status", "error"),
            ("error_owner", "website"),
        ],
        value: "1",
    },
    Expected {
        series: "neo_operation_duration_seconds_count",
        labels: &[("op", "page.goto"), ("status", "ok")],
        value: "1",
    },
    Expected {
        series: "neo_operation_duration_seconds_count",
        labels: &[("op", "locator.click"), ("status", "error")],
        value: "1",
    },
    Expected {
        series: "neo_errors_total",
        labels: &[("error_owner", "website"), ("error_code", "no_match")],
        value: "1",
    },
    Expected {
        series: "neo_errors_total",
        labels: &[
            ("error_owner", "neo"),
            ("error_code", "browser_unavailable"),
        ],
        value: "1",
    },
    Expected {
        series: "neo_tabs_open",
        labels: &[("origin", "agent")],
        value: "0",
    },
    Expected {
        series: "neo_tabs_open",
        labels: &[("origin", "found")],
        value: "1",
    },
    Expected {
        series: "neo_tabs_open",
        labels: &[("origin", "neo")],
        value: "1",
    },
    Expected {
        series: "neo_tabs_opened_total",
        labels: &[("origin", "agent")],
        value: "1",
    },
    Expected {
        series: "neo_tabs_opened_total",
        labels: &[("origin", "found")],
        value: "1",
    },
    Expected {
        series: "neo_tabs_closed_total",
        labels: &[("origin", "agent"), ("close_reason", "janitor")],
        value: "1",
    },
    Expected {
        series: "neo_tab_lifetime_seconds_sum",
        labels: &[("origin", "agent"), ("close_reason", "janitor")],
        value: "90",
    },
    Expected {
        series: "neo_tab_lifetime_seconds_count",
        labels: &[("origin", "agent"), ("close_reason", "janitor")],
        value: "1",
    },
    Expected {
        series: "neo_renderer_crashes_total",
        labels: &[],
        value: "1",
    },
];

#[test]
fn every_metric_reports_its_facade_calls() -> Result<(), Box<dyn Error>> {
    let (telemetry, _probe) = live(SiteDetail::None);
    let run = telemetry.start_run(facts(&"run-1"));
    run.ops().op_settled(settled("page.goto", &"run-1.1"));
    run.ops().op_settled(OpSettled {
        outcome: OpOutcome::Error,
        delivery: Delivery::NotSent,
        counting: OpCounting::Counted,
        error: Some(failure(ErrorOwner::Website, "no_match")),
        ..settled("locator.click", &"run-1.2")
    });
    run.finish(RunVerdict {
        status: RunStatus::Error,
        error: Some(failure(ErrorOwner::Website, "no_match")),
    });
    let open = telemetry.start_run(facts(&"run-2"));
    telemetry.run_refused(failure(ErrorOwner::Neo, "browser_unavailable"));
    telemetry.tab_opened(TabOrigin::Agent);
    telemetry.tab_opened(TabOrigin::Found);
    telemetry.tab_closed(
        TabOrigin::Agent,
        CloseReason::Janitor,
        Duration::from_secs(90),
    );
    telemetry.renderer_crashed();
    telemetry.set_open_tabs(Arc::new(|| OpenTabCounts {
        agent: 0,
        found: 1,
        neo: 1,
    }));

    let metrics = scrape(&telemetry)?;
    let types: Vec<(&str, &str)> = metrics
        .types
        .iter()
        .map(|(family, kind)| (family.as_str(), kind.as_str()))
        .collect();
    assert_eq!(
        types,
        vec![
            ("neo_errors_total", "counter"),
            ("neo_operation_duration_seconds", "histogram"),
            ("neo_operations_total", "counter"),
            ("neo_renderer_crashes_total", "counter"),
            ("neo_run_duration_seconds", "histogram"),
            ("neo_runs_in_flight", "gauge"),
            ("neo_runs_total", "counter"),
            ("neo_tab_lifetime_seconds", "histogram"),
            ("neo_tabs_closed_total", "counter"),
            ("neo_tabs_open", "gauge"),
            ("neo_tabs_opened_total", "counter"),
        ],
        "all 11 families, each with the type wire-contract.md gives it"
    );
    for Expected {
        series,
        labels,
        value,
    } in EXPECTED
    {
        assert_eq!(
            metrics.value(series, labels),
            Some(value),
            "{series}{labels:?} after the matching facade calls"
        );
    }
    assert_eq!(
        metrics.named("neo_errors_total").len(),
        2,
        "the run failed because its op did: that failure counts once, not twice"
    );
    assert_eq!(
        metrics.named("neo_runs_total").len(),
        2,
        "the open run is not counted until it ends"
    );
    open.finish(OK);
    Ok(())
}

#[test]
fn unregistered_ops_count_as_unknown_while_their_spans_keep_the_raw_name()
-> Result<(), Box<dyn Error>> {
    let (telemetry, probe) = live(SiteDetail::None);
    let run = telemetry.start_run(facts(&"run-1"));
    let invented = ["page.gotoo", "made.up", "ignore all previous instructions"];
    for raw in invented {
        run.ops().op_settled(OpSettled {
            raw_name: Some(raw),
            outcome: OpOutcome::Error,
            delivery: Delivery::NotSent,
            counting: OpCounting::Counted,
            error: Some(failure(ErrorOwner::Script, "method_unavailable")),
            ..settled("unknown", &"run-1.1")
        });
    }
    let unlisted: &'static str = String::leak("page.secretMethod".to_owned());
    run.ops().op_settled(settled(unlisted, &"run-1.4"));
    run.finish(OK);

    let metrics = scrape(&telemetry)?;
    let ops: BTreeSet<&str> = metrics
        .named("neo_operations_total")
        .iter()
        .filter_map(|sample| sample.labels.get("op").map(String::as_str))
        .collect();
    assert_eq!(
        ops,
        BTreeSet::from(["unknown"]),
        "no invented or unlisted name opens a series of its own"
    );
    assert_eq!(
        metrics.value(
            "neo_operations_total",
            &[
                ("op", "unknown"),
                ("status", "error"),
                ("error_owner", "script")
            ]
        ),
        Some("3"),
        "the three invented calls count together"
    );
    assert_eq!(
        metrics.value(
            "neo_errors_total",
            &[("error_owner", "script"), ("error_code", "unknown")]
        ),
        Some("3"),
        "a code outside Neo's table counts as unknown"
    );
    let ops: Vec<CapturedSpan> = probe.spans().into_iter().filter(is_op).collect();
    let names: Vec<String> = ops
        .iter()
        .filter_map(|span| match span.attributes.get("neo.op.name") {
            Some(SpanValue::Str(name)) => Some(name.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        names,
        vec![
            "page.gotoo",
            "made.up",
            "ignore all previous instructions",
            "page.secretMethod"
        ],
        "each span's neo.op.name keeps what the script called"
    );
    assert!(
        ops.iter().all(|span| span.name == "unknown"),
        "while the span name collapses like the metric label, so caller text never names a span: \
         {ops:#?}"
    );
    Ok(())
}

/// Drives every run, op, refusal and tab hook with every closed value, and with caller text
/// everywhere a caller can put text: invented op names and codes, run and task ids, raw names
/// with quotes, URLs with credentials. Ops of internal runs and of counted ones alike.
fn drive_every_hook_with_hostile_text(telemetry: &Telemetry) {
    let owners = [
        ErrorOwner::Website,
        ErrorOwner::Script,
        ErrorOwner::Neo,
        ErrorOwner::Host,
    ];
    let statuses = [
        RunStatus::Ok,
        RunStatus::Error,
        RunStatus::Cancelled,
        RunStatus::Timeout,
    ];
    let outcomes = [OpOutcome::Ok, OpOutcome::Error, OpOutcome::Cancelled];
    let calls = (0..240_usize)
        .zip(owners.into_iter().cycle())
        .zip(statuses.into_iter().cycle())
        .zip(outcomes.into_iter().cycle())
        .zip(OPS.into_iter().cycle());
    for ((((index, owner), status), outcome), registered) in calls {
        let run_id = format!("run-secret-{index}");
        let run = if index % 5 == 0 {
            telemetry.internal_run()
        } else {
            telemetry.start_run(RunFacts {
                run_id: &run_id,
                task_id: Some("task-secret"),
                parent: None,
            })
        };
        let invented: &'static str = String::leak(format!("op_invented_{index}"));
        let error = ErrorLabel {
            owner,
            code: ErrorCode::new(String::leak(format!("code_invented_{index}"))),
        };
        let raw = format!("{invented}\" injected=\"1");
        let op_id = format!("{run_id}.1");
        run.ops().op_settled(OpSettled {
            raw_name: Some(&raw),
            site: Some(OpSite { line: 7, column: 9 }),
            tab_id: i64::try_from(index).ok(),
            outcome,
            error: Some(error),
            url: Some("https://user:pw@secret.example/path?token=abc#frag"),
            ..settled(if index % 2 == 0 { invented } else { registered }, &op_id)
        });
        run.finish(RunVerdict {
            status,
            error: Some(error),
        });
        telemetry.run_refused(error);
    }
    for origin in [TabOrigin::Agent, TabOrigin::Found, TabOrigin::Neo] {
        telemetry.tab_opened(origin);
        for reason in [
            CloseReason::Script,
            CloseReason::Janitor,
            CloseReason::User,
            CloseReason::Crash,
            CloseReason::Shutdown,
        ] {
            telemetry.tab_closed(origin, reason, Duration::from_secs(5));
        }
    }
}

/// Every sample whose label names are not exactly its family's, or whose label value is outside
/// the contract's closed set for that label.
fn label_violations(metrics: &Exposition) -> Vec<String> {
    let ops: Vec<&str> = OPS.iter().copied().chain(["unknown"]).collect();
    let codes: Vec<&str> = CODES.iter().copied().chain(["unknown"]).collect();
    let allowed: BTreeMap<&str, Vec<&str>> = BTreeMap::from([
        ("status", vec!["ok", "error", "cancelled", "timeout"]),
        (
            "error_owner",
            vec!["website", "script", "neo", "host", "none"],
        ),
        ("error_code", codes),
        ("op", ops),
        ("origin", vec!["agent", "found", "neo"]),
        (
            "close_reason",
            vec!["script", "janitor", "user", "crash", "shutdown"],
        ),
    ]);
    let keys: BTreeMap<&str, Vec<&str>> = BTreeMap::from([
        ("neo_runs_total", vec!["error_owner", "status"]),
        ("neo_runs_in_flight", vec![]),
        ("neo_run_duration_seconds", vec!["status"]),
        ("neo_operations_total", vec!["error_owner", "op", "status"]),
        ("neo_operation_duration_seconds", vec!["op", "status"]),
        ("neo_errors_total", vec!["error_code", "error_owner"]),
        ("neo_tabs_open", vec!["origin"]),
        ("neo_tabs_opened_total", vec!["origin"]),
        ("neo_tabs_closed_total", vec!["close_reason", "origin"]),
        ("neo_tab_lifetime_seconds", vec!["close_reason", "origin"]),
        ("neo_renderer_crashes_total", vec![]),
    ]);
    let mut violations = Vec::new();
    for sample in &metrics.samples {
        let family = family_of(&sample.name, &metrics.types).unwrap_or_default();
        let bucket = sample.name.ends_with("_bucket");
        let names: Vec<&str> = sample
            .labels
            .keys()
            .map(String::as_str)
            .filter(|key| !(bucket && *key == "le"))
            .collect();
        if keys.get(family) != Some(&names) {
            violations.push(format!("{}: label names {names:?}", sample.name));
        }
        for (key, value) in &sample.labels {
            let bound = bucket && key == "le";
            let closed = allowed
                .get(key.as_str())
                .is_some_and(|values| values.contains(&value.as_str()));
            if !(bound || closed) {
                violations.push(format!("{}: {key}=\"{value}\"", sample.name));
            }
        }
    }
    violations
}

#[test]
fn no_label_outside_the_closed_sets_reaches_the_exposition() -> Result<(), Box<dyn Error>> {
    let (telemetry, _probe) = live(SiteDetail::Url);
    drive_every_hook_with_hostile_text(&telemetry);
    let exposition = telemetry.render_prometheus();
    for secret in ["secret", "token", "injected", "invented", "pw@", "frag"] {
        assert!(
            !exposition.contains(secret),
            "`{secret}` came from a caller and must never reach a metric"
        );
    }
    let metrics = Exposition::parse(&exposition)?;
    let violations = label_violations(&metrics);
    assert!(
        violations.is_empty(),
        "every label name and value is the contract's: {violations:#?}"
    );
    let ops_series = metrics.named("neo_operations_total").len();
    assert!(
        ops_series <= (OPS.len() + 1) * 3 * 5,
        "240 invented names and codes still fit the closed budget: {ops_series} series"
    );
    assert!(
        metrics.named("neo_errors_total").len() <= 4 * (CODES.len() + 1),
        "error series stay within owners x (codes + unknown)"
    );
    Ok(())
}

#[test]
fn histograms_are_cumulative_and_end_in_inf_equal_to_count() -> Result<(), Box<dyn Error>> {
    let (telemetry, _probe) = live(SiteDetail::None);
    let sweep = telemetry.internal_run();
    for millis in [1, 7, 30, 30, 450, 2_000, 61_000, 500_000] {
        sweep.ops().op_settled(OpSettled {
            started: ago(Duration::from_millis(millis))?,
            ..settled("page.goto", &"run-1.1")
        });
    }
    for seconds in [1, 30, 4_000, 200_000] {
        telemetry.tab_closed(
            TabOrigin::Agent,
            CloseReason::User,
            Duration::from_secs(seconds),
        );
    }
    sweep.finish(OK);
    let metrics = scrape(&telemetry)?;
    for family in ["neo_operation_duration_seconds", "neo_tab_lifetime_seconds"] {
        let buckets = metrics.named(&format!("{family}_bucket"));
        let mut bounds = Vec::new();
        let mut counts = Vec::new();
        for bucket in &buckets {
            let bound = bucket.labels.get("le").ok_or("a bucket without `le`")?;
            bounds.push(if bound == "+Inf" {
                f64::INFINITY
            } else {
                bound.parse::<f64>()?
            });
            counts.push(bucket.value.parse::<u64>()?);
        }
        assert!(
            bounds.windows(2).all(|pair| pair[0] < pair[1]),
            "{family}: `le` rises strictly: {bounds:?}"
        );
        assert_eq!(
            buckets
                .last()
                .and_then(|bucket| bucket.labels.get("le"))
                .map(String::as_str),
            Some("+Inf"),
            "{family}: the last bucket is +Inf"
        );
        assert!(
            counts.windows(2).all(|pair| pair[0] <= pair[1]),
            "{family}: counts are cumulative: {counts:?}"
        );
        let count = metrics.named(&format!("{family}_count"));
        assert_eq!(
            count
                .first()
                .map(|sample| sample.value.parse::<u64>())
                .transpose()?,
            counts.last().copied(),
            "{family}: +Inf equals _count"
        );
        assert_eq!(
            metrics.named(&format!("{family}_sum")).len(),
            1,
            "{family}: one _sum"
        );
    }
    assert_eq!(
        metrics.value(
            "neo_operation_duration_seconds_count",
            &[("op", "page.goto"), ("status", "ok")]
        ),
        Some("8"),
        "every op observation is counted"
    );
    assert_eq!(
        metrics.value(
            "neo_tab_lifetime_seconds_sum",
            &[("origin", "agent"), ("close_reason", "user")]
        ),
        Some("204031"),
        "the sum is the lifetimes given"
    );
    assert_eq!(
        metrics.value(
            "neo_tab_lifetime_seconds_bucket",
            &[
                ("origin", "agent"),
                ("close_reason", "user"),
                ("le", "86400")
            ]
        ),
        Some("3"),
        "a tab kept longer than the last bound counts only in +Inf"
    );
    Ok(())
}

#[test]
fn in_flight_returns_to_zero_after_finish_drop_and_panic() -> Result<(), Box<dyn Error>> {
    let (telemetry, _probe) = live(SiteDetail::None);
    let in_flight = |telemetry: &Telemetry| -> Result<String, String> {
        scrape(telemetry)?
            .value("neo_runs_in_flight", &[])
            .map(str::to_owned)
            .ok_or_else(|| "no neo_runs_in_flight sample".to_owned())
    };
    assert_eq!(
        in_flight(&telemetry)?,
        "0",
        "the gauge shows zero before any run"
    );
    let finished = telemetry.start_run(facts(&"run-1"));
    let dropped = telemetry.start_run(facts(&"run-2"));
    assert_eq!(in_flight(&telemetry)?, "2", "both runs are in flight");
    finished.finish(OK);
    assert_eq!(
        in_flight(&telemetry)?,
        "1",
        "a finished run leaves in-flight"
    );
    drop(dropped);
    assert_eq!(
        in_flight(&telemetry)?,
        "0",
        "a run dropped unfinished leaves it too"
    );
    let handle = telemetry.clone();
    let panicked = thread::spawn(move || {
        let _run = handle.start_run(facts(&"run-3"));
        panic!("the run's task panicked");
    })
    .join();
    assert!(panicked.is_err(), "the run's thread panicked");
    assert_eq!(
        in_flight(&telemetry)?,
        "0",
        "unwinding drops the run, which leaves in-flight"
    );
    let metrics = scrape(&telemetry)?;
    assert_eq!(
        metrics.value(
            "neo_runs_total",
            &[("status", "error"), ("error_owner", "neo")]
        ),
        Some("2"),
        "a dropped run and a panicked one end as Neo's own failure"
    );
    assert_eq!(
        metrics.value(
            "neo_errors_total",
            &[("error_owner", "neo"), ("error_code", "internal_error")]
        ),
        Some("2"),
        "each counted once, as internal_error"
    );
    Ok(())
}

#[test]
fn the_open_tabs_gauge_reads_the_live_source_at_each_scrape() -> Result<(), Box<dyn Error>> {
    let (telemetry, _probe) = live(SiteDetail::None);
    assert!(
        scrape(&telemetry)?.named("neo_tabs_open").is_empty(),
        "with no source registered the gauge reports nothing, not a made-up zero"
    );
    let agent_tabs = Arc::new(AtomicU64::new(3));
    let source = Arc::clone(&agent_tabs);
    telemetry.set_open_tabs(Arc::new(move || OpenTabCounts {
        agent: source.load(Ordering::Relaxed),
        found: 2,
        neo: 1,
    }));
    let open = |metrics: &Exposition, origin: &str| {
        metrics
            .value("neo_tabs_open", &[("origin", origin)])
            .map(str::to_owned)
    };
    let first = scrape(&telemetry)?;
    assert_eq!(
        open(&first, "agent").as_deref(),
        Some("3"),
        "read from the source"
    );
    assert_eq!(
        open(&first, "found").as_deref(),
        Some("2"),
        "every origin is read"
    );
    assert_eq!(
        open(&first, "neo").as_deref(),
        Some("1"),
        "every origin is read"
    );
    agent_tabs.store(0, Ordering::Relaxed);
    assert_eq!(
        open(&scrape(&telemetry)?, "agent").as_deref(),
        Some("0"),
        "each scrape reads the live count: it follows the registry, never a running sum"
    );
    Ok(())
}

#[test]
fn an_op_span_is_a_child_of_neo_run_which_is_a_child_of_the_callers_trace() {
    let (telemetry, probe) = live(SiteDetail::None);
    let parent = TraceContext::from_w3c(PARENT, Some("vendor=1"));
    let run = telemetry.start_run(RunFacts {
        run_id: &"run-1",
        task_id: None,
        parent: parent.as_ref(),
    });
    run.ops().op_settled(settled("page.goto", &"run-1.1"));
    run.finish(OK);
    let fresh = telemetry.start_run(facts(&"run-2"));
    fresh.ops().op_settled(settled("page.goto", &"run-2.1"));
    fresh.finish(OK);

    let spans = probe.spans();
    let [op, run, fresh_op, fresh] = spans.as_slice() else {
        panic!("two ops then their runs, in the order they ended: {spans:#?}");
    };
    assert_eq!(
        (
            run.name.as_str(),
            run.trace_id.as_str(),
            run.parent_span_id.as_deref()
        ),
        (
            "neo.run",
            "4bf92f3577b34da6a3ce929d0e0e4736",
            Some("00f067aa0ba902b7")
        ),
        "neo.run joins the caller's trace, under the caller's span"
    );
    assert!(run.parent_is_remote, "its parent is the caller's, not ours");
    assert_eq!(
        (run.kind, op.kind),
        (CapturedKind::Server, CapturedKind::Internal),
        "neo.run serves the caller's call, so it pairs with the caller's client span; an op is \
         in-process work under it"
    );
    assert_eq!(
        (
            op.name.as_str(),
            op.trace_id.as_str(),
            op.parent_span_id.as_deref()
        ),
        (
            "page.goto",
            run.trace_id.as_str(),
            Some(run.span_id.as_str())
        ),
        "the op's span, named after the op, is a child of its run's neo.run"
    );
    assert!(
        !op.parent_is_remote,
        "the op's parent is a span of this process"
    );
    assert_eq!(
        fresh.parent_span_id, None,
        "with no caller trace, neo.run is a root"
    );
    assert_ne!(
        fresh.trace_id, run.trace_id,
        "and starts a trace of its own"
    );
    assert_eq!(
        fresh_op.parent_span_id.as_deref(),
        Some(fresh.span_id.as_str()),
        "its op hangs under it"
    );
}

#[test]
fn span_attributes_and_status_match_the_wire_contract() -> Result<(), Box<dyn Error>> {
    let (telemetry, probe) = live(SiteDetail::None);
    let run = telemetry.start_run(RunFacts {
        run_id: &"run-8",
        task_id: Some("task-8"),
        parent: None,
    });
    run.ops().op_settled(OpSettled {
        site: Some(OpSite {
            line: 3,
            column: 14,
        }),
        tab_id: Some(42),
        started: ago(Duration::from_millis(250))?,
        outcome: OpOutcome::Error,
        delivery: Delivery::Unknown,
        counting: OpCounting::Counted,
        error: Some(failure(ErrorOwner::Website, "no_match")),
        url: Some("https://shop.example.com/cart?id=1"),
        ..settled("locator.click", &"run-8.1")
    });
    run.ops().op_settled(settled("page.goto", &"run-8.2"));
    run.finish(RunVerdict {
        status: RunStatus::Error,
        error: Some(failure(ErrorOwner::Script, "script_error")),
    });

    let spans = probe.spans();
    let names: BTreeSet<&str> = spans.iter().map(|span| span.name.as_str()).collect();
    assert_eq!(
        names,
        BTreeSet::from(["locator.click", "neo.run", "page.goto"]),
        "the run's span, and each op's named after its op"
    );
    let [failed, ok, run] = spans.as_slice() else {
        panic!("two ops then the run: {spans:#?}");
    };
    assert_eq!(
        failed.attributes,
        attributes(&[
            ("neo.span.type", text("op")),
            ("neo.op.name", text("locator.click")),
            ("neo.op.id", text("run-8.1")),
            ("neo.op.site.line", SpanValue::Int(3)),
            ("neo.op.site.column", SpanValue::Int(14)),
            ("neo.tab.id", SpanValue::Int(42)),
            ("neo.op.outcome", text("error")),
            ("neo.op.delivery", text("unknown")),
            ("neo.error.owner", text("website")),
            ("neo.error.code", text("no_match")),
        ]),
        "a failed op: every contract attribute, and no URL under the default dial"
    );
    assert_eq!(
        failed.status,
        SpanStatus::Error {
            description: "no_match".to_owned()
        },
        "a failed op's span is an error"
    );
    assert!(
        failed.end.duration_since(failed.start)? >= Duration::from_millis(250),
        "the op span starts when the op started, not when it settled"
    );
    assert_eq!(
        ok.attributes,
        attributes(&[
            ("neo.span.type", text("op")),
            ("neo.op.name", text("page.goto")),
            ("neo.op.id", text("run-8.2")),
            ("neo.op.outcome", text("ok")),
            ("neo.op.delivery", text("done")),
        ]),
        "an ok op with no site or tab carries neither, and no error"
    );
    assert_eq!(ok.status, SpanStatus::Unset, "an ok op's status is unset");
    assert_eq!(
        run.attributes,
        attributes(&[
            ("neo.span.type", text("run")),
            ("neo.run.id", text("run-8")),
            ("neo.task.id", text("task-8")),
            ("neo.run.status", text("error")),
            ("neo.error.owner", text("script")),
            ("neo.error.code", text("script_error")),
        ]),
        "neo.run carries its type, the run id, the task id, its status and its failure"
    );
    assert_eq!(
        run.status,
        SpanStatus::Error {
            description: "script_error".to_owned()
        },
        "a run that did not end ok is an error"
    );
    Ok(())
}

#[test]
fn the_privacy_dial_decides_what_of_the_url_a_span_keeps() -> Result<(), Box<dyn Error>> {
    let url = Some("https://alice:pw@shop.example.com:8443/cart?session=abc#pay");
    for (detail, domain, full) in [
        (SiteDetail::None, None, None),
        (SiteDetail::Host, Some("shop.example.com"), None),
        (
            SiteDetail::Url,
            Some("shop.example.com"),
            Some("https://shop.example.com:8443/cart"),
        ),
    ] {
        let (telemetry, probe) = live(detail);
        let run = telemetry.start_run(facts(&"run-1"));
        run.ops().op_settled(OpSettled {
            url,
            ..settled("page.goto", &"run-1.1")
        });
        run.finish(OK);
        let spans = probe.spans();
        let op = the_span(&spans, "page.goto")?;
        assert_eq!(
            op.attributes.get("url.domain"),
            domain.map(text).as_ref(),
            "{detail:?}: url.domain"
        );
        assert_eq!(
            op.attributes.get("url.full"),
            full.map(text).as_ref(),
            "{detail:?}: url.full, with userinfo, query and fragment gone"
        );
        let every_value: Vec<&SpanValue> = spans
            .iter()
            .flat_map(|span| span.attributes.values())
            .collect();
        assert!(
            !format!("{every_value:?}").contains("session=abc")
                && !format!("{every_value:?}").contains("alice"),
            "{detail:?}: no span keeps the query string or the credentials"
        );
        assert!(
            !telemetry.render_prometheus().contains("shop.example.com"),
            "{detail:?}: no metric ever carries a URL or a host"
        );
    }
    Ok(())
}

#[test]
fn an_internal_runs_ops_count_and_their_spans_are_roots() -> Result<(), Box<dyn Error>> {
    let (telemetry, probe) = live(SiteDetail::None);
    let sweep = telemetry.internal_run();
    sweep.ops().op_settled(OpSettled {
        outcome: OpOutcome::Error,
        error: Some(failure(ErrorOwner::Neo, "internal_error")),
        ..settled("page.close", &"run-1.1")
    });
    sweep.finish(OK);
    let metrics = scrape(&telemetry)?;
    assert!(
        metrics.named("neo_runs_total").is_empty()
            && metrics.named("neo_run_duration_seconds_count").is_empty(),
        "Neo's own runs leave the run metrics alone"
    );
    assert_eq!(
        metrics.value("neo_runs_in_flight", &[]),
        Some("0"),
        "and are never in flight"
    );
    assert_eq!(
        metrics.value(
            "neo_operations_total",
            &[
                ("op", "page.close"),
                ("status", "error"),
                ("error_owner", "neo")
            ]
        ),
        Some("1"),
        "their ops still count"
    );
    assert_eq!(
        metrics.value(
            "neo_errors_total",
            &[("error_owner", "neo"), ("error_code", "internal_error")]
        ),
        Some("1"),
        "and so do their failures"
    );
    let spans = probe.spans();
    let op = the_span(&spans, "page.close")?;
    assert_eq!(
        op.parent_span_id, None,
        "with no neo.run, the op's span is a root"
    );
    assert_eq!(spans.len(), 1, "an internal run has no span of its own");
    Ok(())
}

#[test]
fn neo_internal_ops_leave_every_op_metric_and_keep_their_spans() -> Result<(), Box<dyn Error>> {
    let (telemetry, probe) = live(SiteDetail::None);
    let gone = failure(ErrorOwner::Website, "no_match");
    let run = telemetry.start_run(facts(&"run-1"));
    let ops = run.ops();
    ops.op_settled(OpSettled {
        counting: OpCounting::Internal,
        ..settled("page.goto", &"run-1.1")
    });
    ops.op_settled(OpSettled {
        counting: OpCounting::Internal,
        outcome: OpOutcome::Error,
        error: Some(gone),
        ..settled("locator.click", &"run-1.2")
    });
    run.finish(RunVerdict {
        status: RunStatus::Error,
        error: Some(gone),
    });
    let metrics = scrape(&telemetry)?;
    assert!(
        metrics.named("neo_operations_total").is_empty()
            && metrics
                .named("neo_operation_duration_seconds_count")
                .is_empty(),
        "housekeeping grows with open tabs, so it never reaches an op rate"
    );
    assert_eq!(
        metrics.value(
            "neo_errors_total",
            &[("error_owner", "website"), ("error_code", "no_match")]
        ),
        Some("1"),
        "the internal op's failure is not counted, so the run that ended on it counts it once"
    );
    assert_eq!(
        metrics.value(
            "neo_runs_total",
            &[("status", "error"), ("error_owner", "website")]
        ),
        Some("1"),
        "the run itself is counted as usual"
    );
    let spans = probe.spans();
    let run_span = the_span(&spans, "neo.run")?;
    let op_spans: Vec<&CapturedSpan> = spans
        .iter()
        .filter(|span| span.parent_span_id.as_ref() == Some(&run_span.span_id))
        .collect();
    assert_eq!(
        op_spans.len(),
        2,
        "both ops keep a span under their run, so the trace still shows Neo's own work"
    );
    Ok(())
}

#[test]
fn an_unsampled_caller_trace_keeps_its_metrics_but_exports_no_span() -> Result<(), Box<dyn Error>> {
    let (telemetry, probe) = live(SiteDetail::None);
    let parent = TraceContext::from_w3c(UNSAMPLED, None);
    let run = telemetry.start_run(RunFacts {
        run_id: &"run-1",
        task_id: None,
        parent: parent.as_ref(),
    });
    run.ops().op_settled(settled("page.goto", &"run-1.1"));
    run.finish(OK);
    assert!(
        probe.spans().is_empty(),
        "parent-based sampling honours the caller's \"not sampled\" for the run and its ops"
    );
    assert_eq!(
        scrape(&telemetry)?.value(
            "neo_runs_total",
            &[("status", "ok"), ("error_owner", "none")]
        ),
        Some("1"),
        "sampling never touches the metrics"
    );
    Ok(())
}

#[test]
fn each_failure_counts_once_where_it_is_attributed() -> Result<(), Box<dyn Error>> {
    let (telemetry, _probe) = live(SiteDetail::None);
    let no_match = failure(ErrorOwner::Website, "no_match");
    let timeout = failure(ErrorOwner::Script, "run_timeout");
    let failed_op = |code: ErrorLabel| OpSettled {
        outcome: OpOutcome::Error,
        error: Some(code),
        ..settled("locator.click", &"run.1")
    };

    let propagated = telemetry.start_run(facts(&"run-a"));
    propagated.ops().op_settled(failed_op(no_match));
    propagated.finish(RunVerdict {
        status: RunStatus::Error,
        error: Some(no_match),
    });

    let own = telemetry.start_run(facts(&"run-b"));
    own.ops().op_settled(failed_op(no_match));
    own.finish(RunVerdict {
        status: RunStatus::Timeout,
        error: Some(timeout),
    });

    let teardown = telemetry.start_run(facts(&"run-c"));
    let late = teardown.ops();
    teardown.finish(RunVerdict {
        status: RunStatus::Timeout,
        error: Some(timeout),
    });
    late.op_settled(OpSettled {
        outcome: OpOutcome::Cancelled,
        ..failed_op(timeout)
    });
    late.op_settled(failed_op(no_match));

    let metrics = scrape(&telemetry)?;
    assert_eq!(
        metrics.value(
            "neo_errors_total",
            &[("error_owner", "website"), ("error_code", "no_match")]
        ),
        Some("3"),
        "every op failure counts, the late one too; run-a's own end repeats its op's and adds none"
    );
    assert_eq!(
        metrics.value(
            "neo_errors_total",
            &[("error_owner", "script"), ("error_code", "run_timeout")]
        ),
        Some("2"),
        "run-b's and run-c's timeouts are their own; the op aborted by run-c's adds none"
    );
    Ok(())
}

#[test]
fn an_op_that_settles_after_its_run_ended_still_counts_and_nests() -> Result<(), Box<dyn Error>> {
    let (telemetry, probe) = live(SiteDetail::None);
    let crash = failure(ErrorOwner::Neo, "internal_error");
    let run = telemetry.start_run(facts(&"run-1"));
    let ops = run.ops();
    run.finish(RunVerdict {
        status: RunStatus::Error,
        error: Some(crash),
    });
    // A crash cut the op short: the door settles it after the run's guard already ended the run.
    ops.op_settled(OpSettled {
        outcome: OpOutcome::Cancelled,
        delivery: Delivery::Unknown,
        counting: OpCounting::Counted,
        error: Some(crash),
        ..settled("page.goto", &"run-1.1")
    });
    let metrics = scrape(&telemetry)?;
    assert_eq!(
        metrics.value(
            "neo_operations_total",
            &[
                ("op", "page.goto"),
                ("status", "cancelled"),
                ("error_owner", "neo")
            ]
        ),
        Some("1"),
        "the late op still counts"
    );
    assert_eq!(
        metrics.value(
            "neo_errors_total",
            &[("error_owner", "neo"), ("error_code", "internal_error")]
        ),
        Some("1"),
        "the crash is one failure: the run's end counted it and the op it cut short adds none"
    );
    assert_eq!(
        metrics.value("neo_runs_in_flight", &[]),
        Some("0"),
        "a late op never touches in-flight"
    );
    let spans = probe.spans();
    let run = the_span(&spans, "neo.run")?;
    let op = the_span(&spans, "page.goto")?;
    assert_eq!(
        (op.trace_id.as_str(), op.parent_span_id.as_deref()),
        (run.trace_id.as_str(), Some(run.span_id.as_str())),
        "the late op's span still nests under its run's"
    );
    Ok(())
}

#[test]
fn a_stop_counts_once_however_many_ops_it_cut_short() -> Result<(), Box<dyn Error>> {
    let (telemetry, probe) = live(SiteDetail::None);
    let timeout = failure(ErrorOwner::Neo, "run_timeout");
    let run = telemetry.start_run(facts(&"run-1"));
    // The deadline cuts three hung ops, and teardown settles them before the run's guard ends
    // the run: the real order of a timeout.
    for op_id in ["run-1.1", "run-1.2", "run-1.3"] {
        run.ops().op_settled(OpSettled {
            outcome: OpOutcome::Cancelled,
            delivery: Delivery::Unknown,
            counting: OpCounting::Counted,
            error: Some(timeout),
            ..settled("locator.click", &op_id)
        });
    }
    run.finish(RunVerdict {
        status: RunStatus::Timeout,
        error: Some(timeout),
    });
    // An ok run whose script returned without awaiting an op: the leftover is cut.
    let ok = telemetry.start_run(facts(&"run-2"));
    ok.ops().op_settled(OpSettled {
        outcome: OpOutcome::Cancelled,
        delivery: Delivery::NotSent,
        counting: OpCounting::Counted,
        error: Some(failure(ErrorOwner::Script, "not_awaited")),
        ..settled("page.goto", &"run-2.1")
    });
    ok.finish(OK);

    let metrics = scrape(&telemetry)?;
    assert_eq!(
        metrics.value(
            "neo_errors_total",
            &[("error_owner", "neo"), ("error_code", "run_timeout")]
        ),
        Some("1"),
        "the deadline is one failure, counted at the run, not once per op it cut"
    );
    assert_eq!(
        metrics.named("neo_errors_total").len(),
        1,
        "the ok run's cut leftover counts in no error total: that run succeeded"
    );
    assert_eq!(
        metrics.value(
            "neo_operations_total",
            &[
                ("op", "locator.click"),
                ("status", "cancelled"),
                ("error_owner", "neo")
            ]
        ),
        Some("3"),
        "each cut op still counts, as cancelled, with its error's owner"
    );
    let cut: Vec<CapturedSpan> = probe
        .spans()
        .into_iter()
        .filter(|span| span.attributes.get("neo.op.name") == Some(&text("locator.click")))
        .collect();
    assert_eq!(cut.len(), 3, "one span per cut op");
    assert!(
        cut.iter().all(|span| {
            span.attributes.get("neo.error.code") == Some(&text("run_timeout"))
                && span.attributes.get("neo.op.outcome") == Some(&text("cancelled"))
                && span.status == SpanStatus::Unset
        }),
        "each cut op's span keeps the error that cut it, and is no error span itself"
    );
    Ok(())
}

#[test]
fn an_ok_run_whose_own_long_poll_was_cut_has_no_error_span() -> Result<(), Box<dyn Error>> {
    let (telemetry, probe) = live(SiteDetail::None);
    let run = telemetry.start_run(facts(&"run-1"));
    run.ops().op_settled(settled("page.goto", &"run-1.1"));
    // Neo's own long-poll, cut when the script returns: it ends this way in every run.
    run.ops().op_settled(OpSettled {
        outcome: OpOutcome::Cancelled,
        delivery: Delivery::NotSent,
        counting: OpCounting::Counted,
        ..settled("events.next", &"run-1.2")
    });
    run.finish(OK);
    let spans = probe.spans();
    assert_eq!(spans.len(), 3, "two ops and the run");
    assert!(
        spans.iter().all(|span| span.status == SpanStatus::Unset),
        "an ok run's trace has no error span, though its long-poll was cut: {spans:#?}"
    );
    let poll = spans
        .iter()
        .find(|span| span.attributes.get("neo.op.name") == Some(&text("events.next")))
        .ok_or("the long-poll's span")?;
    assert_eq!(
        poll.attributes.get("neo.op.outcome"),
        Some(&text("cancelled")),
        "its outcome still says it was cut"
    );
    assert!(
        !poll.attributes.contains_key("neo.error.code"),
        "with no error label, a cut op carries no error attributes"
    );
    Ok(())
}

#[test]
fn dropping_pushing_telemetry_without_shutdown_never_waits_on_its_collector()
-> Result<(), Box<dyn Error>> {
    let (telemetry, _collector) = pushing_to_a_hung_collector()?;
    let started = Instant::now();
    drop(telemetry);
    let took = started.elapsed();
    assert!(
        took < Duration::from_secs(1),
        "dropped without shutdown (a start that failed later, say), telemetry waited {took:?} on \
         a collector that never answers; the SDK's own drops wait about 5 s each"
    );
    Ok(())
}

#[tokio::test]
async fn a_hung_collector_still_lets_shutdown_report_done_within_its_budget()
-> Result<(), Box<dyn Error>> {
    let (telemetry, _collector) = pushing_to_a_hung_collector()?;
    let budget = Duration::from_secs(1);
    let started = Instant::now();
    let outcome = telemetry.shutdown(budget).await;
    let took = started.elapsed();
    assert!(
        outcome.is_ok(),
        "the deadline, armed before the wait and a margin short of it, cut the last exports in \
         time, so the stop reports done rather than late: {outcome:?}"
    );
    assert!(
        took <= budget,
        "shutdown({budget:?}) returned after {took:?}"
    );
    Ok(())
}

#[tokio::test]
async fn shutdown_returns_promptly_once_and_later_spans_stay_home() -> Result<(), Box<dyn Error>> {
    let (telemetry, probe) = live(SiteDetail::None);
    telemetry.start_run(facts(&"run-1")).finish(OK);
    let started = Instant::now();
    telemetry.shutdown(Duration::from_secs(1)).await?;
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "with nothing pending, stopping is immediate"
    );
    telemetry.shutdown(Duration::from_secs(1)).await?;
    telemetry.start_run(facts(&"run-2")).finish(OK);
    assert_eq!(
        probe.spans().len(),
        1,
        "a run after shutdown records nothing that leaves the process"
    );
    Exposition::parse(&telemetry.render_prometheus())?;
    Ok(())
}
