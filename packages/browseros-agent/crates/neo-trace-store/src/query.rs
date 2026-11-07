//! Reads: each one an index range mapped to the public views. A run's rows are primary-key
//! ranges (`run_id` leads every child key), runs list by their ULID, sessions by activity, and a
//! host's id is one range of `(key, value)`. They run on the read pool, never on the writer's
//! connection, except the session head, which the writer reads right after its own commit.

use std::collections::{BTreeMap, HashMap};

use neo_runtime::{CallId, CallLocation, ConsoleLine, IdKey, PageId, PageRef, RunId, SessionKey};
use sea_orm::sea_query::Query;
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, EntityTrait, LoaderTrait, ModelTrait,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
};
use serde_json::Value;

use crate::StoreError;
use crate::db::entities::{
    trace_call_artifacts, trace_calls, trace_getters, trace_logs, trace_pages, trace_run_ids,
    trace_runs, trace_sessions,
};
use crate::db::kinds;
use crate::db::{signed, unsigned};
use crate::views::{
    AfterCall, CallQuery, CallTrace, GetterOutcome, GetterTrace, Listing, Loss, MAX_PAGE, OverCap,
    PageTrace, RunQuery, RunState, RunSummary, RunTrace, SessionQuery, SessionSummary,
    SessionTimeline, Settled, Timeline, TraceError,
};

pub(crate) async fn runs(
    db: &DatabaseConnection,
    query: &RunQuery,
) -> Result<Listing<RunSummary>, StoreError> {
    let mut select = trace_runs::Entity::find();
    if let Some(session) = &query.session {
        select = select.filter(trace_runs::Column::SessionKey.eq(session.as_str()));
    }
    if let Some((key, value)) = &query.id {
        select = select.filter(
            trace_runs::Column::RunId.in_subquery(
                Query::select()
                    .column(trace_run_ids::Column::RunId)
                    .from(trace_run_ids::Entity)
                    .and_where(trace_run_ids::Column::Key.eq(key.as_str()))
                    .and_where(trace_run_ids::Column::Value.eq(value.as_str()))
                    .to_owned(),
            ),
        );
    }
    if let Some(before) = query.before {
        select = select.filter(trace_runs::Column::RunId.lt(before.to_string()));
    }
    let limit = page_size(query.limit);
    let mut rows = select
        .order_by_desc(trace_runs::Column::RunId)
        .limit(limit + 1)
        .all(db)
        .await
        .map_err(StoreError::db)?;
    let more = trim(&mut rows, limit);
    Ok(Listing {
        items: summaries(db, rows).await?,
        more,
    })
}

pub(crate) async fn run(
    db: &DatabaseConnection,
    id: RunId,
) -> Result<Option<RunTrace>, StoreError> {
    let Some(row) = trace_runs::Entity::find_by_id(id.to_string())
        .one(db)
        .await
        .map_err(StoreError::db)?
    else {
        return Ok(None);
    };
    let ids = row
        .find_related(trace_run_ids::Entity)
        .all(db)
        .await
        .map_err(StoreError::db)?;
    let logs = row
        .find_related(trace_logs::Entity)
        .order_by_asc(trace_logs::Column::LineNo)
        .all(db)
        .await
        .map_err(StoreError::db)?;
    let pages = row
        .find_related(trace_pages::Entity)
        .order_by_asc(trace_pages::Column::OpenedOffsetUs)
        .all(db)
        .await
        .map_err(StoreError::db)?;
    let value = row
        .value_json
        .as_deref()
        .map(|json| parse_json(json, "value_json"))
        .transpose()?;
    Ok(Some(RunTrace {
        script: row.script.clone(),
        script_truncated: row.script_truncated,
        value,
        value_truncated: row.value_truncated,
        logs: logs
            .into_iter()
            .map(|line| ConsoleLine {
                offset_us: unsigned(line.offset_us),
                text: line.text,
            })
            .collect(),
        pages: pages.into_iter().map(page_trace).collect(),
        summary: summary(row, ids)?,
    }))
}

