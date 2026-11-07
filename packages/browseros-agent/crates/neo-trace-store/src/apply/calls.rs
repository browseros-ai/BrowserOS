//! Neo: Call and getter projections. Call ends and delivery are separate guarded upserts: replaying
//! an older end must never overwrite a delivery acknowledged after that end was published.

use neo_runtime::{CallResult, GetterResult, RunEvent};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{DatabaseTransaction, DbErr, EntityTrait, IntoActiveModel};

use super::{CALL_CAP, GETTER_CAP, detail};
use crate::db::entities::{trace_calls, trace_getters, trace_runs};
use crate::db::kinds::{CallState, Delivery, Detail, GetterOutcome};
use crate::db::signed;

pub(crate) async fn call(
    txn: &DatabaseTransaction,
    run: &mut trace_runs::Model,
    event: &RunEvent,
    stub: bool,
) -> Result<(), DbErr> {
    let no = match event {
        RunEvent::BeforeCall { call, .. }
        | RunEvent::AfterCall { call, .. }
        | RunEvent::InputSent { call, .. }
        | RunEvent::DeliverySettled { call, .. } => *call,
        _ => return Ok(()),
    };
    if no > CALL_CAP {
        // Numbers are dense arrival order from the runtime. An absolute high-water count makes
        // hints, end, final delivery, and a replay of all three count each excess call once.
        run.calls_over_cap = run.calls_over_cap.max(signed(no - CALL_CAP));
        let delivery = match event {
            RunEvent::AfterCall { record, .. } => record.delivery,
            RunEvent::DeliverySettled { delivery, .. } => Some(*delivery),
            _ => None,
        };
        run.over_cap_delivery = run
            .over_cap_delivery
            .map(Into::into)
            .max(delivery)
            .map(Into::into);
        return Ok(());
    }
    let old = trace_calls::Entity::find_by_id((run.run_id.clone(), signed(no)))
        .one(txn)
        .await?;
    let mut row = old.clone().unwrap_or_else(|| empty_call(run, no));
    match event {
        RunEvent::BeforeCall { .. } => begin_call(txn, run, event, row, stub).await?,
        RunEvent::AfterCall { .. } => finish_call(txn, run, event, row, stub).await?,
        RunEvent::InputSent { offset_us, .. } => {
            if row.input_offset_us.is_none() {
                row.input_offset_us = Some(signed(*offset_us));
            }
            if !row.delivery_settled {
                row.delivery = Some(Delivery::Unknown);
            }
            insert(txn, row.clone(), None).await?;
            if row.state == CallState::Running && !row.delivery_settled {
                delivery_update(txn, row, Some(Delivery::Unknown), false).await?;
            }
        }
        RunEvent::DeliverySettled { delivery, .. } => {
            if old.is_none() {
                insert(txn, row.clone(), None).await?;
            }
            delivery_update(txn, row, Some((*delivery).into()), true).await?;
        }
        _ => {}
    }
    Ok(())
}

/// Hints enrich either a running row or an end that arrived first. Charge only new detail;
/// replaying the hint must not consume another share of the run budget or change its end.
async fn begin_call(
    txn: &DatabaseTransaction,
    run: &mut trace_runs::Model,
    event: &RunEvent,
    mut row: trace_calls::Model,
    stub: bool,
) -> Result<(), DbErr> {
    let RunEvent::BeforeCall {
        parent,
        object,
        page,
        method,
        title,
        selector,
        location,
        started_at_ms,
        offset_us,
        ..
    } = event
    else {
        return Ok(());
    };
    row.parent_no = parent.map(signed);
    row.object.clone_from(object);
    row.method.clone_from(method);
    row.page_id = page
        .as_ref()
        .map(|page| page.target.as_str().to_string())
        .or(row.page_id);
    row.tab_id = page.as_ref().and_then(|page| page.tab_id).or(row.tab_id);
    row.started_at_ms = signed(*started_at_ms);
    row.offset_us = signed(*offset_us);
    row.loc_line = location.map(|location| i64::from(location.line));
    row.loc_column = location.and_then(|location| location.column.map(i64::from));
    let selector: Option<String> = selector
        .as_ref()
        .map(|text| text.chars().take(256).collect());
    let bytes = title
        .as_ref()
        .map_or(0, String::len)
        .saturating_sub(row.title.as_ref().map_or(0, String::len))
        + selector
            .as_ref()
            .map_or(0, String::len)
            .saturating_sub(row.selector.as_ref().map_or(0, String::len));
    if detail(run, bytes, stub) {
        row.title.clone_from(title);
        row.selector = selector;
    } else {
        row.detail = Detail::Stub;
    }
    insert(txn, row, None).await
}

