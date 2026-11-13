//! One transaction and the only row-write interface for the trace file. The ordered writer,
//! startup recovery (before readers exist) and salvage (before the writer starts) are its only
//! callers. Callers bound payloads before supplying them; this layer never transforms text.
//! Dropping a batch without commit rolls back; replies, events and camera jobs must wait for commit.

use browseros_core::TabId;
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::{Expr, Func, OnConflict, SimpleExpr};
use sea_orm::{
    ColumnTrait, DatabaseConnection, DatabaseTransaction, EntityTrait, FromQueryResult,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Select, TransactionTrait, UpdateMany,
};
use serde_json::Value;

use super::entities::{operations, run_tabs, runs};
use super::record_rows::{json_error, parse_id};
use crate::run_outputs::RunOutputs;
use crate::screenshot_files::ShotFiles;
use crate::{
    Delivery, Effect, EpochMillis, ErrorRecord, OperationId, OperationKind, OperationSite, Outcome,
    PageState, RequestId, RetentionPolicy, RunEffects, RunId, RunStatus, RunTabsScope, Shot,
    ShotState, Swept, TaskId, TraceError, trace_store,
};

/// A run's start row, containing only identity and state. Code and host data arrive with the
/// end row, preserving the reader-visible lifecycle ([`RunEndRow::host_data`]).
pub(crate) struct RunStartRow<'a> {
    pub(crate) id: RunId,
    pub(crate) task: Option<&'a TaskId>,
    pub(crate) at: EpochMillis,
    pub(crate) request_id: Option<&'a RequestId>,
    /// `None` for a run whose start the writer never saw.
    pub(crate) tabs_scope: Option<RunTabsScope>,
}

/// What a run's end stores of its code.
pub(crate) enum RunCode<'a> {
    /// The bounded code.
    Stored { text: &'a str, truncated: bool },
    /// The writer never received the run's code.
    Unavailable,
    /// Intentionally omitted by the host's trace.values policy.
    NotRecorded,
}

/// A run's end row, as the writer stores it.
pub(crate) struct RunEndRow<'a> {
    pub(crate) values: Option<&'a str>,
    pub(crate) id: RunId,
    pub(crate) at: EpochMillis,
    pub(crate) status: RunStatus,
    pub(crate) error: Option<&'a ErrorRecord>,
    pub(crate) result: Option<&'a Value>,
    pub(crate) result_truncated: bool,
    pub(crate) code: RunCode<'a>,
    /// The run's bounded host data; `None` keeps what the row has (nothing, for a
    /// run whose start the writer never saw).
    pub(crate) host_data: Option<&'a Value>,
    pub(crate) replay: Option<&'a Value>,
    pub(crate) replay_until: Option<EpochMillis>,
    pub(crate) gaps: u64,
    /// No screenshot job of the run is unanswered, so the end completes it.
    pub(crate) complete: bool,
}

/// An operation's start row, as the writer stores it. It has no input: the operation's end
/// stores that, bounded to its storage budget. An operation with a target tab also
/// lists its run under that tab.
pub(crate) struct OperationStartRow<'a> {
    pub(crate) site: Option<OperationSite>,
    pub(crate) id: OperationId,
    pub(crate) parent: Option<OperationId>,
    pub(crate) name: &'a str,
    pub(crate) kind: OperationKind,
    pub(crate) effect: Effect,
    pub(crate) tab: Option<TabId>,
    pub(crate) at: EpochMillis,
}

