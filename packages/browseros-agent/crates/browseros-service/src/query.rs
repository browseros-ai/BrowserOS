//! Request text into domain types: query strings and path segments, parsed by hand so every
//! refusal is the contract's 400 `invalid_request` (or 404 for an id that names nothing),
//! never framework text.
//!
//! A known key with a bad value, or given twice, is refused; an unknown key is ignored, so a
//! host can send a parameter a newer contract adds. Every function here is pure.

use crate::api_error::ApiError;
use crate::event_log::EventFilter;
use crate::limits::{
    DEFAULT_LIMIT, DEFAULT_TAIL, MAX_INLINE_SCREENSHOTS, MAX_LIMIT, MAX_TAIL, MAX_WAIT_SECONDS,
};
use browseros_core::TabId;
use browseros_trace::{
    EpochMillis, OperationId, RequestId, RunId, RunQuery, RunStatus, TailQuery, TaskId,
};
use std::borrow::Cow;
use std::fmt::Display;
use std::ops::RangeInclusive;
use std::str::FromStr;
use std::time::Duration;

/// `GET /v1/runs`.
pub(crate) fn runs(raw: Option<&str>) -> Result<RunQuery, ApiError> {
    let params = Params::parse(raw);
    Ok(RunQuery {
        request_id: params
            .get("requestId")?
            .map(|text| {
                RequestId::parse(text)
                    .map_err(|_| ApiError::invalid("`requestId` must be 1 to 128 bytes"))
            })
            .transpose()?,
        task_id: params.task()?,
        tab: params.tab()?,
        status: params
            .get("status")?
            .map(|text| {
                RunStatus::parse(text).ok_or_else(|| {
                    ApiError::invalid(
                        "`status` must be running, ok, error, cancelled, timeout or interrupted",
                    )
                })
            })
            .transpose()?,
        since: params.millis("since")?,
        until: params.millis("until")?,
        cursor: params.get("cursor")?.map(cursor).transpose()?,
        limit: params
            .number("limit", 1..=MAX_LIMIT)?
            .unwrap_or(DEFAULT_LIMIT),
    })
}

/// Which operations' screenshots a read inlines as base64.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Inline {
    None,
    All,
    /// The latest N operations that have one.
    Last(u32),
}

/// `GET /v1/runs/{runId}`: how long `wait` may hold the request, and `include=screenshots`.
pub(crate) fn run(raw: Option<&str>) -> Result<(Duration, Inline), ApiError> {
    let params = Params::parse(raw);
    let seconds = params.number("wait", 0..=MAX_WAIT_SECONDS)?;
    let inline = match params.get("include")? {
        None => Inline::None,
        Some("screenshots") => Inline::All,
        Some(_) => return Err(ApiError::invalid("`include` must be `screenshots`")),
    };
    Ok((Duration::from_secs(seconds.unwrap_or(0)), inline))
}

/// `GET /v1/trace`, with `screenshots=none|all|last:N`.
pub(crate) fn trace(raw: Option<&str>) -> Result<(TailQuery, Inline), ApiError> {
    let params = Params::parse(raw);
    let query = TailQuery {
        task_id: params.task()?,
        tab: params.tab()?,
        tail: params.number("tail", 1..=MAX_TAIL)?.unwrap_or(DEFAULT_TAIL),
    };
    let inline = match params.get("screenshots")? {
        None | Some("none") => Inline::None,
        Some("all") => Inline::All,
        Some(text) => text
            .strip_prefix("last:")
            .and_then(|count| count.parse::<u32>().ok())
            .filter(|count| (1..=MAX_INLINE_SCREENSHOTS).contains(count))
            .map(Inline::Last)
            .ok_or_else(|| {
                ApiError::invalid(format!(
                    "`screenshots` must be none, all or last:N with N from 1 to \
                     {MAX_INLINE_SCREENSHOTS}"
                ))
            })?,
    };
    Ok((query, inline))
}

/// `GET /v1/events`.
pub(crate) fn events(raw: Option<&str>) -> Result<EventFilter, ApiError> {
    let params = Params::parse(raw);
    Ok(EventFilter {
        task: params.task()?,
        tab: params.tab()?,
    })
}

/// A run id in a path. Text that is not one names no run.
pub(crate) fn run_id(text: &str) -> Result<RunId, ApiError> {
    text.parse()
        .map_err(|_| ApiError::not_found("no run has this id"))
}

