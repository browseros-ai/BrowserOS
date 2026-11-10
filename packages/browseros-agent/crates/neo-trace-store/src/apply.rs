//! Neo: Transaction-local projection of typed events onto SeaORM entities. This layer has no queue,
//! clock, live notifications, or retry state: applying an input twice has the same durable effect.
//! The writer supplies the receipt time and publishes heads only after this transaction commits.

use neo_runtime::{RunEvent, RunId, RunStarted};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ColumnTrait, DatabaseTransaction, DbErr, EntityTrait, IntoActiveModel, Iterable, QueryFilter,
};
use ulid::Ulid;

use crate::db::entities::{trace_run_ids, trace_runs, trace_sessions};
use crate::db::{signed, unsigned};
use crate::ingest::{self, Command};

mod artifacts;
mod calls;
mod lifecycle;

pub(crate) use artifacts::{forget_chunk, withhold};
pub(crate) use lifecycle::{SETTLE_GRACE_MS, reconcile, sweep};

/// Neo: bounds rows per run; overflow remains visible in run counts and worst delivery.
pub(crate) const CALL_CAP: u64 = 50_000;
/// Neo: bounds synchronous getter rows independently of browser call numbering.
pub(crate) const GETTER_CAP: u64 = 10_000;
/// Neo: bounds per-run detail bytes while retaining later operations as counted stubs.
pub(crate) const DETAIL_CAP: usize = 16 << 20;

/// Applies one accepted input. A returned session key is dirtied only in the writer's local
/// batch; rolling back discards that set along with the rows.
pub(crate) async fn command(
    txn: &DatabaseTransaction,
    command: &Command,
    now: u64,
) -> Result<Option<String>, DbErr> {
    let Some(id) = ingest::run_id(command) else {
        return Ok(None);
    };
    // The writer filters forgotten unsettled runs before projection. Once settlement clears
    // that tombstone, late artifacts still only decorate a run that already exists.
    if matches!(command, Command::Artifact(..))
        && trace_runs::Entity::find_by_id(id.to_string())
            .one(txn)
            .await?
            .is_none()
    {
        return Ok(None);
    }
    let (mut run, inserted) = ensure_run(txn, id).await?;
    let old_session = run.session_key.clone();
    match command {
        Command::Started(started) => start(txn, &mut run, started).await?,
        Command::Event { event, stub, loss } => match event {
            RunEvent::BeforeCall { .. }
            | RunEvent::AfterCall { .. }
            | RunEvent::InputSent { .. }
            | RunEvent::DeliverySettled { .. } => calls::call(txn, &mut run, event, *stub).await?,
            RunEvent::Getter { .. } => calls::getter(txn, &mut run, event, *stub).await?,
            RunEvent::PageOpened { .. } | RunEvent::PageClosed { .. } => {
                lifecycle::page(txn, event).await?;
            }
            RunEvent::RunEnded { .. } => lifecycle::end(txn, &mut run, event, *stub).await?,
            RunEvent::RunSettled { .. } => lifecycle::settle(txn, &mut run, *loss, now).await?,
        },
        Command::Artifact(call, after) => artifacts::attach(txn, &mut run, *call, after).await?,
        _ => {}
    }
    save_run(txn, run.clone()).await?;
    // A late start moves a shed-start placeholder out of its fallback one-run session.
    // Ordinary start/end projection work is one upsert, never a rebuild over browser calls.
    if old_session != run.session_key && !inserted {
        remove_session_run(txn, &old_session).await?;
    }
    if inserted
        || old_session != run.session_key
        || matches!(
            command,
            Command::Started(_)
                | Command::Event {
                    event: RunEvent::RunEnded { .. },
                    ..
                }
        )
    {
        session_run(
            txn,
            &run,
            i64::from(inserted || old_session != run.session_key),
        )
        .await?;
    }
    Ok(Some(run.session_key))
}

async fn ensure_run(
    txn: &DatabaseTransaction,
    id: RunId,
) -> Result<(trace_runs::Model, bool), DbErr> {
    let key = id.to_string();
    if let Some(row) = trace_runs::Entity::find_by_id(&key).one(txn).await? {
        return Ok((row, false));
    }
    let started = Ulid::from_string(&key)
        .map_err(|error| DbErr::Custom(error.to_string()))?
        .timestamp_ms();
    let row = trace_runs::Model {
        run_id: key.clone(),
        session_key: key,
        client: None,
        start_page: None,
        timeout_ms: None,
        code_bytes: None,
        script: None,
        script_truncated: false,
        started_at_ms: signed(started),
        status: None,
        end_cause: None,
        error_name: None,
        error_message: None,
        value_json: None,
        value_truncated: false,
        ended_at_ms: None,
        duration_us: None,
        calls_made: 0,
        calls_over_cap: 0,
        over_cap_delivery: None,
        unreadable_calls: 0,
        getters_over_cap: 0,
        settled_at_ms: None,
        settled_by: None,
        detail_bytes: 0,
        events_stubbed: 0,
        events_dropped: 0,
        incomplete: false,
    };
    save_run(txn, row.clone()).await?;
    Ok((row, true))
}