async fn finish_call(
    txn: &DatabaseTransaction,
    run: &mut trace_runs::Model,
    event: &RunEvent,
    mut row: trace_calls::Model,
    stub: bool,
) -> Result<(), DbErr> {
    let RunEvent::AfterCall {
        parent,
        page,
        record,
        params_truncated,
        offset_us,
        ..
    } = event
    else {
        return Ok(());
    };
    if row.state != CallState::Running {
        return Ok(());
    }
    let params = serde_json::to_string(&record.params).map_err(|error| json_error(&error))?;
    let logs = serde_json::to_string(&record.log).map_err(|error| json_error(&error))?;
    let message_bytes = match &record.result {
        CallResult::Error { message, .. } => message.len(),
        _ => 0,
    };
    let full = detail(run, params.len() + logs.len() + message_bytes, stub);
    let had_start = !row.object.is_empty();
    row.parent_no = parent.map(signed);
    row.object.clone_from(&record.guid);
    row.method.clone_from(&record.method);
    row.title = row.title.or_else(|| record.title.clone());
    row.page_id = page
        .as_ref()
        .map(|page| page.target.as_str().to_string())
        .or(row.page_id);
    row.tab_id = page.as_ref().and_then(|page| page.tab_id).or(row.tab_id);
    row.started_at_ms = signed(record.started_at_ms);
    if !had_start {
        row.offset_us = signed(*offset_us);
    }
    row.duration_us = Some(signed(
        u64::try_from(record.duration.as_micros()).unwrap_or(u64::MAX),
    ));
    row.timeout_ms = record.timeout_ms;
    row.loc_line = record.location.map(|location| i64::from(location.line));
    row.loc_column = record
        .location
        .and_then(|location| location.column.map(i64::from));
    row.params_json = full.then_some(params);
    row.log_json = full.then_some(logs);
    row.params_truncated = *params_truncated || !full;
    row.detail = if full { Detail::Full } else { Detail::Stub };
    match &record.result {
        CallResult::Ok => row.state = CallState::Ok,
        CallResult::Error { name, message } => {
            row.state = CallState::Error;
            row.error_name = Some(name.clone());
            row.error_message = full.then(|| message.clone());
        }
        CallResult::Unfinished { reason } => {
            row.state = CallState::Unfinished;
            row.unfinished_reason = Some((*reason).into());
        }
    }
    // DeliverySettled may have arrived first. Exclude delivery from the end upsert and
    // set it separately with its own SQL guard even when replaying the same AfterCall.
    let delivery = record.delivery.map(Into::into);
    insert(txn, row.clone(), Some(CallState::Running)).await?;
    delivery_update(txn, row, delivery, false).await?;
    Ok(())
}

/// End columns have one gate in SQLite, independently of the in-memory order check. The
/// delivery columns are deliberately absent from this guarded end update.
async fn insert(
    txn: &DatabaseTransaction,
    row: trace_calls::Model,
    guard: Option<CallState>,
) -> Result<(), DbErr> {
    use sea_orm::Iterable;
    let mut conflict =
        OnConflict::columns([trace_calls::Column::RunId, trace_calls::Column::CallNo]);
    conflict.update_columns(trace_calls::Column::iter().filter(|column| {
        !matches!(
            column,
            trace_calls::Column::RunId
                | trace_calls::Column::CallNo
                | trace_calls::Column::Delivery
                | trace_calls::Column::DeliverySettled
        )
    }));
    if let Some(state) = guard {
        conflict.action_and_where(
            Expr::col((trace_calls::Entity, trace_calls::Column::State)).eq(state),
        );
    }
    trace_calls::Entity::insert(row.into_active_model())
        .on_conflict(conflict)
        .exec_without_returning(txn)
        .await?;
    Ok(())
}