/// An operation id (`<runId>.<seq>`) in a path. Text that is not one names no operation.
pub(crate) fn operation_id(text: &str) -> Result<OperationId, ApiError> {
    text.parse()
        .map_err(|_| ApiError::not_found("no operation has this id"))
}

/// A tab id in a path: the browser's integer id.
pub(crate) fn tab_id(text: &str) -> Result<TabId, ApiError> {
    text.parse()
        .map(TabId)
        .map_err(|_| ApiError::invalid("`tabId` must be an integer"))
}

/// A `nextCursor` the store gave: `{millis}:{run id}`. The store reports a malformed one as
/// `Corrupt`, the same as a damaged file, so it is checked here and refused with a 400 rather
/// than reaching the store and becoming a 500.
fn cursor(text: &str) -> Result<String, ApiError> {
    let valid = text
        .split_once(':')
        .is_some_and(|(millis, run)| millis.parse::<i64>().is_ok() && run.parse::<RunId>().is_ok());
    if valid {
        Ok(text.to_string())
    } else {
        Err(ApiError::invalid("`cursor` is not one Neo gave"))
    }
}

/// A query string's percent-decoded pairs.
struct Params<'a>(Vec<(Cow<'a, str>, Cow<'a, str>)>);

impl<'a> Params<'a> {
    fn parse(raw: Option<&'a str>) -> Self {
        Self(
            raw.map(|raw| url::form_urlencoded::parse(raw.as_bytes()).collect())
                .unwrap_or_default(),
        )
    }

    /// The value of `key`. A key given twice is ambiguous, so it is refused.
    fn get(&self, key: &str) -> Result<Option<&str>, ApiError> {
        let mut values = self
            .0
            .iter()
            .filter(|(name, _)| name == key)
            .map(|(_, value)| value.as_ref());
        let value = values.next();
        if values.next().is_some() {
            return Err(ApiError::invalid(format!(
                "`{key}` is given more than once"
            )));
        }
        Ok(value)
    }

    fn number<T>(&self, key: &str, range: RangeInclusive<T>) -> Result<Option<T>, ApiError>
    where
        T: FromStr + PartialOrd + Display,
    {
        let Some(text) = self.get(key)? else {
            return Ok(None);
        };
        match text.parse::<T>() {
            Ok(value) if range.contains(&value) => Ok(Some(value)),
            _ => Err(ApiError::invalid(format!(
                "`{key}` must be an integer from {} to {}",
                range.start(),
                range.end()
            ))),
        }
    }

    /// Unix epoch milliseconds, from 0.
    fn millis(&self, key: &str) -> Result<Option<EpochMillis>, ApiError> {
        let Some(text) = self.get(key)? else {
            return Ok(None);
        };
        match text.parse::<i64>() {
            Ok(millis) if millis >= 0 => Ok(Some(EpochMillis::from_millis(millis))),
            _ => Err(ApiError::invalid(format!(
                "`{key}` must be Unix epoch milliseconds"
            ))),
        }
    }

    fn task(&self) -> Result<Option<TaskId>, ApiError> {
        self.get("taskId")?
            .map(|text| {
                TaskId::parse(text)
                    .map_err(|_| ApiError::invalid("`taskId` must be 1 to 128 bytes"))
            })
            .transpose()
    }

