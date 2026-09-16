//! Decides whether a `run` failure leaves the machine, and in what form.
//!
//! Five gates, cheapest and most privacy-protective first, so a report that will not be
//! sent costs nothing to refuse and no payload is built for it.

use crate::{
    db::run_error_budget::{BudgetDecision, DAILY_RUN_ERROR_CAP, RunErrorBudgetRepository},
    telemetry::{error_allowlist::classify, script_fingerprint::fingerprint},
};
use std::sync::Arc;

const BUILD_SENTRY_DSN: Option<&str> = option_env!("CLAW_SENTRY_DSN");

/// Everything we are willing to say about one failed run.
///
/// Constructed only from a fingerprint and an allowlisted error class, so there is no
/// field here that can hold user text even if something upstream changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFailureEvent {
    /// The script with every literal removed.
    pub script_fingerprint: String,
    /// A low-cardinality label, for example `timeout` or `engine:ReferenceError:fetch`.
    pub error_label: String,
    /// Coarse timing. A bucket rather than a duration, because a precise duration is a
    /// weak machine fingerprint and the classes separate perfectly well without it.
    pub duration_bucket: &'static str,
    /// The anonymous per-install UUID, the same one PostHog already uses.
    pub install_id: String,
}

/// Timing carries real diagnostic signal, so keep it, but only as a bucket.
#[must_use]
pub fn duration_bucket(duration_ms: i64) -> &'static str {
    match duration_ms {
        i64::MIN..=50 => "instant",
        51..=2_000 => "short",
        2_001..=28_999 => "medium",
        _ => "at_cap",
    }
}

/// Where a built event goes. A trait so the decision logic can be tested without a
/// network, and so tests can assert on exactly what would have left the machine.
pub trait RunFailureSink: Send + Sync {
    fn send(&self, event: RunFailureEvent);
}

/// Forwards to Sentry as a hand-built event.
///
/// Nothing is derived from ambient state: no request, no user, no breadcrumbs. Only the
/// fields above are attached.
pub struct SentrySink {
    client: Arc<sentry::Client>,
}

impl SentrySink {
    /// Returns `None` when no DSN is configured, which is the case for every developer
    /// build unless one is supplied deliberately.
    #[must_use]
    pub fn new(dsn: &str) -> Option<Self> {
        let options = sentry::ClientOptions {
            dsn: dsn.parse().ok(),
            // Never let Sentry infer anything about the machine or the user.
            send_default_pii: false,
            attach_stacktrace: false,
            max_breadcrumbs: 0,
            ..Default::default()
        };
        let client = sentry::Client::from_config(options);
        client.is_enabled().then(|| Self {
            client: Arc::new(client),
        })
    }
}

impl RunFailureSink for SentrySink {
    fn send(&self, event: RunFailureEvent) {
        let mut sentry_event = sentry::protocol::Event::new();
        sentry_event.level = sentry::Level::Error;
        sentry_event.logger = Some("run_failure".to_string());
        // Group by cause rather than by script, so one bug reads as one issue.
        sentry_event.message = Some(format!("run failed: {}", event.error_label));
        sentry_event.fingerprint = std::borrow::Cow::Owned(vec![std::borrow::Cow::Owned(format!(
            "run:{}:{}",
            event.error_label, event.duration_bucket
        ))]);
        sentry_event
            .tags
            .insert("error_label".to_string(), event.error_label);
        sentry_event.tags.insert(
            "duration_bucket".to_string(),
            event.duration_bucket.to_string(),
        );
        sentry_event.user = Some(sentry::User {
            id: Some(event.install_id),
            ..Default::default()
        });
        sentry_event.extra.insert(
            "script_fingerprint".to_string(),
            event.script_fingerprint.into(),
        );
        self.client.capture_event(sentry_event, None);
    }
}

