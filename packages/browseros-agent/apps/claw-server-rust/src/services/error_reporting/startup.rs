//! Reports a fatal boot failure or a panic to Sentry before the process exits.
//!
//! The runtime error reporter needs the database for its daily budget and a running server, so it
//! cannot report the two failures that matter most here: a database that will not open, and a panic
//! during startup. This reporter is independent of both. It reads consent and the install id from
//! the analytics state file, counts against a small file-backed daily budget that needs no database
//! and no lock (startup is single-threaded), builds the same hand-made, allowlisted event as the
//! runtime path, and flushes the client before the process exits so the report is not lost.

use super::{GeneralErrorEvent, build_general_event, build_sentry_client, resolve_dsn};
use crate::{
    analytics::state::{load_or_create_state, read_consent, state_path},
    clock::now_epoch_ms,
    db::run_error_budget::utc_day,
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

/// How many startup or panic reports one install may forward in a UTC day. Small because a failing
/// boot tends to repeat identically on every launch, so a handful is enough to see it, and it is a
/// separate allowance from the runtime cap only because a dead database cannot reach that one.
pub const DAILY_STARTUP_REPORT_CAP: i64 = 3;

/// How long to wait for the transport to drain before letting the process exit.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// The persisted shape of the boot budget: a day key and a count.
#[derive(serde::Serialize, serde::Deserialize)]
struct BudgetFile {
    day: String,
    sent: i64,
}

/// A file-backed daily counter for pre-database reports. Startup is single-threaded (one sidecar
/// boots per data directory, one claim per launch), so the counter needs no cross-process lock; a
/// mutex only serializes against a panic that fires on another thread.
pub struct BootBudget {
    path: PathBuf,
    guard: Mutex<()>,
}

impl BootBudget {
    #[must_use]
    pub fn new(browserclaw_dir: &Path) -> Self {
        Self {
            path: browserclaw_dir.join("startup-report-budget.json"),
            guard: Mutex::new(()),
        }
    }

    /// Reserves one report against today's budget, returning whether there was room.
    ///
    /// Fails closed: only a missing file (nothing sent yet) or a valid counter from an earlier day
    /// starts at zero. A file that is present but unreadable or malformed is assumed spent, so a
    /// corrupt counter can never re-grant the day's allowance. Writes go through a temp file and a
    /// rename so an interrupted write never leaves a torn file behind.
    #[must_use]
    pub fn claim(&self, epoch_ms: i64, cap: i64) -> bool {
        let _lock = self
            .guard
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let today = utc_day(epoch_ms);
        let sent_today = match std::fs::read(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(_) => return false,
            Ok(bytes) => match serde_json::from_slice::<BudgetFile>(&bytes) {
                Ok(file) if file.day == today => file.sent,
                Ok(_) => 0,
                Err(_) => return false,
            },
        };
        if sent_today >= cap {
            return false;
        }
        let next = BudgetFile {
            day: today,
            sent: sent_today + 1,
        };
        let Ok(serialized) = serde_json::to_vec(&next) else {
            return false;
        };
        let temp = self.path.with_extension("tmp");
        std::fs::write(&temp, &serialized)
            .and_then(|()| std::fs::rename(&temp, &self.path))
            .is_ok()
    }
}

/// Reports fatal startup failures and panics, independent of the database.
#[derive(Clone)]
pub struct StartupReporter {
    client: Option<Arc<sentry::Client>>,
    budget: Arc<BootBudget>,
    install_id: Option<String>,
    state_path: PathBuf,
}

impl StartupReporter {
    /// Builds the reporter from the data directory, reading the install id from the analytics state
    /// file so it is usable before the database or anything else is available. Consent is not cached
    /// here: it is read live on each report, so a later opt-out is honored by the long-lived hook.
    pub async fn from_dir(browserclaw_dir: &Path) -> Self {
        let state_path = state_path(browserclaw_dir);
        let state = load_or_create_state(&state_path).await;
        let client = resolve_dsn()
            .as_deref()
            .and_then(build_sentry_client)
            .map(Arc::new);
        Self {
            client,
            budget: Arc::new(BootBudget::new(browserclaw_dir)),
            install_id: state.distinct_id,
            state_path,
        }
    }

    /// Reports one fatal boot failure or panic, then flushes so the event is not lost to the
    /// imminent exit. Best effort and never panics; a report that cannot be sent is dropped. `label`
    /// and `detail` must be caller-fixed and free of user data (a fixed label, or a panic's source
    /// location, never a raw error message).
    pub fn report(&self, kind: &'static str, label: &str, detail: Option<&str>) {
        // Read live, not cached at boot: a user who turns telemetry off through the running server
        // persists that choice to this file, and the long-lived panic hook must honor it.
        if !read_consent(&self.state_path) {
            return;
        }
        let (Some(client), Some(install_id)) = (&self.client, &self.install_id) else {
            return;
        };
        if !self.budget.claim(now_epoch_ms(), DAILY_STARTUP_REPORT_CAP) {
            return;
        }
        let event = build_general_event(&GeneralErrorEvent {
            kind,
            label: label.to_string(),
            detail: detail.map(str::to_string),
            install_id: install_id.clone(),
        });
        client.capture_event(event, None);
        // Drain before the process exits; capture alone queues on a background transport that a
        // process::exit or a panic abort would discard.
        client.close(Some(FLUSH_TIMEOUT));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const DAY_ONE: i64 = 1_789_560_000_000; // 2026-09-16T12:00:00Z, midday
    const DAY_TWO: i64 = DAY_ONE + 86_400_000;

    #[test]
    fn claims_are_allowed_up_to_the_cap_then_refused() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let budget = BootBudget::new(dir.path());
        for _ in 0..DAILY_STARTUP_REPORT_CAP {
            assert!(budget.claim(DAY_ONE, DAILY_STARTUP_REPORT_CAP));
        }
        assert!(
            !budget.claim(DAY_ONE, DAILY_STARTUP_REPORT_CAP),
            "the report past the cap must be refused"
        );
        Ok(())
    }

    #[test]
    fn a_new_day_resets_the_budget() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let budget = BootBudget::new(dir.path());
        for _ in 0..DAILY_STARTUP_REPORT_CAP {
            assert!(budget.claim(DAY_ONE, DAILY_STARTUP_REPORT_CAP));
        }
        assert!(!budget.claim(DAY_ONE, DAILY_STARTUP_REPORT_CAP));
        assert!(
            budget.claim(DAY_TWO, DAILY_STARTUP_REPORT_CAP),
            "a new day must reset the budget"
        );
        Ok(())
    }

    #[test]
    fn the_budget_survives_a_restart() -> anyhow::Result<()> {
        let dir = tempdir()?;
        for _ in 0..DAILY_STARTUP_REPORT_CAP {
            assert!(BootBudget::new(dir.path()).claim(DAY_ONE, DAILY_STARTUP_REPORT_CAP));
        }
        assert!(
            !BootBudget::new(dir.path()).claim(DAY_ONE, DAILY_STARTUP_REPORT_CAP),
            "a fresh BootBudget over the same directory must see the spent budget"
        );
        Ok(())
    }

    #[tokio::test]
    async fn without_a_dsn_the_reporter_is_silent_but_does_not_panic() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let reporter = StartupReporter::from_dir(dir.path()).await;
        // No DSN in a dev build, so there is no client and nothing is sent; this must not panic and
        // must not consume budget.
        reporter.report("startup", "init_failed", None);
        assert!(
            BootBudget::new(dir.path()).claim(DAY_ONE, DAILY_STARTUP_REPORT_CAP),
            "a report with no client must not have drawn down the budget"
        );
        Ok(())
    }

    #[test]
    fn a_corrupt_counter_is_assumed_spent() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let budget = BootBudget::new(dir.path());
        assert!(budget.claim(DAY_ONE, DAILY_STARTUP_REPORT_CAP));
        // A torn write leaves the counter unreadable. It must not re-grant the day's allowance.
        std::fs::write(
            dir.path().join("startup-report-budget.json"),
            b"{ not valid",
        )?;
        assert!(
            !budget.claim(DAY_ONE, DAILY_STARTUP_REPORT_CAP),
            "a corrupt counter must fail closed, not reset the cap"
        );
        Ok(())
    }

    #[test]
    fn consent_is_read_live_from_the_state_file() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let path = state_path(dir.path());
        assert!(read_consent(&path), "a missing file is the system default");
        std::fs::write(&path, br#"{"enabled":false}"#)?;
        assert!(!read_consent(&path), "a persisted opt-out must be honored");
        std::fs::write(&path, br#"{"enabled":true}"#)?;
        assert!(read_consent(&path));
        std::fs::write(&path, b"{ not valid")?;
        assert!(!read_consent(&path), "a corrupt consent file fails closed");
        Ok(())
    }
}