pub(crate) async fn calls(
    db: &DatabaseConnection,
    run: RunId,
    query: CallQuery,
) -> Result<Listing<CallTrace>, StoreError> {
    let Some(row) = trace_runs::Entity::find_by_id(run.to_string())
        .one(db)
        .await
        .map_err(StoreError::db)?
    else {
        return Ok(Listing {
            items: Vec::new(),
            more: false,
        });
    };
    let limit = page_size(query.limit);
    let mut rows = row
        .find_related(trace_calls::Entity)
        .filter(trace_calls::Column::CallNo.gt(signed(query.after.unwrap_or(0))))
        .order_by_asc(trace_calls::Column::CallNo)
        .limit(limit + 1)
        .all(db)
        .await
        .map_err(StoreError::db)?;
    let more = trim(&mut rows, limit);
    let settled = HashMap::from([(row.run_id.clone(), row.settled_at_ms.is_some())]);
    let artifacts = artifacts_for(db, &rows).await?;
    Ok(Listing {
        items: call_traces(rows, &settled, artifacts)?,
        more,
    })
}

pub(crate) async fn getters(
    db: &DatabaseConnection,
    run: RunId,
) -> Result<Vec<GetterTrace>, StoreError> {
    let Some(row) = trace_runs::Entity::find_by_id(run.to_string())
        .one(db)
        .await
        .map_err(StoreError::db)?
    else {
        return Ok(Vec::new());
    };
    row.find_related(trace_getters::Entity)
        .order_by_asc(trace_getters::Column::GetterNo)
        .all(db)
        .await
        .map_err(StoreError::db)?
        .into_iter()
        .map(getter_trace)
        .collect()
}

pub(crate) async fn session(
    db: &DatabaseConnection,
    key: &SessionKey,
    scope: Timeline,
) -> Result<Option<SessionTimeline>, StoreError> {
    let Some(session) = trace_sessions::Entity::find_by_id(key.as_str())
        .filter(trace_sessions::Column::Runs.gt(0))
        .one(db)
        .await
        .map_err(StoreError::db)?
    else {
        return Ok(None);
    };
    let runs = trace_runs::Entity::find()
        .filter(trace_runs::Column::SessionKey.eq(key.as_str()))
        .order_by_asc(trace_runs::Column::RunId)
        .all(db)
        .await
        .map_err(StoreError::db)?;
    let settled: HashMap<String, bool> = runs
        .iter()
        .map(|run| (run.run_id.clone(), run.settled_at_ms.is_some()))
        .collect();
    let in_session = trace_calls::Entity::find()
        .inner_join(trace_runs::Entity)
        .filter(trace_runs::Column::SessionKey.eq(key.as_str()));
    let (calls, calls_truncated) = match scope {
        Timeline::All => (
            in_session
                .order_by_asc(trace_calls::Column::RunId)
                .order_by_asc(trace_calls::Column::CallNo)
                .all(db)
                .await
                .map_err(StoreError::db)?,
            false,
        ),
        Timeline::LastCalls(last) => {
            let last = u64::from(last);
            let mut newest = in_session
                .order_by_desc(trace_calls::Column::RunId)
                .order_by_desc(trace_calls::Column::CallNo)
                .limit(last + 1)
                .all(db)
                .await
                .map_err(StoreError::db)?;
            let truncated = trim(&mut newest, last);
            newest.reverse();
            (newest, truncated)
        }
    };
    let artifacts = artifacts_for(db, &calls).await?;
    Ok(Some(SessionTimeline {
        session: session_summary(session)?,
        runs: summaries(db, runs).await?,
        calls: call_traces(calls, &settled, artifacts)?,
        calls_truncated,
    }))
}

pub(crate) async fn sessions(
    db: &DatabaseConnection,
    query: &SessionQuery,
) -> Result<Listing<SessionSummary>, StoreError> {
    let mut select = trace_sessions::Entity::find().filter(trace_sessions::Column::Runs.gt(0));
    if let Some((at, key)) = &query.before {
        let at = signed(*at);
        select = select.filter(
            Condition::any()
                .add(trace_sessions::Column::LastActivityMs.lt(at))
                .add(
                    Condition::all()
                        .add(trace_sessions::Column::LastActivityMs.eq(at))
                        .add(trace_sessions::Column::SessionKey.lt(key.as_str())),
                ),
        );
    }
    let limit = page_size(query.limit);
    let mut rows = select
        .order_by_desc(trace_sessions::Column::LastActivityMs)
        .order_by_desc(trace_sessions::Column::SessionKey)
        .limit(limit + 1)
        .all(db)
        .await
        .map_err(StoreError::db)?;
    let more = trim(&mut rows, limit);
    Ok(Listing {
        items: rows
            .into_iter()
            .map(session_summary)
            .collect::<Result<_, _>>()?,
        more,
    })
}

