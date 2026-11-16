//! The Home meter for an ultrafast customer: what neo saved since this computer got ultrafast,
//! derived from the task history and the efficiency rows already kept. The start date lives in
//! `<data dir>/billing/ultrafast-since.json`.

use crate::{
    db::{pursue_runs::PursueOutcome, ultrafast_ledger::UltrafastRecord},
    error::{AppError, AppResult},
    storage::JsonStore,
};
use serde::{Deserialize, Serialize};

pub const SINCE_FILE: &str = "billing/ultrafast-since.json";

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const WEEK_MS: i64 = 7 * DAY_MS;
const MONTH_MS: i64 = 30 * DAY_MS;
/// How many sites a period lists under "Where neo helped".
const TOP_SITES: usize = 3;
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
    pub periods: Periods,
}

/// neo's own saving in a period, from the efficiency rows the cockpit stats read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NeoWindow {
    pub tokens_saved: i64,
    pub time_saved_ms: i64,
    pub tasks_finished: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NeoWindows {
    pub since: NeoWindow,
    pub last_30_days: NeoWindow,
    pub last_7_days: NeoWindow,
}

/// What the Details dialog shows for one period.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Period {
    /// neo's own saving plus the page text neo read itself in the period's ultrafast tasks.
    pub tokens_saved: i64,
    pub runs: i64,
    pub pages_read: i64,
    pub duration_ms: i64,
    pub fastest_run_ms: Option<i64>,
    pub neo_step_share: Option<i32>,
    /// Tasks in the period that used pursue, and the ones it finished without handing back.
    pub site_tasks: i64,
    pub site_tasks_finished: i64,
    /// The sites with the most ultrafast tasks, most first.
    pub top_sites: Vec<(String, i64)>,
    pub time_saved_ms: i64,
    pub tasks_finished: i64,
    /// When each of the period's ultrafast tasks last ran, newest first.
    pub run_times: Vec<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Periods {
    pub since: Period,
    pub last_30_days: Period,
    pub last_7_days: Period,
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

/// The saved start date, if this computer has one yet.
pub async fn saved_since(store: &JsonStore) -> AppResult<Option<i64>> {
    match store.read_json::<SinceFile>(SINCE_FILE).await {
        Ok(file) => Ok(Some(file.since)),
        Err(AppError::StorageNotFound(_)) => Ok(None),
        Err(error) => Err(error),
    }
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
    let saved = saved_since(store).await?;
    let since = start_date(saved, on, first_run, now);
    if saved.is_none()
        && let Some(since) = since
    {
        store.write_json(SINCE_FILE, &SinceFile { since }).await?;
    }
    Ok(since)
}

/// When each period starts: the start date, and 30 and 7 days back from `now`, matching the
/// cockpit's own windows.
#[must_use]
pub fn period_starts(since: i64, now: i64) -> [i64; 3] {
    [
        since,
        now.saturating_sub(MONTH_MS),
        now.saturating_sub(WEEK_MS),
    ]
}

/// The meter from the start date, neo's own saving in each period, and every ultrafast task.
#[must_use]
pub fn meter(since: i64, neo: &NeoWindows, records: &[UltrafastRecord], now: i64) -> Meter {
    let [_, month, week] = period_starts(since, now);
    let periods = Periods {
        since: period(since, neo.since, records),
        last_30_days: period(month, neo.last_30_days, records),
        last_7_days: period(week, neo.last_7_days, records),
    };
    Meter {
        since,
        tokens_saved: periods.since.tokens_saved,
        run_times: periods.last_7_days.run_times.clone(),
        neo_step_share: step_share(records.iter()),
        fastest_run_ms: fastest(records.iter()),
        periods,
    }
}

/// One period: the ultrafast tasks whose last run started at or after `start`, plus neo's own
/// saving over the same span.
#[must_use]
pub fn period(start: i64, neo: NeoWindow, records: &[UltrafastRecord]) -> Period {
    let inside = records
        .iter()
        .filter(|record| record.last_run_at >= start)
        .collect::<Vec<_>>();
    let sum = |value: fn(&UltrafastRecord) -> i64| {
        inside
            .iter()
            .map(|record| value(record).max(0))
            .fold(0_i64, i64::saturating_add)
    };
    let mut run_times = inside
        .iter()
        .map(|record| record.last_run_at)
        .collect::<Vec<_>>();
    run_times.sort_unstable_by(|a, b| b.cmp(a));
    let site_tasks = inside
        .iter()
        .filter(|record| record.pursue_outcome.is_some())
        .count();
    let site_tasks_finished = inside
        .iter()
        .filter(|record| record.pursue_outcome == Some(PursueOutcome::Completed))
        .count();
    Period {
        tokens_saved: neo
            .tokens_saved
            .max(0)
            .saturating_add(sum(|record| record.totals.tokens_saved))
            .min(JS_SAFE_INTEGER),
        runs: count(inside.len()),
        pages_read: sum(|record| record.totals.pages_read),
        duration_ms: sum(|record| record.totals.duration_ms),
        fastest_run_ms: fastest(inside.iter().copied()),
        neo_step_share: step_share(inside.iter().copied()),
        site_tasks: count(site_tasks),
        site_tasks_finished: count(site_tasks_finished),
        top_sites: top_sites(&inside),
        time_saved_ms: neo.time_saved_ms.max(0),
        tasks_finished: neo.tasks_finished.max(0),
        run_times,
    }
}

fn count(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// The average share of browser steps neo took across tasks that used pursue, as a whole percent.
fn step_share<'a>(records: impl Iterator<Item = &'a UltrafastRecord>) -> Option<i32> {
    let shares = records
        .filter_map(|record| record.pursue_step_share)
        .filter(|(_, all)| *all > 0)
        .map(|(neo, all)| neo as f64 / all as f64)
        .collect::<Vec<_>>();
    (!shares.is_empty()).then(|| {
        let mean = shares.iter().sum::<f64>() / shares.len() as f64;
        (mean * 100.0).round().clamp(0.0, 100.0) as i32
    })
}

fn fastest<'a>(records: impl Iterator<Item = &'a UltrafastRecord>) -> Option<i64> {
    records
        .filter_map(|record| record.fastest_run_ms)
        .filter(|duration| *duration > 0)
        .min()
}

