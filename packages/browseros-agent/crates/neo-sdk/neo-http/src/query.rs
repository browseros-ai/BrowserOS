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
use neo_screencast::Quality;
use neo_takeover::{FitId, Follow, LockId, TakeoverId};
use neo_trace::{
    EpochMillis, OperationFilter, OperationId, RequestId, RunId, RunQuery, RunStatus, TailQuery,
    TaskId,
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Inline {
    #[default]
    None,
    All,
    /// The latest N operations that have one.
    Last(u32),
}

/// Which stored values the wire projection may include. Storage and recording are unchanged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Fields {
    #[default]
    Full,
    Summary,
}

/// Independent field projection and row selection, shared by the run-list audit view.
#[derive(Default)]
pub(crate) struct View {
    pub fields: Fields,
    pub operations: OperationFilter,
}

pub(crate) fn view(raw: Option<&str>) -> Result<View, ApiError> {
    let params = Params::parse(raw);
    Ok(View {
        fields: params.fields()?,
        operations: params.operations()?,
    })
}

/// A run read's independent wait, image and operation-view policies.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RunRead {
    pub wait: Duration,
    pub inline: Inline,
    pub fields: Fields,
    pub operations: OperationFilter,
}

/// A trace window, filtered in storage before its limit, and projected at the wire boundary.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct TraceRead {
    pub query: TailQuery,
    pub inline: Inline,
    pub fields: Fields,
}

/// `GET /v1/runs/{runId}`: how long `wait` may hold the request, and `include=screenshots`.
pub(crate) fn run(raw: Option<&str>) -> Result<RunRead, ApiError> {
    let params = Params::parse(raw);
    let seconds = params.number("wait", 0..=MAX_WAIT_SECONDS)?;
    let inline = match params.get("include")? {
        None => Inline::None,
        Some("screenshots") => Inline::All,
        Some(_) => return Err(ApiError::invalid("`include` must be `screenshots`")),
    };
    Ok(RunRead {
        wait: Duration::from_secs(seconds.unwrap_or(0)),
        inline,
        fields: params.fields()?,
        operations: params.operations()?,
    })
}

/// `GET /v1/trace`, with `screenshots=none|all|last:N`.
pub(crate) fn trace(raw: Option<&str>) -> Result<TraceRead, ApiError> {
    let params = Params::parse(raw);
    let query = TailQuery {
        task_id: params.task()?,
        tab: params.tab()?,
        tail: params.number("tail", 1..=MAX_TAIL)?.unwrap_or(DEFAULT_TAIL),
        operations: params.operations()?,
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
    Ok(TraceRead {
        query,
        inline,
        fields: params.fields()?,
    })
}

/// `GET /v1/events`.
pub(crate) fn events(raw: Option<&str>) -> Result<EventFilter, ApiError> {
    let params = Params::parse(raw);
    Ok(EventFilter {
        task: params.task()?,
        tab: params.tab()?,
    })
}

/// `GET /v1/display`, parsed: which display, before any takeover is looked up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DisplayQuery {
    /// `mode=view`, and what it keeps in front.
    View(Option<Follow>),
    /// `mode=control`, and the takeover it belongs to (`release`).
    Control(Option<TakeoverId>),
}

/// `GET /v1/display`: `mode` is required; a view may take `followTab` or `followTask` (one of
/// them, decision 18), and a control display `release`. Any other combination is a 400. A
/// `release` that is not a takeover id names no takeover: a 404.
pub(crate) fn display(raw: Option<&str>) -> Result<DisplayQuery, ApiError> {
    let params = Params::parse(raw);
    let follow_tab = params
        .get("followTab")?
        .map(|text| {
            text.parse()
                .map(TabId)
                .map_err(|_| ApiError::invalid("`followTab` must be an integer"))
        })
        .transpose()?;
    let follow_task = params
        .get("followTask")?
        .map(|text| {
            TaskId::parse(text)
                .map_err(|_| ApiError::invalid("`followTask` must be 1 to 128 bytes"))
        })
        .transpose()?;
    let follow = match (follow_tab, follow_task) {
        (Some(_), Some(_)) => {
            return Err(ApiError::invalid(
                "`followTab` and `followTask` cannot both be given",
            ));
        }
        (Some(tab), None) => Some(Follow::Tab(tab)),
        (None, Some(task)) => Some(Follow::Task(task)),
        (None, None) => None,
    };
    let release = params.get("release")?;
    match params.get("mode")? {
        Some("view") if release.is_some() => {
            Err(ApiError::invalid("`release` is for a control display"))
        }
        Some("view") => Ok(DisplayQuery::View(follow)),
        Some("control") if follow.is_some() => Err(ApiError::invalid(
            "a control display follows nothing: the owner moves the screen",
        )),
        Some("control") => release
            .map(|text| {
                text.parse()
                    .map_err(|_| ApiError::not_found("no takeover has this id"))
            })
            .transpose()
            .map(DisplayQuery::Control),
        _ => Err(ApiError::invalid("`mode` must be view or control")),
    }
}

