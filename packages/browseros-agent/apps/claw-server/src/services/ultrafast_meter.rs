//! The Home meter for an ultrafast customer: what neo saved since this computer got ultrafast,
//! derived from the task history and the efficiency rows already kept. The start date lives in
//! `<data dir>/billing/ultrafast-since.json`.

use crate::{
    db::audit_log::UltrafastSession,
    error::{AppError, AppResult},
    storage::JsonStore,
};
use serde::{Deserialize, Serialize};

pub const SINCE_FILE: &str = "billing/ultrafast-since.json";

const WEEK_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const JS_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meter {
    pub since: i64,
    pub tokens_saved: i64,
    /// When each ultrafast task of the last week last ran, newest first.
    pub run_times: Vec<i64>,
    /// In tasks that used pursue, the average share of browser steps neo took, as a whole percent.
    pub neo_step_share: Option<i32>,
    pub fastest_run_ms: Option<i64>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SinceFile {
    since: i64,
}

/// The day to count from. A saved date always wins, so a lapse, a renewal or moving to a key never
/// restarts the count. Without one, the first time ultrafast is on it is the earlier of now and the
/// first ultrafast run, so a computer that used ultrafast before this was recorded doesn't start at
/// zero. `None` while ultrafast has never been on here.
#[must_use]
pub fn start_date(saved: Option<i64>, on: bool, first_run: Option<i64>, now: i64) -> Option<i64> {
    saved.or_else(|| on.then(|| first_run.map_or(now, |first| first.min(now))))
}

/// Serialises the read-then-save, so two first requests can't race on the same file: the first one
/// saves the date and the other reads it.
static SINCE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Reads the saved start date, and saves one the first time ultrafast is on.
pub async fn since(
    store: &JsonStore,
    on: bool,
    first_run: Option<i64>,
    now: i64,
) -> AppResult<Option<i64>> {
    let _guard = SINCE_LOCK.lock().await;
    let saved = match store.read_json::<SinceFile>(SINCE_FILE).await {
        Ok(file) => Some(file.since),
        Err(AppError::StorageNotFound(_)) => None,
        Err(error) => return Err(error),
    };
    let since = start_date(saved, on, first_run, now);
    if saved.is_none()
        && let Some(since) = since
    {
        store.write_json(SINCE_FILE, &SinceFile { since }).await?;
    }
    Ok(since)
}

/// The meter from the start date, neo's own token saving since then, and every ultrafast session.
#[must_use]
pub fn meter(since: i64, neo_saved: i64, sessions: &[UltrafastSession], now: i64) -> Meter {
    let ultrafast_tokens = sessions
        .iter()
        .filter(|session| session.last_run_at >= since)
        .map(|session| session.totals.tokens_saved.max(0))
        .fold(0_i64, i64::saturating_add);
    let mut run_times = sessions
        .iter()
        .map(|session| session.last_run_at)
        .filter(|at| *at >= now.saturating_sub(WEEK_MS))
        .collect::<Vec<_>>();
    run_times.sort_unstable_by(|a, b| b.cmp(a));
    let shares = sessions
        .iter()
        .filter_map(|session| session.pursue_step_share)
        .filter(|(_, all)| *all > 0)
        .map(|(neo, all)| neo as f64 / all as f64)
        .collect::<Vec<_>>();
    let neo_step_share = (!shares.is_empty()).then(|| {
        let mean = shares.iter().sum::<f64>() / shares.len() as f64;
        (mean * 100.0).round().clamp(0.0, 100.0) as i32
    });
    let fastest_run_ms = sessions
        .iter()
        .flat_map(|session| session.run_durations.iter().copied())
        .filter(|duration| *duration > 0)
        .min();
    Meter {
        since,
        tokens_saved: neo_saved
            .max(0)
            .saturating_add(ultrafast_tokens)
            .min(JS_SAFE_INTEGER),
        run_times,
        neo_step_share,
        fastest_run_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ultrafast_runs::UltrafastRunStats;

    const DAY: i64 = 24 * 60 * 60 * 1000;
    const NOW: i64 = 100 * DAY;

    fn session(
        last_run_at: i64,
        tokens: i64,
        runs: &[i64],
        share: Option<(i64, i64)>,
    ) -> UltrafastSession {
        UltrafastSession {
            session_id: format!("s{last_run_at}"),
            totals: UltrafastRunStats {
                duration_ms: runs.iter().sum(),
                pages_read: 1,
                tokens_saved: tokens,
            },
            measured: None,
            pursue: None,
            run_durations: runs.to_vec(),
            pursue_step_share: share,
            first_run_at: last_run_at,
            last_run_at,
        }
    }

    #[test]
    fn a_saved_date_is_never_replaced() {
        assert_eq!(start_date(Some(5), true, Some(1), NOW), Some(5));
        assert_eq!(start_date(Some(5), false, None, NOW), Some(5));
    }

    #[test]
    fn the_first_time_ultrafast_is_on_counts_from_the_earliest_run() {
        assert_eq!(start_date(None, true, Some(40 * DAY), NOW), Some(40 * DAY));
        assert_eq!(start_date(None, true, None, NOW), Some(NOW));
        // A clock that ran backwards never puts the start in the future.
        assert_eq!(start_date(None, true, Some(NOW + DAY), NOW), Some(NOW));
    }

    #[test]
    fn without_ultrafast_there_is_no_start_date() {
        assert_eq!(start_date(None, false, Some(40 * DAY), NOW), None);
    }

    #[tokio::test]
    async fn the_start_date_is_saved_once_and_then_kept() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = JsonStore::new(dir.path().to_path_buf());
        assert_eq!(since(&store, false, None, NOW).await?, None);
        assert_eq!(
            since(&store, true, Some(90 * DAY), NOW).await?,
            Some(90 * DAY)
        );
        // Later reads keep it, even with ultrafast off or an earlier run turning up.
        assert_eq!(since(&store, false, None, NOW + DAY).await?, Some(90 * DAY));
        assert_eq!(
            since(&store, true, Some(DAY), NOW + DAY).await?,
            Some(90 * DAY)
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn first_requests_at_once_all_get_the_one_saved_date() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = JsonStore::new(dir.path().to_path_buf());
        let requests = (0..16_i64)
            .map(|offset| {
                let store = store.clone();
                tokio::spawn(async move { since(&store, true, Some(50 * DAY + offset), NOW).await })
            })
            .collect::<Vec<_>>();
        let mut dates = Vec::new();
        for request in requests {
            dates.push(request.await??);
        }
        let first = dates[0];
        assert!(first.is_some());
        assert!(dates.iter().all(|date| *date == first), "{dates:?}");
        assert_eq!(since(&store, false, None, NOW).await?, first);
        Ok(())
    }

    #[test]
    fn tokens_saved_adds_neos_saving_to_ultrafast_since_the_start() {
        let sessions = [
            session(NOW - DAY, 6_000, &[3_000], None),
            session(80 * DAY, 9_000, &[4_000], None),
            session(70 * DAY, 50_000, &[2_000], None),
        ];
        let meter = meter(75 * DAY, 1_000_000, &sessions, NOW);
        assert_eq!(meter.since, 75 * DAY);
        assert_eq!(meter.tokens_saved, 1_015_000);
        // A negative neo saving never takes away from ultrafast's.
        assert_eq!(
            super::meter(75 * DAY, -500, &sessions, NOW).tokens_saved,
            15_000
        );
    }

    #[test]
    fn the_week_lists_runs_from_the_last_seven_days_newest_first() {
        let sessions = [
            session(NOW - 8 * DAY, 1, &[1_000], None),
            session(NOW - 2 * DAY, 1, &[1_000], None),
            session(NOW - DAY, 1, &[1_000], None),
        ];
        assert_eq!(
            meter(0, 0, &sessions, NOW).run_times,
            vec![NOW - DAY, NOW - 2 * DAY]
        );
    }

    #[test]
    fn the_step_share_is_the_mean_of_each_pursue_task() {
        let sessions = [
            session(NOW, 1, &[1_000], Some((3, 4))),
            session(NOW, 1, &[1_000], Some((1, 2))),
            session(NOW, 1, &[1_000], None),
        ];
        // 75% and 50% average to 63% (62.5 rounded), not the 4 of 6 steps pooled.
        assert_eq!(meter(0, 0, &sessions, NOW).neo_step_share, Some(63));
        assert_eq!(meter(0, 0, &sessions[2..], NOW).neo_step_share, None);
    }

    #[test]
    fn the_fastest_run_is_the_shortest_single_run() {
        let sessions = [
            session(NOW, 1, &[15_354, 6_000], None),
            session(NOW, 1, &[3_095, 0], None),
        ];
        assert_eq!(meter(0, 0, &sessions, NOW).fastest_run_ms, Some(3_095));
        assert_eq!(meter(0, 0, &[], NOW).fastest_run_ms, None);
    }
}