/// Sites by how many ultrafast tasks ran there, most first; a tie goes to the more recent one.
fn top_sites(records: &[&UltrafastRecord]) -> Vec<(String, i64)> {
    let mut sites: std::collections::HashMap<&str, (i64, i64)> = std::collections::HashMap::new();
    for record in records {
        let Some(site) = record.site.as_deref().filter(|site| !site.is_empty()) else {
            continue;
        };
        let entry = sites.entry(site).or_insert((0, i64::MIN));
        entry.0 += 1;
        entry.1 = entry.1.max(record.last_run_at);
    }
    let mut sites = sites.into_iter().collect::<Vec<_>>();
    sites.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(b.1.1.cmp(&a.1.1)).then(a.0.cmp(b.0)));
    sites
        .into_iter()
        .take(TOP_SITES)
        .map(|(site, (runs, _))| (site.to_string(), runs))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ultrafast_runs::UltrafastRunStats;

    const DAY: i64 = 24 * 60 * 60 * 1000;
    const NOW: i64 = 100 * DAY;

    /// neo's own saving: the same in every period unless a test says otherwise.
    fn neo(tokens: i64) -> NeoWindows {
        let window = NeoWindow {
            tokens_saved: tokens,
            time_saved_ms: 0,
            tasks_finished: 0,
        };
        NeoWindows {
            since: window,
            last_30_days: window,
            last_7_days: window,
        }
    }

    fn session(
        last_run_at: i64,
        tokens: i64,
        runs: &[i64],
        share: Option<(i64, i64)>,
    ) -> UltrafastRecord {
        UltrafastRecord {
            session_id: format!("s{last_run_at}"),
            totals: UltrafastRunStats {
                duration_ms: runs.iter().sum(),
                pages_read: 1,
                tokens_saved: tokens,
            },
            fastest_run_ms: runs.iter().copied().filter(|run| *run > 0).min(),
            pursue_step_share: share,
            first_run_at: last_run_at,
            last_run_at,
            site: None,
            pursue_outcome: None,
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
        let meter = meter(75 * DAY, &neo(1_000_000), &sessions, NOW);
        assert_eq!(meter.since, 75 * DAY);
        assert_eq!(meter.tokens_saved, 1_015_000);
        // A negative neo saving never takes away from ultrafast's.
        assert_eq!(
            super::meter(75 * DAY, &neo(-500), &sessions, NOW).tokens_saved,
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
            meter(0, &neo(0), &sessions, NOW).run_times,
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
        assert_eq!(meter(0, &neo(0), &sessions, NOW).neo_step_share, Some(63));
        assert_eq!(meter(0, &neo(0), &sessions[2..], NOW).neo_step_share, None);
    }

    #[test]
    fn the_fastest_run_is_the_shortest_single_run() {
        let sessions = [
            session(NOW, 1, &[15_354, 6_000], None),
            session(NOW, 1, &[3_095, 0], None),
        ];
        assert_eq!(
            meter(0, &neo(0), &sessions, NOW).fastest_run_ms,
            Some(3_095)
        );
        assert_eq!(meter(0, &neo(0), &[], NOW).fastest_run_ms, None);
    }

    fn site_task(
        last_run_at: i64,
        site: &str,
        outcome: Option<PursueOutcome>,
        tokens: i64,
    ) -> UltrafastRecord {
        UltrafastRecord {
            site: Some(site.into()),
            pursue_outcome: outcome,
            pursue_step_share: outcome.map(|_| (2, 4)),
            ..session(last_run_at, tokens, &[5_000], None)
        }
    }

    fn windows() -> NeoWindows {
        NeoWindows {
            since: NeoWindow {
                tokens_saved: 72_000,
                time_saved_ms: 52 * 60_000,
                tasks_finished: 14,
            },
            last_30_days: NeoWindow {
                tokens_saved: 110_000,
                time_saved_ms: 60 * 60_000,
                tasks_finished: 18,
            },
            last_7_days: NeoWindow {
                tokens_saved: 41_000,
                time_saved_ms: 24 * 60_000,
                tasks_finished: 7,
            },
        }
    }

    /// Bought 12 days ago; runs 10, 5, 4, 1 and 0 days ago.
    fn history() -> Vec<UltrafastRecord> {
        vec![
            site_task(NOW, "google.com", Some(PursueOutcome::HandedBack), 0),
            site_task(NOW - DAY, "amazon.in", Some(PursueOutcome::Completed), 0),
            site_task(NOW - 4 * DAY, "isthereanydeal.com", None, 6_118),
            site_task(NOW - 5 * DAY, "flipkart.com", None, 61_942),
            site_task(NOW - 10 * DAY, "flipkart.com", None, 186_000),
        ]
    }

    #[test]
    fn the_since_period_is_the_meters_number() {
        let meter = meter(NOW - 12 * DAY, &windows(), &history(), NOW);
        assert_eq!(meter.periods.since.tokens_saved, meter.tokens_saved);
        assert_eq!(meter.tokens_saved, 72_000 + 254_060);
        assert_eq!(meter.periods.since.runs, 5);
        assert_eq!(meter.periods.since.time_saved_ms, 52 * 60_000);
        assert_eq!(meter.periods.since.tasks_finished, 14);
    }

    #[test]
    fn each_period_counts_its_own_runs() {
        let periods = meter(NOW - 12 * DAY, &windows(), &history(), NOW).periods;
        let week = &periods.last_7_days;
        assert_eq!(week.runs, 4);
        assert_eq!(week.tokens_saved, 41_000 + 68_060);
        assert_eq!(week.pages_read, 4);
        assert_eq!(week.duration_ms, 20_000);
        assert_eq!(
            week.run_times,
            vec![NOW, NOW - DAY, NOW - 4 * DAY, NOW - 5 * DAY]
        );
        assert_eq!(week.site_tasks, 2);
        assert_eq!(week.site_tasks_finished, 1);
        assert_eq!(week.neo_step_share, Some(50));
        assert_eq!(week.fastest_run_ms, Some(5_000));
        assert_eq!(periods.last_30_days.runs, 5);
        assert_eq!(periods.last_30_days.tokens_saved, 110_000 + 254_060);
    }

    #[test]
    fn a_shorter_period_never_holds_more_than_a_longer_one() {
        let periods = meter(NOW - 12 * DAY, &windows(), &history(), NOW).periods;
        let (week, month) = (&periods.last_7_days, &periods.last_30_days);
        assert!(week.tokens_saved <= month.tokens_saved);
        assert!(week.runs <= month.runs && week.runs <= periods.since.runs);
        assert!(week.pages_read <= month.pages_read);
        assert!(week.duration_ms <= month.duration_ms);
        assert!(week.site_tasks <= month.site_tasks);
        // Bought within the 30 days, so the since period is part of the 30 days.
        assert!(periods.since.runs <= month.runs);
    }

    #[test]
    fn a_run_on_the_purchase_moment_counts_and_one_before_it_does_not() {
        let since = NOW - 12 * DAY;
        let records = vec![
            site_task(since, "google.com", None, 1_000),
            site_task(since - 1, "amazon.in", None, 2_000),
        ];
        let periods = meter(since, &neo(0), &records, NOW).periods;
        assert_eq!(periods.since.runs, 1);
        assert_eq!(periods.since.tokens_saved, 1_000);
        assert_eq!(periods.last_30_days.runs, 2);
    }

    #[test]
    fn a_period_without_runs_still_has_neos_own_saving() {
        let mut quiet = windows();
        quiet.last_7_days.tokens_saved = 12_000;
        let records = vec![site_task(NOW - 10 * DAY, "flipkart.com", None, 186_000)];
        let week = meter(NOW - 12 * DAY, &quiet, &records, NOW)
            .periods
            .last_7_days;
        assert_eq!(week.runs, 0);
        assert_eq!(week.tokens_saved, 12_000);
        assert!(week.top_sites.is_empty());
        assert_eq!(week.fastest_run_ms, None);
        assert_eq!(week.neo_step_share, None);
        assert_eq!(week.tasks_finished, 7);
    }

    #[test]
    fn top_sites_rank_by_runs_then_the_latest() {
        let since = meter(NOW - 12 * DAY, &windows(), &history(), NOW)
            .periods
            .since
            .top_sites;
        assert_eq!(
            since,
            vec![
                ("flipkart.com".to_string(), 2),
                ("google.com".to_string(), 1),
                ("amazon.in".to_string(), 1),
            ]
        );
    }
}