/// `GET /v1/tabs/{tabId}/screencast` and `/video`: `quality`, `high` when absent.
pub(crate) fn live(raw: Option<&str>) -> Result<Quality, ApiError> {
    match Params::parse(raw).get("quality")? {
        None => Ok(Quality::High),
        Some(text) => {
            Quality::parse(text).ok_or_else(|| ApiError::invalid("`quality` must be low or high"))
        }
    }
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

/// A lock id (`lk_<uuid>`) in a path. Text that is not one names no lock.
pub(crate) fn lock_id(text: &str) -> Result<LockId, ApiError> {
    text.parse()
        .map_err(|_| ApiError::not_found("no lock has this id"))
}

/// A fit id (`ft_<uuid>`) in a path. Text that is not one names no fit.
pub(crate) fn fit_id(text: &str) -> Result<FitId, ApiError> {
    text.parse()
        .map_err(|_| ApiError::not_found("no fit has this id"))
}

/// A takeover id (`tk_<uuid>`) in a path. Text that is not one names no takeover.
pub(crate) fn takeover_id(text: &str) -> Result<TakeoverId, ApiError> {
    text.parse()
        .map_err(|_| ApiError::not_found("no takeover has this id"))
}

/// `GET /v1/takeovers/{takeoverId}`: how long to wait for the takeover's release. `wait` holds
/// the request until the state differs from `state`, `active` when absent. Release is the only
/// change a takeover goes through, so `state=released` has nothing to wait for: an active
/// takeover already differs from it, and a released one never changes again.
pub(crate) fn takeover(raw: Option<&str>) -> Result<Duration, ApiError> {
    let params = Params::parse(raw);
    let wait = Duration::from_secs(params.number("wait", 0..=MAX_WAIT_SECONDS)?.unwrap_or(0));
    match params.get("state")? {
        None | Some("active") => Ok(wait),
        Some("released") => Ok(Duration::ZERO),
        Some(_) => Err(ApiError::invalid("`state` must be active or released")),
    }
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
    fn fields(&self) -> Result<Fields, ApiError> {
        match self.get("fields")? {
            None | Some("full") => Ok(Fields::Full),
            Some("summary") => Ok(Fields::Summary),
            Some(_) => Err(ApiError::invalid("`fields` must be full or summary")),
        }
    }

    fn operations(&self) -> Result<OperationFilter, ApiError> {
        match self.get("internal")? {
            None | Some("true") => Ok(OperationFilter::All),
            Some("false") => Ok(OperationFilter::Script),
            Some(_) => Err(ApiError::invalid("`internal` must be true or false")),
        }
    }
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
    use super::{
        DisplayQuery, Inline, display, events, live, run, run_id, runs, tab_id, takeover, trace,
    };
    use crate::api_error::ErrorCode;
    use crate::limits::{DEFAULT_LIMIT, DEFAULT_TAIL};
    use axum::http::StatusCode;
    use browseros_core::TabId;
    use neo_screencast::Quality;
    use neo_takeover::{Follow, TakeoverId};
    use neo_trace::{EpochMillis, RunId, RunStatus, TaskId};
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
        assert_eq!(
            run(None).map(|read| (read.wait, read.inline)).ok(),
            Some((Duration::ZERO, Inline::None))
        );
        assert_eq!(
            run(Some("wait=60&include=screenshots"))
                .map(|read| (read.wait, read.inline))
                .ok(),
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
    fn a_takeover_is_waited_for_only_while_it_is_seen_active() {
        assert_eq!(takeover(None).ok(), Some(Duration::ZERO));
        assert_eq!(
            takeover(Some("wait=60")).ok(),
            Some(Duration::from_secs(60))
        );
        assert_eq!(
            takeover(Some("wait=60&state=active")).ok(),
            Some(Duration::from_secs(60))
        );
        assert_eq!(
            takeover(Some("wait=60&state=released")).ok(),
            Some(Duration::ZERO),
            "nothing changes after the release"
        );
        for raw in ["wait=61", "state=expired", "state=active&state=released"] {
            assert!(refused(takeover(Some(raw))), "{raw}");
        }
    }

    #[test]
    fn the_trace_tail_defaults_to_100_and_caps_at_1000() {
        let tail = |raw| trace(raw).map(|read| read.query.tail).ok();
        assert_eq!(tail(None), Some(DEFAULT_TAIL));
        assert_eq!(tail(Some("tail=1000")), Some(1000));
        assert!(refused(trace(Some("tail=1001"))));
        assert!(refused(trace(Some("tail=0"))));
        let query = trace(Some("taskId=t&tab=4")).ok();
        assert_eq!(
            query.as_ref().and_then(|read| read.query.tab),
            Some(TabId(4))
        );
    }

    #[test]
    fn trace_screenshots_are_none_all_or_the_last_1_to_50() {
        let inline = |raw| trace(Some(raw)).map(|read| read.inline).ok();
        assert_eq!(trace(None).map(|read| read.inline).ok(), Some(Inline::None));
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
    fn a_display_is_a_view_that_may_follow_or_a_control_display_that_may_be_bound() {
        let takeover = "tk_01900000-0000-7000-8000-000000000000";
        let cases = [
            ("mode=view", Some(DisplayQuery::View(None))),
            (
                "mode=view&followTab=12",
                Some(DisplayQuery::View(Some(Follow::Tab(TabId(12))))),
            ),
            (
                "mode=view&followTask=checkout%201",
                TaskId::parse("checkout 1")
                    .ok()
                    .map(|task| DisplayQuery::View(Some(Follow::Task(task)))),
            ),
            ("mode=control", Some(DisplayQuery::Control(None))),
            (
                &format!("mode=control&release={takeover}"),
                takeover
                    .parse::<TakeoverId>()
                    .ok()
                    .map(|id| DisplayQuery::Control(Some(id))),
            ),
            ("mode=view&future=ignored", Some(DisplayQuery::View(None))),
        ];
        for (raw, expected) in cases {
            assert!(expected.is_some(), "{raw}: the case itself is malformed");
            assert_eq!(display(Some(raw)).ok(), expected, "{raw}");
        }
    }

    #[test]
    fn a_display_refuses_contradictions_and_a_release_that_names_no_takeover() {
        for raw in [
            "",
            "mode=",
            "mode=watch",
            "mode=view&mode=view",
            "mode=view&followTab=12&followTask=checkout",
            "mode=view&followTab=front",
            "mode=view&followTask=",
            "mode=view&release=tk_01900000-0000-7000-8000-000000000000",
            "mode=control&followTab=12",
            "mode=control&followTask=checkout",
        ] {
            assert!(refused(display(Some(raw))), "{raw}");
        }
        assert!(refused(display(None)), "no mode");
        for raw in ["mode=control&release=nope", "mode=control&release=lk_x"] {
            assert!(
                display(Some(raw)).is_err_and(|error| error.status == StatusCode::NOT_FOUND),
                "{raw}"
            );
        }
    }

    #[test]
    fn live_quality_is_low_or_high_and_high_when_absent() {
        assert_eq!(live(None).ok(), Some(Quality::High));
        assert_eq!(live(Some("tab=3&other=1")).ok(), Some(Quality::High));
        assert_eq!(live(Some("quality=low")).ok(), Some(Quality::Low));
        assert_eq!(live(Some("quality=high")).ok(), Some(Quality::High));
        for raw in [
            "quality=medium",
            "quality=",
            "quality=LOW",
            "quality=low&quality=high",
        ] {
            assert!(refused(live(Some(raw))), "{raw}");
        }
    }

    #[test]
    fn a_path_id_that_names_nothing_is_a_404_and_a_bad_tab_a_400() {
        assert!(run_id("nope").is_err_and(|error| error.status == StatusCode::NOT_FOUND));
        assert!(refused(tab_id("front")));
        assert_eq!(tab_id("7").ok(), Some(TabId(7)));
    }
}