/// An operation's end row, as the writer stores it (payloads already bounded).
pub(crate) struct OperationEndRow<'a> {
    pub(crate) values: Option<&'a str>,
    pub(crate) id: OperationId,
    pub(crate) at: EpochMillis,
    pub(crate) outcome: Outcome,
    pub(crate) delivery: Delivery,
    pub(crate) sent_at: Option<EpochMillis>,
    pub(crate) args: Option<(&'a Value, bool)>,
    pub(crate) result: Option<(&'a Value, bool)>,
    pub(crate) error: Option<&'a ErrorRecord>,
    pub(crate) notes: &'a [String],
    pub(crate) page: Option<&'a PageState>,
    pub(crate) main_world: bool,
    pub(crate) frames_incomplete: bool,
}

/// The recoverable request-id portion of a damaged run; salvage has already validated it.
pub(crate) struct SalvagedRun {
    pub(crate) request: RequestId,
    pub(crate) task: Option<TaskId>,
    pub(crate) started_at: EpochMillis,
    pub(crate) ended_at: Option<EpochMillis>,
    pub(crate) status: RunStatus,
    pub(crate) effects: RunEffects,
    pub(crate) replay: Option<String>,
    pub(crate) replay_until: Option<EpochMillis>,
}

/// Owns a write batch, so a failed/cancelled caller cannot leave a partial update committed.
pub(crate) struct TraceBatch {
    txn: DatabaseTransaction,
}

impl TraceBatch {
    pub(crate) async fn begin(db: &DatabaseConnection) -> Result<Self, TraceError> {
        Ok(Self {
            txn: db.begin().await?,
        })
    }

    pub(crate) async fn commit(self) -> Result<(), TraceError> {
        self.txn.commit().await?;
        Ok(())
    }

    pub(crate) async fn recover_crash(&self) -> Result<(), TraceError> {
        operations::Entity::update_many()
            .col_expr(operations::Column::Outcome, Expr::value("cancelled"))
            .col_expr(operations::Column::Delivery, cancelled_delivery())
            .filter(operations::Column::Outcome.is_null())
            .exec(&self.txn)
            .await?;
        // Code only reaches disk at the end. An interrupted run has no completed code to show.
        runs::Entity::update_many()
            .col_expr(runs::Column::Status, Expr::value("interrupted"))
            .col_expr(runs::Column::Effects, Expr::value("unknown"))
            .col_expr(
                runs::Column::CodeWithheld,
                Expr::col(runs::Column::CodeWithheld).if_null("unavailable"),
            )
            .filter(runs::Column::EndedAt.is_null())
            .exec(&self.txn)
            .await?;
        runs::Entity::update_many()
            .col_expr(runs::Column::Complete, Expr::value(true))
            .filter(runs::Column::Complete.eq(false))
            .exec(&self.txn)
            .await?;
        Ok(())
    }

    pub(crate) async fn insert_run(&self, run: RunStartRow<'_>) -> Result<(), TraceError> {
        runs::Entity::insert(runs::ActiveModel {
            id: Set(run.id.to_string()),
            task_id: Set(run.task.map(ToString::to_string)),
            started_at: Set(run.at.millis()),
            status: Set("running".to_owned()),
            request_id: Set(run.request_id.map(ToString::to_string)),
            tabs_scope: Set(run.tabs_scope.map(|scope| scope.as_str().to_owned())),
            ..Default::default()
        })
        .on_conflict(OnConflict::column(runs::Column::Id).do_nothing().to_owned())
        .exec_without_returning(&self.txn)
        .await?;
        Ok(())
    }

    pub(crate) async fn run_task(&self, run: RunId) -> Result<Option<Option<TaskId>>, TraceError> {
        Ok(runs::Entity::find_by_id(run.to_string())
            .select_only()
            .column(runs::Column::TaskId)
            .into_tuple::<Option<String>>()
            .one(&self.txn)
            .await?
            .map(|task| task.and_then(|text| TaskId::parse(&text).ok())))
    }

    pub(crate) async fn end_run(&self, run: RunEndRow<'_>) -> Result<RunEffects, TraceError> {
        let id = run.id.to_string();
        operations::Entity::update_many()
            .col_expr(operations::Column::Outcome, Expr::value("cancelled"))
            .col_expr(operations::Column::EndedAt, Expr::value(run.at.millis()))
            .col_expr(operations::Column::Delivery, cancelled_delivery())
            .col_expr(operations::Column::Values, Expr::value(run.values))
            .filter(operations::Column::RunId.eq(&id))
            .filter(operations::Column::Outcome.is_null())
            .exec(&self.txn)
            .await?;
        let counts = operations::Entity::find()
            .select_only()
            .column(operations::Column::Delivery)
            .column_as(operations::Column::Seq.count(), "n")
            .filter(operations::Column::RunId.eq(&id))
            .filter(operations::Column::Effect.eq("write"))
            .group_by(operations::Column::Delivery)
            .into_tuple::<(Option<String>, i64)>()
            .all(&self.txn)
            .await?;
        let mut unknown = 0;
        let mut attempted = 0;
        for (delivery, count) in counts {
            match delivery.as_deref() {
                Some("unknown") => {
                    unknown += count;
                    attempted += count;
                }
                Some("done") => attempted += count,
                _ => {}
            }
        }
        let gaps = runs::Entity::find_by_id(&id)
            .select_only()
            .column(runs::Column::TraceGaps)
            .into_tuple::<i64>()
            .one(&self.txn)
            .await?
            .unwrap_or(0)
            .saturating_add(i64::try_from(run.gaps).unwrap_or(i64::MAX));
        // A missing event could have been a write. Empty delivery groups still mean zero.
        let effects = if unknown > 0 || gaps > 0 {
            RunEffects::Unknown
        } else if attempted > 0 {
            RunEffects::Done
        } else {
            RunEffects::None
        };
        let (code, code_truncated, code_withheld) = match run.code {
            RunCode::Stored { text, truncated } => (Some(text.to_owned()), truncated, None),
            RunCode::Unavailable => (None, false, Some("unavailable".to_owned())),
            RunCode::NotRecorded => (None, false, None),
        };
        let mut model = runs::ActiveModel {
            values: Set(run.values.map(str::to_owned)),
            ended_at: Set(Some(run.at.millis())),
            status: Set(run.status.as_str().to_owned()),
            effects: Set(Some(effects.as_str().to_owned())),
            error: Set(run
                .error
                .map(serde_json::to_string)
                .transpose()
                .map_err(json_error)?),
            result: Set(run.result.map(Value::to_string)),
            result_truncated: Set(run.result_truncated),
            trace_gaps: Set(gaps),
            replay: Set(run.replay.map(Value::to_string)),
            replay_until: Set(run.replay_until.map(EpochMillis::millis)),
            code: Set(code),
            code_truncated: Set(code_truncated),
            code_withheld: Set(code_withheld),
            complete: Set(run.complete),
            ..Default::default()
        };
        // COALESCE(input, stored) without reading a model or touching an absent host payload.
        if let Some(host_data) = run.host_data {
            model.host_data = Set(Some(host_data.to_string()));
        }
        runs::Entity::update_many()
            .set(model)
            .filter(runs::Column::Id.eq(id))
            .exec(&self.txn)
            .await?;
        Ok(effects)
    }

    pub(crate) async fn insert_operation(
        &self,
        op: OperationStartRow<'_>,
    ) -> Result<(), TraceError> {
        let inserted = operations::Entity::insert(operations::ActiveModel {
            run_id: Set(op.id.run.to_string()),
            seq: Set(i64::from(op.id.seq)),
            parent_id: Set(op.parent.map(|parent| parent.to_string())),
            name: Set(op.name.to_owned()),
            site: Set(op
                .site
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(json_error)?),
            kind: Set(op.kind.as_str().to_owned()),
            attempt: Set(op.kind.attempt().map(i64::from)),
            effect: Set(op.effect.as_str().to_owned()),
            target_tab_id: Set(op.tab.map(|tab| tab.0)),
            started_at: Set(op.at.millis()),
            ..Default::default()
        })
        .on_conflict(
            OnConflict::columns([operations::Column::RunId, operations::Column::Seq])
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(&self.txn)
        .await?;
        if inserted > 0 {
            runs::Entity::update_many()
                .col_expr(
                    runs::Column::OperationCount,
                    Expr::col(runs::Column::OperationCount).add(1),
                )
                .filter(runs::Column::Id.eq(op.id.run.to_string()))
                .exec(&self.txn)
                .await?;
        }
        // A subscription or one of Neo's own reads names a tab without acting on it, so it links
        // no tab to the run.
        if let Some(tab) = trace_store::touched_tab(op.name, op.site.is_some(), op.tab) {
            self.link_tab(op.id.run, tab).await?;
        }
        Ok(())
    }

    /// Makes `tab` one of `run`'s tabs (the run record's `tabs`).
    pub(crate) async fn link_tab(&self, run: RunId, tab: TabId) -> Result<(), TraceError> {
        run_tabs::Entity::insert(run_tabs::ActiveModel {
            tab_id: Set(tab.0),
            run_id: Set(run.to_string()),
        })
        .on_conflict(
            OnConflict::columns([run_tabs::Column::TabId, run_tabs::Column::RunId])
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(&self.txn)
        .await?;
        Ok(())
    }

    pub(crate) async fn end_operation(&self, op: OperationEndRow<'_>) -> Result<bool, TraceError> {
        let page = op.page.cloned().unwrap_or_default();
        // The legacy page_id column stays untouched for old files. New records leave it NULL;
        // only the native tab ID crosses the trace boundary.
        let mut model = operations::ActiveModel {
            values: Set(op.values.map(str::to_owned)),
            ended_at: Set(Some(op.at.millis())),
            outcome: Set(Some(op.outcome.as_str().to_owned())),
            delivery: Set(Some(op.delivery.as_str().to_owned())),
            sent_at: Set(op.sent_at.map(EpochMillis::millis)),
            result: Set(op.result.map(|(value, _)| value.to_string())),
            result_truncated: Set(op.result.is_some_and(|(_, cut)| cut)),
            error: Set(op
                .error
                .map(serde_json::to_string)
                .transpose()
                .map_err(json_error)?),
            notes: Set((!op.notes.is_empty())
                .then(|| serde_json::to_string(op.notes))
                .transpose()
                .map_err(json_error)?),
            tab_id: Set(page.tab_id.map(|tab| tab.0)),
            url: Set(page.url),
            title: Set(page.title),
            main_world: Set(Some(op.main_world)),
            frames_incomplete: Set(Some(op.frames_incomplete)),
            ..Default::default()
        };
        if let Some((args, cut)) = op.args {
            model.args = Set(Some(args.to_string()));
            model.args_truncated = Set(cut);
        }
        let updated = operations::Entity::update_many()
            .set(model)
            .filter(operations::Column::RunId.eq(op.id.run.to_string()))
            .filter(operations::Column::Seq.eq(i64::from(op.id.seq)))
            .filter(operations::Column::Outcome.is_null())
            .exec(&self.txn)
            .await?;
        Ok(updated.rows_affected > 0)
    }

    pub(crate) async fn store_screenshot(
        &self,
        op: OperationId,
        shot: Shot,
    ) -> Result<(), TraceError> {
        let (state, bytes) = match shot {
            Shot::Taken { bytes_len } => (
                ShotState::Stored,
                Some(i64::try_from(bytes_len).unwrap_or(i64::MAX)),
            ),
            Shot::Withheld => (ShotState::Withheld, None),
            Shot::Dropped => return Ok(()),
        };
        operations::Entity::update_many()
            .set(operations::ActiveModel {
                screenshot: Set(Some(state.as_str().to_owned())),
                screenshot_bytes: Set(bytes),
                ..Default::default()
            })
            .filter(operations::Column::RunId.eq(op.run.to_string()))
            .filter(operations::Column::Seq.eq(i64::from(op.seq)))
            .exec(&self.txn)
            .await?;
        Ok(())
    }

    pub(crate) async fn complete_run(&self, run: RunId) -> Result<bool, TraceError> {
        Ok(runs::Entity::update_many()
            .col_expr(runs::Column::Complete, Expr::value(true))
            .filter(runs::Column::Id.eq(run.to_string()))
            .filter(runs::Column::Complete.eq(false))
            .filter(runs::Column::Status.ne("running"))
            .exec(&self.txn)
            .await?
            .rows_affected
            > 0)
    }

    /// Delete files before rows. A failed file delete retains its rows and its cap accounting;
    /// rollback may leave missing files, which read as expired and converge on the next sweep.
    /// An aged run's outputs (its kept downloads) go with its screenshots; the cap below frees
    /// screenshots only.
    pub(crate) async fn sweep(
        &self,
        files: &ShotFiles,
        outputs: &RunOutputs,
        policy: RetentionPolicy,
        now: EpochMillis,
    ) -> Result<Swept, TraceError> {
        let max_age = i64::try_from(policy.max_age.as_millis()).unwrap_or(i64::MAX);
        let aged = runs::Entity::find()
            .select_only()
            .column(runs::Column::Id)
            .filter(runs::Column::StartedAt.lt(now.plus_millis(-max_age).millis()))
            .filter(runs::Column::Status.ne("running"))
            .filter(runs::Column::Complete.eq(true))
            .into_tuple::<String>()
            .all(&self.txn)
            .await?;
        let mut swept = Swept::default();
        for id in aged {
            if let Ok(run) = id.parse::<RunId>()
                && !(remove_run_files(files, run).await && remove_run_outputs(outputs, run).await)
            {
                continue;
            }
            swept.screenshots += operations::Entity::find()
                .filter(operations::Column::RunId.eq(&id))
                .filter(operations::stored_shot())
                .count(&self.txn)
                .await?;
            run_tabs::Entity::delete_many()
                .filter(run_tabs::Column::RunId.eq(&id))
                .exec(&self.txn)
                .await?;
            swept.operations += operations::Entity::delete_many()
                .filter(operations::Column::RunId.eq(&id))
                .exec(&self.txn)
                .await?
                .rows_affected;
            swept.runs += runs::Entity::delete_many()
                .filter(runs::Column::Id.eq(&id))
                .exec(&self.txn)
                .await?
                .rows_affected;
        }
        swept.screenshots += self.cap_screenshots(files, policy.screenshot_cap).await?;
        swept.replays = runs::Entity::update_many()
            .set(runs::ActiveModel {
                replay: Set(None),
                replay_until: Set(None),
                ..Default::default()
            })
            .filter(runs::Column::ReplayUntil.is_not_null())
            .filter(runs::Column::ReplayUntil.lte(now.millis()))
            .exec(&self.txn)
            .await?
            .rows_affected;
        Ok(swept)
    }

    async fn cap_screenshots(&self, files: &ShotFiles, cap: u64) -> Result<u64, TraceError> {
        let stored = cap_total_select()
            .into_tuple::<i64>()
            .one(&self.txn)
            .await?
            .unwrap_or(0);
        let mut over = stored.saturating_sub(i64::try_from(cap).unwrap_or(i64::MAX));
        if over <= 0 {
            return Ok(0);
        }
        let oldest = operations::Entity::find()
            .select_only()
            .column(operations::Column::RunId)
            .column_as(operations::Column::ScreenshotBytes.sum(), "bytes")
            .inner_join(runs::Entity)
            .filter(operations::stored_shot())
            .filter(runs::Column::Complete.eq(true))
            .group_by(operations::Column::RunId)
            .order_by_asc(runs::Column::StartedAt)
            .order_by_asc(operations::Column::RunId)
            .into_model::<RunBytes>()
            .all(&self.txn)
            .await?;
        let mut expired = 0;
        for row in oldest {
            if over <= 0 {
                break;
            }
            let run: RunId = parse_id(&row.run_id)?;
            if !remove_run_files(files, run).await {
                continue;
            }
            over = over.saturating_sub(row.bytes);
            expired += expire_shots_update(run)
                .exec(&self.txn)
                .await?
                .rows_affected;
        }
        Ok(expired)
    }

    pub(crate) async fn insert_salvaged_run(
        &self,
        id: RunId,
        run: &SalvagedRun,
    ) -> Result<(), TraceError> {
        // No code or outstanding camera job came across from the broken store.
        runs::Entity::insert(runs::ActiveModel {
            id: Set(id.to_string()),
            task_id: Set(run.task.as_ref().map(ToString::to_string)),
            started_at: Set(run.started_at.millis()),
            ended_at: Set(run.ended_at.map(EpochMillis::millis)),
            status: Set(run.status.as_str().to_owned()),
            effects: Set(Some(run.effects.as_str().to_owned())),
            request_id: Set(Some(run.request.to_string())),
            code_withheld: Set(Some("unavailable".to_owned())),
            replay: Set(run.replay.clone()),
            replay_until: Set(run.replay_until.map(EpochMillis::millis)),
            complete: Set(true),
            ..Default::default()
        })
        .on_conflict(OnConflict::column(runs::Column::Id).do_nothing().to_owned())
        .exec_without_returning(&self.txn)
        .await?;
        Ok(())
    }
}

fn cancelled_delivery() -> SimpleExpr {
    Expr::case(Expr::col(operations::Column::Effect).eq("write"), "unknown")
        .finally("not_sent")
        .into()
}

/// SUM of an empty set is NULL; keeping zero in SQL prevents every no-camera sweep failing.
fn cap_total_select() -> Select<operations::Entity> {
    let total: SimpleExpr = Func::coalesce([
        Expr::col((operations::Entity, operations::Column::ScreenshotBytes)).sum(),
        Expr::val(0).into(),
    ])
    .into();
    operations::Entity::find()
        .select_only()
        .column_as(total, "stored")
        .filter(operations::stored_shot())
}

fn expire_shots_update(run: RunId) -> UpdateMany<operations::Entity> {
    operations::Entity::update_many()
        .col_expr(operations::Column::Screenshot, Expr::value("expired"))
        .filter(operations::Column::RunId.eq(run.to_string()))
        .filter(operations::stored_shot())
}

#[derive(FromQueryResult)]
struct RunBytes {
    run_id: String,
    bytes: i64,
}

async fn remove_run_files(files: &ShotFiles, run: RunId) -> bool {
    match tokio::fs::remove_dir_all(files.run_dir(run)).await {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => {
            tracing::warn!(%error, %run, "a run's screenshots stay on disk until the next sweep");
            false
        }
    }
}

async fn remove_run_outputs(outputs: &RunOutputs, run: RunId) -> bool {
    match tokio::fs::remove_dir_all(outputs.run_dir(run)).await {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => {
            tracing::warn!(%error, %run, "a run's kept outputs stay on disk until the next sweep");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use sea_orm::{DbBackend, QueryTrait};

    use super::{cap_total_select, expire_shots_update};
    use crate::storage::test_faults;
    use crate::{RunId, TraceStore};

    #[tokio::test]
    async fn the_cap_uses_operations_stored_shots() -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let store = TraceStore::open(&dir.path().join("trace.db")).await?;
        for statement in [
            cap_total_select().build(DbBackend::Sqlite),
            expire_shots_update(RunId::new()).build(DbBackend::Sqlite),
        ] {
            let plan = test_faults::query_plan(store.db(), statement).await?;
            assert!(
                plan.contains("operations_stored_shots"),
                "the cap must use the partial stored-shot index: {plan}"
            );
        }
        Ok(())
    }
}
