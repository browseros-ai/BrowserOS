//! Neo: Run/page lifecycle and crash reconciliation. Runtime settlement, timeout, and restart are
//! distinct durable facts; synthetic settlement marks incomplete capture instead of inventing
//! a successful end or acknowledging input whose delivery is unknown.

use neo_runtime::RunEvent;
use sea_orm::sea_query::{Expr, ExprTrait, OnConflict};
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, DatabaseTransaction, DbErr, EntityTrait,
    IntoActiveModel, QueryFilter, TransactionTrait,
};

use super::{detail, save_run};
use crate::db::entities::{trace_calls, trace_logs, trace_pages, trace_runs};
use crate::db::kinds::{CallState, RunStatus, SettledBy, UnfinishedReason};
use crate::db::signed;

/// Neo: lost runtime settlement is synthesized after the run's end or budget plus this grace.
pub(crate) const SETTLE_GRACE_MS: u64 = 120_000;

pub(super) async fn end(
    txn: &DatabaseTransaction,
    run: &mut trace_runs::Model,
    event: &RunEvent,
    stub: bool,
) -> Result<(), DbErr> {
    let RunEvent::RunEnded {
        status,
        cause,
        error,
        value,
        value_truncated,
        logs,
        script,
        script_truncated,
        calls_made,
        unreadable_calls,
        duration_us,
        ..
    } = event
    else {
        return Ok(());
    };
    if run.end_cause.is_some() {
        return Ok(());
    }
    let value = value
        .as_deref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| DbErr::Custom(error.to_string()))?;
    let bytes = value.as_ref().map_or(0, String::len)
        + script.as_ref().map_or(0, |script| script.len())
        + logs.iter().map(|line| line.text.len()).sum::<usize>()
        + error.as_ref().map_or(0, |error| error.message.len());
    let full = detail(run, bytes, stub);
    run.status = Some((*status).into());
    run.end_cause = Some((*cause).into());
    run.error_name = error.as_ref().map(|error| error.name.clone());
    run.error_message = error
        .as_ref()
        .filter(|_| full)
        .map(|error| error.message.clone());
    run.value_json = value.filter(|_| full);
    run.value_truncated = *value_truncated || !full;
    run.script = script.as_ref().filter(|_| full).map(ToString::to_string);
    run.script_truncated = *script_truncated || !full;
    run.calls_made = signed(*calls_made);
    run.calls_over_cap = run
        .calls_over_cap
        .max(signed(calls_made.saturating_sub(super::CALL_CAP)));
    run.unreadable_calls = signed(*unreadable_calls);
    run.duration_us = Some(signed(*duration_us));
    run.ended_at_ms = Some(
        run.started_at_ms
            .saturating_add(signed(*duration_us / 1000)),
    );
    if full {
        for (no, line) in logs.iter().enumerate() {
            trace_logs::Entity::insert(
                trace_logs::Model {
                    run_id: run.run_id.clone(),
                    line_no: signed(no as u64),
                    offset_us: signed(line.offset_us),
                    text: line.text.clone(),
                }
                .into_active_model(),
            )
            .on_conflict(
                OnConflict::columns([trace_logs::Column::RunId, trace_logs::Column::LineNo])
                    .do_nothing()
                    .to_owned(),
            )
            .exec_without_returning(txn)
            .await?;
        }
    }
    Ok(())
}

pub(super) async fn settle(
    txn: &DatabaseTransaction,
    run: &mut trace_runs::Model,
    loss: (u64, u64),
    now: u64,
) -> Result<(), DbErr> {
    // Settlement time is first-observed and therefore replay-stable. Absolute counts replace,
    // rather than increment, the per-run totals. Incomplete writer losses are never erased.
    if run.settled_by != Some(SettledBy::Runtime) {
        run.settled_at_ms = Some(signed(now));
        run.settled_by = Some(SettledBy::Runtime);
    }
    // Settlement without an end means the lifecycle event was shed. Do not leave a run
    // simultaneously settled and running in feeds; a later real end can still fill its facts.
    if run.status.is_none() {
        run.status = Some(RunStatus::Interrupted);
        run.ended_at_ms = Some(signed(now));
        run.incomplete = true;
    }
    run.events_stubbed = run.events_stubbed.max(signed(loss.0));
    run.events_dropped = run.events_dropped.max(signed(loss.1));
    lost_calls(txn, &run.run_id, false).await
}