/// Owns the decision about whether a run failure is reported.
pub struct RunFailureReporter {
    sink: Option<Box<dyn RunFailureSink>>,
    budget: RunErrorBudgetRepository,
    install_id: String,
    cap: i64,
}

impl RunFailureReporter {
    /// Builds a reporter from the ambient configuration. With no DSN this is a reporter
    /// that refuses everything, which is the correct default for a developer build.
    #[must_use]
    pub fn from_env(budget: RunErrorBudgetRepository, install_id: String) -> Self {
        let dsn = std::env::var("CLAW_SENTRY_DSN")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| BUILD_SENTRY_DSN.map(str::to_string))
            .filter(|value| !value.trim().is_empty());
        let sink = dsn
            .and_then(|dsn| SentrySink::new(&dsn))
            .map(|sink| Box::new(sink) as Box<dyn RunFailureSink>);
        Self {
            sink,
            budget,
            install_id,
            cap: DAILY_RUN_ERROR_CAP,
        }
    }

    #[must_use]
    pub fn with_sink(
        sink: Box<dyn RunFailureSink>,
        budget: RunErrorBudgetRepository,
        install_id: String,
        cap: i64,
    ) -> Self {
        Self {
            sink: Some(sink),
            budget,
            install_id,
            cap,
        }
    }

    /// True when a report could be sent at all. Checked before any payload is built.
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.sink.is_some()
    }

    /// Considers one failed run.
    ///
    /// `consent` is passed in rather than read here so this stays a pure decision that a
    /// test can drive, and so the caller cannot forget that consent is part of it: the
    /// signature will not let it.
    ///
    /// Returns whether an event was sent, for the caller's own metrics.
    pub async fn report(
        &self,
        consent: bool,
        script: &str,
        error_message: &str,
        duration_ms: i64,
        now_ms: i64,
    ) -> bool {
        if !consent {
            return false;
        }
        let Some(sink) = self.sink.as_ref() else {
            return false;
        };
        match self.budget.claim(now_ms, self.cap).await {
            Ok(BudgetDecision::Allowed { .. }) => {}
            Ok(BudgetDecision::Suppressed { .. }) => return false,
            Err(error) => {
                // A budget we cannot read is a budget we must assume is spent.
                tracing::debug!(%error, "run failure budget unavailable; not reporting");
                return false;
            }
        }
        sink.send(RunFailureEvent {
            script_fingerprint: fingerprint(script),
            error_label: classify(error_message).label(),
            duration_bucket: duration_bucket(duration_ms),
            install_id: self.install_id.clone(),
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use std::sync::Mutex;
    use tempfile::tempdir;

    const NOW: i64 = 1_789_560_000_000; // 2026-09-16T12:00:00Z

    #[derive(Default)]
    struct RecordingSink {
        sent: Mutex<Vec<RunFailureEvent>>,
    }

    impl RecordingSink {
        fn events(&self) -> Vec<RunFailureEvent> {
            self.sent.lock().unwrap_or_else(|e| e.into_inner()).clone()
        }
    }

    impl RunFailureSink for Arc<RecordingSink> {
        fn send(&self, event: RunFailureEvent) {
            self.sent
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(event);
        }
    }

    async fn reporter(
        dir: &std::path::Path,
        cap: i64,
    ) -> anyhow::Result<(RunFailureReporter, Arc<RecordingSink>)> {
        let db = Database::open(dir.join("browserclaw.sqlite")).await?;
        let sink = Arc::new(RecordingSink::default());
        let reporter = RunFailureReporter::with_sink(
            Box::new(sink.clone()),
            RunErrorBudgetRepository::new(db),
            "11111111-2222-3333-4444-555555555555".to_string(),
            cap,
        );
        Ok((reporter, sink))
    }

    #[tokio::test]
    async fn consent_off_sends_nothing() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (reporter, sink) = reporter(dir.path(), 10).await?;
        assert!(
            !reporter
                .report(false, "const a = 1;", "boom", 10, NOW)
                .await
        );
        assert!(sink.events().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn without_a_dsn_the_reporter_refuses_everything() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let db = Database::open(dir.path().join("browserclaw.sqlite")).await?;
        let reporter = RunFailureReporter {
            sink: None,
            budget: RunErrorBudgetRepository::new(db),
            install_id: "id".to_string(),
            cap: 10,
        };
        assert!(!reporter.is_configured());
        assert!(!reporter.report(true, "const a = 1;", "boom", 10, NOW).await);
        Ok(())
    }

    #[tokio::test]
    async fn the_daily_cap_stops_the_eleventh_report() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (reporter, sink) = reporter(dir.path(), 10).await?;
        for _ in 0..10 {
            assert!(reporter.report(true, "const a = 1;", "boom", 10, NOW).await);
        }
        assert!(!reporter.report(true, "const a = 1;", "boom", 10, NOW).await);
        assert_eq!(sink.events().len(), 10);
        Ok(())
    }

    #[tokio::test]
    async fn the_payload_carries_shape_and_class_only() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (reporter, sink) = reporter(dir.path(), 10).await?;
        reporter
            .report(
                true,
                "const pid = 20;\nawait browser.nav(pid).goto('https://x.test/?q=secret');",
                "ReferenceError: fetch is not defined",
                12,
                NOW,
            )
            .await;
        let events = sink.events();
        let event = events.first().unwrap_or_else(|| panic!("no event sent"));

        assert!(event.script_fingerprint.contains("browser.nav"));
        assert!(event.script_fingerprint.contains("<str>"));
        assert_eq!(event.error_label, "engine:ReferenceError:fetch");
        assert_eq!(event.duration_bucket, "instant");
        assert_eq!(event.install_id, "11111111-2222-3333-4444-555555555555");
        Ok(())
    }

    /// The whole pipeline in one assertion. A script and an error both stuffed with the
    /// kinds of thing a real session produces, and nothing of either may appear in what
    /// leaves the machine.
    #[tokio::test]
    async fn nothing_from_a_hostile_script_or_error_reaches_the_sink() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (reporter, sink) = reporter(dir.path(), 10).await?;

        let script = "\
// booking for Amara Okafor, confirmation AX8812\n\
const email = 'amara.okafor@example.com';\n\
await browser.input(3).fill(userField, email);\n\
await browser.input(3).fill(pwField, 'hunter2-correct-horse');\n\
await browser.nav(3).goto('https://bank.test/transfer?to=4471&amount=48201.55');\n\
await browser.upload(3, { path: '/Users/amara/Desktop/passport.pdf' });";
        let error = "Error: transfer declined for amara.okafor@example.com, balance 48,201.55";

        reporter.report(true, script, error, 900, NOW).await;
        let events = sink.events();
        let event = events.first().unwrap_or_else(|| panic!("no event sent"));
        let payload = format!(
            "{} {} {} {}",
            event.script_fingerprint, event.error_label, event.duration_bucket, event.install_id
        );

        for secret in [
            "Amara",
            "Okafor",
            "AX8812",
            "amara.okafor@example.com",
            "hunter2-correct-horse",
            "bank.test",
            "4471",
            "48201.55",
            "48,201.55",
            "/Users/amara",
            "passport.pdf",
        ] {
            assert!(!payload.contains(secret), "leaked {secret:?} in {payload}");
        }

        // and it is still diagnosable
        assert!(event.script_fingerprint.contains("browser.input"));
        assert!(event.script_fingerprint.contains(".fill"));
        assert!(event.script_fingerprint.contains("browser.upload"));
        Ok(())
    }

    #[tokio::test]
    async fn duration_buckets_separate_the_classes_the_audit_found() {
        assert_eq!(duration_bucket(1), "instant");
        assert_eq!(duration_bucket(619), "short");
        assert_eq!(duration_bucket(5_000), "medium");
        assert_eq!(duration_bucket(30_003), "at_cap");
    }
}
