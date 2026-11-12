//! Stored primitives become public trace records here. Unknown private reasons are handled
//! fail closed by TraceStore; other unknown enum/JSON/id values fail just the affected read.

use std::str::FromStr;

use browseros_core::TabId;
use sea_orm::FromQueryResult;
use serde::de::DeserializeOwned;

use super::entities::operations;
use crate::{
    Delivery, Effect, EpochMillis, IdError, OperationId, OperationKind, OperationRow, Outcome,
    PageState, RequestId, RunDetail, RunEffects, RunId, RunStatus, RunSummary, ShotState, TaskId,
    TraceError,
};

/// A list projection: deliberately excludes code, host data, results and replay bodies.
#[derive(FromQueryResult)]
pub(crate) struct RunSummaryRow {
    pub(crate) id: String,
    pub(crate) task_id: Option<String>,
    pub(crate) status: String,
    pub(crate) effects: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) request_id: Option<String>,
    pub(crate) started_at: i64,
    pub(crate) ended_at: Option<i64>,
    pub(crate) operation_count: i64,
    pub(crate) trace_gaps: i64,
    pub(crate) complete: bool,
    pub(crate) tabs: Option<String>,
}

/// Detail payloads alongside the same correlated summary projection used by list reads.
#[derive(FromQueryResult)]
pub(crate) struct RunDetailRow {
    #[sea_orm(nested)]
    pub(crate) summary: RunSummaryRow,
    pub(crate) host_data: Option<String>,
    pub(crate) code: Option<String>,
    pub(crate) code_truncated: bool,
    pub(crate) code_withheld: Option<String>,
    pub(crate) result: Option<String>,
    pub(crate) result_truncated: bool,
}

pub(crate) fn run_summary(row: RunSummaryRow) -> Result<RunSummary, TraceError> {
    Ok(RunSummary {
        id: parse_id(&row.id)?,
        task_id: host_id(row.task_id.as_deref(), "task_id", TaskId::parse)?,
        tabs: tabs(row.tabs.as_deref())?,
        status: RunStatus::parse(&row.status).ok_or_else(|| corrupt("status", &row.status))?,
        effects: row
            .effects
            .as_deref()
            .map(|text| RunEffects::parse(text).ok_or_else(|| corrupt("effects", text)))
            .transpose()?,
        error: json_column(row.error.as_deref())?,
        request_id: host_id(row.request_id.as_deref(), "request_id", RequestId::parse)?,
        started_at: EpochMillis::from_millis(row.started_at),
        ended_at: row.ended_at.map(EpochMillis::from_millis),
        operation_count: u64::try_from(row.operation_count).unwrap_or(0),
        trace_gaps: u64::try_from(row.trace_gaps).unwrap_or(0),
        complete: row.complete,
    })
}

pub(crate) fn run_detail(
    row: RunDetailRow,
    operations: Vec<OperationRow>,
) -> Result<RunDetail, TraceError> {
    Ok(RunDetail {
        summary: run_summary(row.summary)?,
        host_data: json_column(row.host_data.as_deref())?,
        code: row.code,
        code_truncated: row.code_truncated,
        code_withheld: row.code_withheld,
        result: json_column(row.result.as_deref())?,
        result_truncated: row.result_truncated,
        operations,
        downloads: Vec::new(),
    })
}

pub(crate) fn operation_row(row: operations::Model) -> Result<OperationRow, TraceError> {
    let run: RunId = parse_id(&row.run_id)?;
    let seq = u32::try_from(row.seq).map_err(|_| TraceError::Corrupt("operation seq".into()))?;
    let attempt = row.attempt.and_then(|value| u32::try_from(value).ok());
    Ok(OperationRow {
        site: json_column(row.site.as_deref())?,
        id: OperationId::new(run, seq),
        parent: row.parent_id.as_deref().map(parse_id).transpose()?,
        tab: row.target_tab_id.map(TabId),
        name: row.name,
        kind: operation_kind(&row.kind, attempt).ok_or_else(|| corrupt("kind", &row.kind))?,
        effect: match row.effect.as_str() {
            "read" => Effect::Read,
            "write" => Effect::Write,
            other => return Err(corrupt("effect", other)),
        },
        // Historical page_id values are deliberately ignored, without rewriting the row.
        page: PageState {
            tab_id: row.tab_id.map(TabId),
            url: row.url,
            title: row.title,
        },
        args: json_column(row.args.as_deref())?,
        args_truncated: row.args_truncated,
        result: json_column(row.result.as_deref())?,
        result_truncated: row.result_truncated,
        outcome: row
            .outcome
            .as_deref()
            .map(|text| {
                [Outcome::Ok, Outcome::Error, Outcome::Cancelled]
                    .into_iter()
                    .find(|value| value.as_str() == text)
                    .ok_or_else(|| corrupt("outcome", text))
            })
            .transpose()?,
        delivery: row
            .delivery
            .as_deref()
            .map(|text| {
                [Delivery::NotSent, Delivery::Done, Delivery::Unknown]
                    .into_iter()
                    .find(|value| value.as_str() == text)
                    .ok_or_else(|| corrupt("delivery", text))
            })
            .transpose()?,
        error: json_column(row.error.as_deref())?,
        notes: json_column(row.notes.as_deref())?.unwrap_or_default(),
        started_at: EpochMillis::from_millis(row.started_at),
        sent_at: row.sent_at.map(EpochMillis::from_millis),
        ended_at: row.ended_at.map(EpochMillis::from_millis),
        screenshot: row
            .screenshot
            .as_deref()
            .map(|text| ShotState::parse(text).ok_or_else(|| corrupt("screenshot", text)))
            .transpose()?,
    })
}

pub(crate) fn parse_id<T: FromStr>(text: &str) -> Result<T, TraceError> {
    text.parse()
        .map_err(|_| TraceError::Corrupt(format!("invalid id {text:?}")))
}

fn host_id<T>(
    text: Option<&str>,
    column: &str,
    parse: fn(&str) -> Result<T, IdError>,
) -> Result<Option<T>, TraceError> {
    text.map(|text| parse(text).map_err(|_| corrupt(column, text)))
        .transpose()
}

fn tabs(text: Option<&str>) -> Result<Vec<TabId>, TraceError> {
    let Some(text) = text else {
        return Ok(Vec::new());
    };
    let mut tabs = text
        .split(',')
        .map(|tab| tab.parse().map(TabId).map_err(|_| corrupt("tabs", text)))
        .collect::<Result<Vec<_>, _>>()?;
    tabs.sort_unstable();
    Ok(tabs)
}

pub(crate) fn json_error(error: serde_json::Error) -> TraceError {
    TraceError::Corrupt(error.to_string())
}

pub(crate) fn json_column<T: DeserializeOwned>(
    text: Option<&str>,
) -> Result<Option<T>, TraceError> {
    text.map(|text| serde_json::from_str(text).map_err(json_error))
        .transpose()
}

pub(crate) fn corrupt(column: &str, value: &str) -> TraceError {
    TraceError::Corrupt(format!("{column} {value:?}"))
}

fn operation_kind(text: &str, attempt: Option<u32>) -> Option<OperationKind> {
    match text {
        "builtin" => Some(OperationKind::Builtin),
        "defined" => Some(OperationKind::Defined),
        "next" => Some(OperationKind::Next {
            attempt: attempt.unwrap_or(1),
        }),
        "host_call" => Some(OperationKind::HostCall),
        "protected" => Some(OperationKind::Protected),
        _ => None,
    }
}