async fn lost_calls(txn: &DatabaseTransaction, run: &str, restart: bool) -> Result<(), DbErr> {
    let mut update = trace_calls::Entity::update_many().col_expr(
        trace_calls::Column::State,
        Expr::value(if restart {
            CallState::Unfinished
        } else {
            CallState::Lost
        }),
    );
    if restart {
        update = update.col_expr(
            trace_calls::Column::UnfinishedReason,
            Expr::value(UnfinishedReason::Interrupted),
        );
    }
    update
        .filter(trace_calls::Column::RunId.eq(run))
        .filter(trace_calls::Column::State.eq(CallState::Running))
        .exec(txn)
        .await?;
    Ok(())
}

pub(super) async fn page(txn: &DatabaseTransaction, event: &RunEvent) -> Result<(), DbErr> {
    let (run, target) = match event {
        RunEvent::PageOpened { run, page, .. } => (*run, page.target.as_str()),
        RunEvent::PageClosed { run, page, .. } => (*run, page.as_str()),
        _ => return Ok(()),
    };
    let mut row = trace_pages::Entity::find_by_id((run.to_string(), target.to_string()))
        .one(txn)
        .await?
        .unwrap_or(trace_pages::Model {
            run_id: run.to_string(),
            page_id: target.to_string(),
            tab_id: None,
            created: false,
            by_call: None,
            opened_offset_us: None,
            closed_offset_us: None,
        });
    match event {
        RunEvent::PageOpened {
            page,
            created,
            by_call,
            offset_us,
            ..
        } => {
            row.tab_id = row.tab_id.or(page.tab_id);
            row.created |= *created;
            row.by_call = row.by_call.or(by_call.map(signed));
            row.opened_offset_us = row.opened_offset_us.or(Some(signed(*offset_us)));
        }
        RunEvent::PageClosed { offset_us, .. } => {
            row.closed_offset_us = row.closed_offset_us.or(Some(signed(*offset_us)));
        }
        _ => {}
    }
    trace_pages::Entity::insert(row.into_active_model())
        .on_conflict(
            OnConflict::columns([trace_pages::Column::RunId, trace_pages::Column::PageId])
                .update_columns([
                    trace_pages::Column::TabId,
                    trace_pages::Column::Created,
                    trace_pages::Column::ByCall,
                    trace_pages::Column::OpenedOffsetUs,
                    trace_pages::Column::ClosedOffsetUs,
                ])
                .to_owned(),
        )
        .exec_without_returning(txn)
        .await?;
    Ok(())
}

/// Boot runs under the file lock before ingest starts. A running call cannot be reported as
/// lost here: the old process died, so its known end is unfinished/interrupted.
pub(crate) async fn reconcile(db: &DatabaseConnection, now: u64) -> Result<(), DbErr> {
    synthetic(db, now, true).await.map(|_| ())
}

pub(crate) async fn sweep(db: &DatabaseConnection, now: u64) -> Result<Vec<String>, DbErr> {
    synthetic(db, now, false).await
}

async fn synthetic(db: &DatabaseConnection, now: u64, restart: bool) -> Result<Vec<String>, DbErr> {
    let mut select = trace_runs::Entity::find().filter(trace_runs::Column::SettledAtMs.is_null());
    if !restart {
        let deadline = signed(now.saturating_sub(SETTLE_GRACE_MS));
        // If both lifecycle events were shed, there is no ended_at_ms. A known runtime budget
        // still bounds the run: allow the same 120 s settlement grace after that budget. Do not
        // guess a deadline for a placeholder whose RunStarted (and timeout) never arrived.
        select = select.filter(
            Condition::any()
                .add(trace_runs::Column::EndedAtMs.lt(deadline))
                .add(
                    Condition::all()
                        .add(trace_runs::Column::EndedAtMs.is_null())
                        .add(trace_runs::Column::TimeoutMs.is_not_null())
                        .add(
                            Expr::col(trace_runs::Column::StartedAtMs)
                                .add(Expr::col(trace_runs::Column::TimeoutMs))
                                .lt(deadline),
                        ),
                ),
        );
    }
    let rows = select.all(db).await?;
    let txn = db.begin().await?;
    let mut sessions = Vec::new();
    for mut run in rows {
        if run.status.is_none() {
            run.status = Some(RunStatus::Interrupted);
            run.ended_at_ms = Some(signed(now));
        }
        run.settled_at_ms = Some(signed(now));
        run.settled_by = Some(if restart {
            SettledBy::Restart
        } else {
            SettledBy::Timeout
        });
        run.incomplete = true;
        lost_calls(&txn, &run.run_id, restart).await?;
        sessions.push(run.session_key.clone());
        save_run(&txn, run).await?;
    }
    txn.commit().await?;
    Ok(sessions)
}