pub(crate) async fn save_run(
    txn: &DatabaseTransaction,
    row: trace_runs::Model,
) -> Result<(), DbErr> {
    trace_runs::Entity::insert(row.into_active_model())
        .on_conflict(
            OnConflict::column(trace_runs::Column::RunId)
                .update_columns(
                    trace_runs::Column::iter()
                        .filter(|column| !matches!(column, trace_runs::Column::RunId)),
                )
                .to_owned(),
        )
        .exec_without_returning(txn)
        .await?;
    Ok(())
}

async fn start(
    txn: &DatabaseTransaction,
    run: &mut trace_runs::Model,
    started: &RunStarted,
) -> Result<(), DbErr> {
    if run.start_page.is_some() {
        return Ok(());
    }
    run.session_key = started.run.session.as_ref().map_or_else(
        || run.run_id.clone(),
        |session| session.as_str().to_string(),
    );
    run.client.clone_from(&started.client);
    run.start_page = Some(started.page.as_str().to_string());
    run.timeout_ms = Some(signed(started.timeout_ms));
    run.code_bytes = Some(signed(started.code_bytes));
    run.started_at_ms = signed(started.started_at_ms);
    for (key, value) in &started.ids {
        trace_run_ids::Entity::insert(trace_run_ids::ActiveModel {
            run_id: Set(run.run_id.clone()),
            key: Set(key.as_str().to_string()),
            value: Set(value.clone()),
        })
        .on_conflict(
            OnConflict::columns([trace_run_ids::Column::RunId, trace_run_ids::Column::Key])
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(txn)
        .await?;
    }
    Ok(())
}

/// Counts change only when a run is first inserted or moved from its fallback session.
/// Replayed starts and ends still update metadata, with delta zero.
async fn session_run(
    txn: &DatabaseTransaction,
    run: &trace_runs::Model,
    delta: i64,
) -> Result<(), DbErr> {
    let previous = trace_sessions::Entity::find_by_id(&run.session_key)
        .one(txn)
        .await?;
    let activity = run.ended_at_ms.unwrap_or(run.started_at_ms);
    let row = trace_sessions::Model {
        session_key: run.session_key.clone(),
        client: previous
            .as_ref()
            .and_then(|row| row.client.clone())
            .or_else(|| run.client.clone()),
        first_run_id: Some(
            previous
                .as_ref()
                .and_then(|row| row.first_run_id.clone())
                .map_or_else(|| run.run_id.clone(), |first| first.min(run.run_id.clone())),
        ),
        last_run_id: Some(
            previous
                .as_ref()
                .and_then(|row| row.last_run_id.clone())
                .map_or_else(|| run.run_id.clone(), |last| last.max(run.run_id.clone())),
        ),
        runs: previous
            .as_ref()
            .map_or(0, |row| row.runs)
            .saturating_add(delta),
        last_activity_ms: previous
            .as_ref()
            .map_or(activity, |row| row.last_activity_ms.max(activity)),
        withheld: previous.is_some_and(|row| row.withheld),
    };
    trace_sessions::Entity::insert(row.into_active_model())
        .on_conflict(
            OnConflict::column(trace_sessions::Column::SessionKey)
                .update_columns([
                    trace_sessions::Column::Client,
                    trace_sessions::Column::FirstRunId,
                    trace_sessions::Column::LastRunId,
                    trace_sessions::Column::Runs,
                    trace_sessions::Column::LastActivityMs,
                ])
                .to_owned(),
        )
        .exec_without_returning(txn)
        .await?;
    Ok(())
}

async fn remove_session_run(txn: &DatabaseTransaction, key: &str) -> Result<(), DbErr> {
    use sea_orm::{PaginatorTrait, QueryOrder};
    let select = trace_runs::Entity::find().filter(trace_runs::Column::SessionKey.eq(key));
    let count = select.clone().count(txn).await?;
    if count == 0 {
        trace_sessions::Entity::delete_by_id(key).exec(txn).await?;
        return Ok(());
    }
    let first = select
        .clone()
        .order_by_asc(trace_runs::Column::RunId)
        .one(txn)
        .await?;
    let last = select
        .order_by_desc(trace_runs::Column::RunId)
        .one(txn)
        .await?;
    trace_sessions::Entity::update_many()
        .col_expr(trace_sessions::Column::Runs, Expr::value(signed(count)))
        .col_expr(
            trace_sessions::Column::FirstRunId,
            Expr::value(first.map(|run| run.run_id)),
        )
        .col_expr(
            trace_sessions::Column::LastRunId,
            Expr::value(last.map(|run| run.run_id)),
        )
        .filter(trace_sessions::Column::SessionKey.eq(key))
        .exec(txn)
        .await?;
    Ok(())
}

/// Detail accounting is monotonic and transactional. Only the first accepted version of a row
/// claims space; duplicates never consume another share of the run's 16 MiB budget.
fn detail(run: &mut trace_runs::Model, bytes: usize, stub: bool) -> bool {
    let used = usize::try_from(run.detail_bytes).unwrap_or(DETAIL_CAP);
    if stub || used.saturating_add(bytes) > DETAIL_CAP {
        return false;
    }
    run.detail_bytes = signed(unsigned(run.detail_bytes).saturating_add(bytes as u64));
    true
}