pub(crate) async fn knows(db: &DatabaseConnection, key: &SessionKey) -> Result<bool, StoreError> {
    let known = trace_sessions::Entity::find_by_id(key.as_str())
        .filter(trace_sessions::Column::Runs.gt(0))
        .count(db)
        .await
        .map_err(StoreError::db)?;
    Ok(known > 0)
}

/// Rows of runs, with their ids loaded in one more query, as summaries.
async fn summaries(
    db: &DatabaseConnection,
    rows: Vec<trace_runs::Model>,
) -> Result<Vec<RunSummary>, StoreError> {
    let ids = rows
        .load_many(trace_run_ids::Entity, db)
        .await
        .map_err(StoreError::db)?;
    rows.into_iter()
        .zip(ids)
        .map(|(row, ids)| summary(row, ids))
        .collect()
}

/// The artifacts of `calls`' runs and numbers, by `(run, call)`, from one range per run.
async fn artifacts_for(
    db: &DatabaseConnection,
    calls: &[trace_calls::Model],
) -> Result<HashMap<(String, i64), trace_call_artifacts::Model>, StoreError> {
    let mut ranges: BTreeMap<&str, (i64, i64)> = BTreeMap::new();
    for call in calls {
        let range = ranges
            .entry(call.run_id.as_str())
            .or_insert((call.call_no, call.call_no));
        range.0 = range.0.min(call.call_no);
        range.1 = range.1.max(call.call_no);
    }
    let mut found = HashMap::new();
    for (run, (first, last)) in ranges {
        let rows = trace_call_artifacts::Entity::find()
            .filter(trace_call_artifacts::Column::RunId.eq(run))
            .filter(trace_call_artifacts::Column::CallNo.between(first, last))
            .all(db)
            .await
            .map_err(StoreError::db)?;
        for row in rows {
            found.insert((row.run_id.clone(), row.call_no), row);
        }
    }
    Ok(found)
}

fn summary(
    row: trace_runs::Model,
    ids: Vec<trace_run_ids::Model>,
) -> Result<RunSummary, StoreError> {
    let ids = ids
        .into_iter()
        .map(|id| {
            IdKey::new(&id.key)
                .map(|key| (key, id.value))
                .map_err(|_| StoreError::Row(format!("trace_run_ids.key {:?}", id.key)))
        })
        .collect::<Result<_, _>>()?;
    Ok(RunSummary {
        id: run_id(&row.run_id)?,
        session: SessionKey::new(&row.session_key),
        client: row.client,
        ids,
        start_page: row.start_page.map(PageId::new),
        timeout_ms: row.timeout_ms.map(unsigned),
        code_bytes: row.code_bytes.map(unsigned),
        started_at_ms: unsigned(row.started_at_ms),
        state: row.status.map_or(RunState::Running, kinds::RunStatus::view),
        end_cause: row.end_cause.map(Into::into),
        error: row.error_name.map(|name| TraceError {
            name,
            message: row.error_message,
        }),
        ended_at_ms: row.ended_at_ms.map(unsigned),
        duration_us: row.duration_us.map(unsigned),
        calls_made: unsigned(row.calls_made),
        unreadable_calls: unsigned(row.unreadable_calls),
        over_cap: OverCap {
            calls: unsigned(row.calls_over_cap),
            delivery: row.over_cap_delivery.map(Into::into),
            getters: unsigned(row.getters_over_cap),
        },
        settled: row
            .settled_at_ms
            .zip(row.settled_by)
            .map(|(at, by)| Settled {
                at_ms: unsigned(at),
                by: by.view(),
            }),
        loss: Loss {
            stubbed: unsigned(row.events_stubbed),
            dropped: unsigned(row.events_dropped),
            incomplete: row.incomplete,
        },
    })
}

fn call_traces(
    rows: Vec<trace_calls::Model>,
    run_settled: &HashMap<String, bool>,
    mut artifacts: HashMap<(String, i64), trace_call_artifacts::Model>,
) -> Result<Vec<CallTrace>, StoreError> {
    rows.into_iter()
        .map(|row| {
            let after = artifacts
                .remove(&(row.run_id.clone(), row.call_no))
                .map(after_call);
            let settled =
                row.delivery_settled || run_settled.get(&row.run_id).copied().unwrap_or(false);
            call_trace(row, settled, after)
        })
        .collect()
}