async fn delivery_update(
    txn: &DatabaseTransaction,
    mut row: trace_calls::Model,
    delivery: Option<Delivery>,
    settled: bool,
) -> Result<(), DbErr> {
    row.delivery = delivery;
    row.delivery_settled = settled;
    trace_calls::Entity::insert(row.into_active_model())
        .on_conflict(
            OnConflict::columns([trace_calls::Column::RunId, trace_calls::Column::CallNo])
                .update_columns([
                    trace_calls::Column::Delivery,
                    trace_calls::Column::DeliverySettled,
                ])
                .action_and_where(
                    Expr::col((trace_calls::Entity, trace_calls::Column::DeliverySettled))
                        .eq(false),
                )
                .to_owned(),
        )
        .exec_without_returning(txn)
        .await?;
    Ok(())
}

fn empty_call(run: &trace_runs::Model, no: u64) -> trace_calls::Model {
    trace_calls::Model {
        run_id: run.run_id.clone(),
        call_no: signed(no),
        parent_no: None,
        object: String::new(),
        page_id: None,
        tab_id: None,
        method: String::new(),
        title: None,
        selector: None,
        params_json: None,
        params_truncated: false,
        timeout_ms: None,
        loc_line: None,
        loc_column: None,
        started_at_ms: run.started_at_ms,
        offset_us: 0,
        duration_us: None,
        input_offset_us: None,
        state: CallState::Running,
        error_name: None,
        error_message: None,
        unfinished_reason: None,
        delivery: None,
        delivery_settled: false,
        log_json: None,
        detail: Detail::Full,
    }
}

pub(crate) async fn getter(
    txn: &DatabaseTransaction,
    run: &mut trace_runs::Model,
    event: &RunEvent,
    stub: bool,
) -> Result<(), DbErr> {
    let RunEvent::Getter {
        no,
        object,
        field,
        page,
        result,
        offset_us,
        duration_us,
        ..
    } = event
    else {
        return Ok(());
    };
    if *no > GETTER_CAP {
        run.getters_over_cap = run.getters_over_cap.max(signed(*no - GETTER_CAP));
        return Ok(());
    }
    if trace_getters::Entity::find_by_id((run.run_id.clone(), signed(*no)))
        .one(txn)
        .await?
        .is_some()
    {
        return Ok(());
    }
    let (outcome, summary, name, message) = match result {
        GetterResult::Ok { summary } => (
            GetterOutcome::Ok,
            Some(serde_json::to_string(summary).map_err(|error| json_error(&error))?),
            None,
            None,
        ),
        GetterResult::Error { name, message } => (
            GetterOutcome::Error,
            None,
            Some(name.clone()),
            Some(message.clone()),
        ),
    };
    let full = detail(
        run,
        summary.as_ref().map_or(0, String::len) + message.as_ref().map_or(0, String::len),
        stub,
    );
    let row = trace_getters::Model {
        run_id: run.run_id.clone(),
        getter_no: signed(*no),
        object: object.clone(),
        field: serde_json::from_value(
            serde_json::to_value(field).map_err(|error| json_error(&error))?,
        )
        .map_err(|error| json_error(&error))?,
        page_id: page.as_ref().map(|page| page.target.as_str().to_string()),
        tab_id: page.as_ref().and_then(|page| page.tab_id),
        outcome,
        summary_json: summary.filter(|_| full),
        error_name: name,
        error_message: message.filter(|_| full),
        offset_us: signed(*offset_us),
        duration_us: signed(*duration_us),
        detail: if full { Detail::Full } else { Detail::Stub },
    };
    trace_getters::Entity::insert(row.into_active_model())
        .on_conflict(
            OnConflict::columns([
                trace_getters::Column::RunId,
                trace_getters::Column::GetterNo,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(txn)
        .await?;
    Ok(())
}

fn json_error(error: &serde_json::Error) -> DbErr {
    DbErr::Custom(error.to_string())
}