    fn tab(&self) -> Result<Option<TabId>, ApiError> {
        self.get("tab")?
            .map(|text| {
                text.parse()
                    .map(TabId)
                    .map_err(|_| ApiError::invalid("`tab` must be an integer"))
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::{Inline, events, run, run_id, runs, tab_id, trace};
    use crate::api_error::ErrorCode;
    use crate::limits::{DEFAULT_LIMIT, DEFAULT_TAIL};
    use axum::http::StatusCode;
    use browseros_core::TabId;
    use browseros_trace::{EpochMillis, RunId, RunStatus, TaskId};
    use std::time::Duration;

    fn refused<T: std::fmt::Debug>(result: Result<T, crate::api_error::ApiError>) -> bool {
        result.is_err_and(|error| {
            error.status == StatusCode::BAD_REQUEST && error.code == ErrorCode::InvalidRequest
        })
    }

    #[test]
    fn run_filters_parse_into_the_store_query() {
        let run = RunId::new();
        let raw = format!(
            "requestId=call%207&taskId=checkout&tab=12&status=timeout&since=5&until=9\
             &cursor=1700:{run}&limit=3&future=ignored"
        );
        let query = runs(Some(&raw));
        let Ok(query) = query else {
            panic!("{query:?}");
        };
        assert_eq!(
            query.request_id.map(|id| id.to_string()).as_deref(),
            Some("call 7")
        );
        assert_eq!(query.task_id, TaskId::parse("checkout").ok());
        assert_eq!(query.tab, Some(TabId(12)));
        assert_eq!(query.status, Some(RunStatus::Timeout));
        assert_eq!(query.since, Some(EpochMillis::from_millis(5)));
        assert_eq!(query.until, Some(EpochMillis::from_millis(9)));
        assert_eq!(query.cursor, Some(format!("1700:{run}")));
        assert_eq!(query.limit, 3);
        assert_eq!(
            runs(None).map(|query| query.limit).ok(),
            Some(DEFAULT_LIMIT)
        );
    }

    #[test]
    fn bad_run_filters_are_refused() {
        let run = RunId::new();
        for raw in [
            "limit=0".to_string(),
            "limit=101".to_string(),
            "limit=ten".to_string(),
            "since=-1".to_string(),
            "until=soon".to_string(),
            "status=done".to_string(),
            "tab=front".to_string(),
            "taskId=".to_string(),
            "requestId=".to_string(),
            format!("requestId={}", "r".repeat(129)),
            "cursor=12".to_string(),
            format!("cursor=later:{run}"),
            "cursor=12:not-a-run".to_string(),
            "limit=2&limit=3".to_string(),
        ] {
            assert!(refused(runs(Some(&raw))), "{raw}");
        }
    }

    #[test]
    fn wait_is_whole_seconds_up_to_a_minute() {
        assert_eq!(run(None).ok(), Some((Duration::ZERO, Inline::None)));
        assert_eq!(
            run(Some("wait=60&include=screenshots")).ok(),
            Some((Duration::from_secs(60), Inline::All))
        );
        for raw in [
            "wait=61",
            "wait=-1",
            "wait=1.5",
            "wait=soon",
            "include=recording",
        ] {
            assert!(refused(run(Some(raw))), "{raw}");
        }
    }

    #[test]
    fn the_trace_tail_defaults_to_100_and_caps_at_1000() {
        let tail = |raw| trace(raw).map(|(query, _)| query.tail).ok();
        assert_eq!(tail(None), Some(DEFAULT_TAIL));
        assert_eq!(tail(Some("tail=1000")), Some(1000));
        assert!(refused(trace(Some("tail=1001"))));
        assert!(refused(trace(Some("tail=0"))));
        let query = trace(Some("taskId=t&tab=4")).ok();
        assert_eq!(
            query.as_ref().and_then(|(query, _)| query.tab),
            Some(TabId(4))
        );
    }

    #[test]
    fn trace_screenshots_are_none_all_or_the_last_1_to_50() {
        let inline = |raw| trace(Some(raw)).map(|(_, inline)| inline).ok();
        assert_eq!(
            trace(None).map(|(_, inline)| inline).ok(),
            Some(Inline::None)
        );
        assert_eq!(inline("screenshots=none"), Some(Inline::None));
        assert_eq!(inline("screenshots=all"), Some(Inline::All));
        assert_eq!(inline("screenshots=last:4"), Some(Inline::Last(4)));
        assert_eq!(inline("screenshots=last:50"), Some(Inline::Last(50)));
        for raw in [
            "screenshots=last:0",
            "screenshots=last:51",
            "screenshots=last:",
            "screenshots=last",
            "screenshots=some",
        ] {
            assert!(refused(trace(Some(raw))), "{raw}");
        }
    }

    #[test]
    fn event_filters_take_a_task_and_a_tab() {
        let filter = events(Some("taskId=checkout&tab=4")).ok();
        assert_eq!(
            filter.as_ref().and_then(|filter| filter.task.clone()),
            TaskId::parse("checkout").ok()
        );
        assert_eq!(filter.and_then(|filter| filter.tab), Some(TabId(4)));
        assert!(refused(events(Some("tab=x"))));
    }

    #[test]
    fn a_path_id_that_names_nothing_is_a_404_and_a_bad_tab_a_400() {
        assert!(run_id("nope").is_err_and(|error| error.status == StatusCode::NOT_FOUND));
        assert!(refused(tab_id("front")));
        assert_eq!(tab_id("7").ok(), Some(TabId(7)));
    }
}