fn call_trace(
    row: trace_calls::Model,
    settled: bool,
    after: Option<AfterCall>,
) -> Result<CallTrace, StoreError> {
    let run = run_id(&row.run_id)?;
    let log = match row.log_json.as_deref() {
        Some(json) => serde_json::from_str(json)
            .map_err(|error| StoreError::Row(format!("trace_calls.log_json: {error}")))?,
        None => Vec::new(),
    };
    Ok(CallTrace {
        id: CallId {
            run,
            no: unsigned(row.call_no),
        },
        parent: row.parent_no.map(unsigned),
        object: row.object,
        page: page_ref(row.page_id, row.tab_id),
        method: row.method,
        title: row.title,
        selector: row.selector,
        params: row
            .params_json
            .as_deref()
            .map(|json| parse_json(json, "params_json"))
            .transpose()?
            .unwrap_or(Value::Null),
        params_truncated: row.params_truncated,
        timeout_ms: row.timeout_ms,
        location: row.loc_line.map(|line| CallLocation {
            line: u32::try_from(line).unwrap_or(0),
            column: row.loc_column.and_then(|column| u32::try_from(column).ok()),
        }),
        started_at_ms: unsigned(row.started_at_ms),
        offset_us: unsigned(row.offset_us),
        duration_us: row.duration_us.map(unsigned),
        input_offset_us: row.input_offset_us.map(unsigned),
        state: row.state.view(),
        error: row.error_name.map(|name| TraceError {
            name,
            message: row.error_message,
        }),
        unfinished: row.unfinished_reason.map(kinds::UnfinishedReason::view),
        delivery: row.delivery.map(Into::into),
        settled,
        log,
        detail: row.detail.view(),
        after,
    })
}

fn getter_trace(row: trace_getters::Model) -> Result<GetterTrace, StoreError> {
    let outcome = match row.outcome {
        kinds::GetterOutcome::Ok => GetterOutcome::Ok {
            summary: row
                .summary_json
                .as_deref()
                .map(|json| parse_json(json, "summary_json"))
                .transpose()?
                .unwrap_or(Value::Null),
        },
        kinds::GetterOutcome::Error => GetterOutcome::Error(TraceError {
            name: row.error_name.unwrap_or_default(),
            message: row.error_message,
        }),
    };
    Ok(GetterTrace {
        run: run_id(&row.run_id)?,
        no: unsigned(row.getter_no),
        object: row.object,
        field: serde_json::from_value(Value::String(row.field))
            .map_err(|error| StoreError::Row(format!("getter field: {error}")))?,
        page: page_ref(row.page_id, row.tab_id),
        outcome,
        offset_us: unsigned(row.offset_us),
        duration_us: unsigned(row.duration_us),
        detail: row.detail.view(),
    })
}

fn page_trace(row: trace_pages::Model) -> PageTrace {
    PageTrace {
        page: PageRef {
            target: PageId::new(row.page_id),
            tab_id: row.tab_id,
        },
        created: row.created,
        by_call: row.by_call.map(unsigned),
        opened_offset_us: row.opened_offset_us.map(unsigned),
        closed_offset_us: row.closed_offset_us.map(unsigned),
    }
}

fn after_call(row: trace_call_artifacts::Model) -> AfterCall {
    AfterCall {
        tab_id: row.tab_id,
        url: row.url,
        title: row.title,
        jpeg: row.jpeg,
        captured_at_ms: unsigned(row.captured_at_ms),
    }
}

fn session_summary(row: trace_sessions::Model) -> Result<SessionSummary, StoreError> {
    Ok(SessionSummary {
        key: SessionKey::new(&row.session_key),
        client: row.client,
        first_run: row.first_run_id.as_deref().map(run_id).transpose()?,
        last_run: row.last_run_id.as_deref().map(run_id).transpose()?,
        runs: unsigned(row.runs),
        last_activity_ms: unsigned(row.last_activity_ms),
        withheld: row.withheld,
    })
}

fn page_ref(page_id: Option<String>, tab_id: Option<i64>) -> Option<PageRef> {
    page_id.map(|target| PageRef {
        target: PageId::new(target),
        tab_id,
    })
}

fn run_id(text: &str) -> Result<RunId, StoreError> {
    text.parse()
        .map_err(|_| StoreError::Row(format!("run id {text:?}")))
}

fn parse_json(json: &str, column: &str) -> Result<Value, StoreError> {
    serde_json::from_str(json).map_err(|error| StoreError::Row(format!("{column}: {error}")))
}

fn page_size(limit: u32) -> u64 {
    u64::from(limit.min(MAX_PAGE))
}

/// Drops the probe row past `limit` and says whether there was one.
fn trim<T>(rows: &mut Vec<T>, limit: u64) -> bool {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let more = rows.len() > limit;
    rows.truncate(limit);
    more
}
