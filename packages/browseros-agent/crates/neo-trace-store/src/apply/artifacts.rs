//! Feed-owned after-call captures share the run's lifetime. Privacy is checked inside the
//! writer's transaction, including for a capture that completes after withhold was committed.

use neo_runtime::CallId;
use sea_orm::sea_query::{Expr, OnConflict, Query};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseTransaction, DbErr, EntityTrait, IntoActiveModel,
    QueryFilter,
};

use crate::db::entities::{trace_call_artifacts, trace_runs, trace_sessions};
use crate::db::signed;
use crate::{AfterCall, retention};

pub(super) async fn attach(
    txn: &DatabaseTransaction,
    run: &mut trace_runs::Model,
    call: CallId,
    after: &AfterCall,
) -> Result<(), DbErr> {
    if call.no > super::CALL_CAP {
        return Ok(());
    }
    let withheld = trace_sessions::Entity::find_by_id(&run.session_key)
        .one(txn)
        .await?
        .is_some_and(|session| session.withheld);
    let row = trace_call_artifacts::Model {
        run_id: run.run_id.clone(),
        call_no: signed(call.no),
        tab_id: after.tab_id,
        url: after.url.clone(),
        title: after.title.clone(),
        jpeg: after
            .jpeg
            .as_ref()
            .filter(|jpeg| !withheld && jpeg.len() <= 64 << 10)
            .cloned(),
        captured_at_ms: signed(after.captured_at_ms),
    };
    trace_call_artifacts::Entity::insert(row.into_active_model())
        .on_conflict(
            OnConflict::columns([
                trace_call_artifacts::Column::RunId,
                trace_call_artifacts::Column::CallNo,
            ])
            .update_columns([
                trace_call_artifacts::Column::TabId,
                trace_call_artifacts::Column::Url,
                trace_call_artifacts::Column::Title,
                trace_call_artifacts::Column::Jpeg,
                trace_call_artifacts::Column::CapturedAtMs,
            ])
            .action_and_where(
                Expr::col((
                    trace_call_artifacts::Entity,
                    trace_call_artifacts::Column::CapturedAtMs,
                ))
                .lte(after.captured_at_ms),
            )
            .to_owned(),
        )
        .exec_without_returning(txn)
        .await?;
    Ok(())
}

pub(crate) async fn withhold(txn: &DatabaseTransaction, key: &str) -> Result<(), DbErr> {
    trace_sessions::Entity::insert(trace_sessions::ActiveModel {
        session_key: Set(key.to_string()),
        client: Set(None),
        first_run_id: Set(None),
        last_run_id: Set(None),
        runs: Set(0),
        last_activity_ms: Set(0),
        withheld: Set(true),
    })
    .on_conflict(
        OnConflict::column(trace_sessions::Column::SessionKey)
            .update_column(trace_sessions::Column::Withheld)
            .to_owned(),
    )
    .exec_without_returning(txn)
    .await?;
    trace_call_artifacts::Entity::update_many()
        .col_expr(
            trace_call_artifacts::Column::Jpeg,
            Expr::value(Option::<Vec<u8>>::None),
        )
        .filter(
            trace_call_artifacts::Column::RunId.in_subquery(
                Query::select()
                    .column(trace_runs::Column::RunId)
                    .from(trace_runs::Entity)
                    .and_where(trace_runs::Column::SessionKey.eq(key))
                    .to_owned(),
            ),
        )
        .exec(txn)
        .await?;
    Ok(())
}

pub(crate) async fn forget_chunk(
    txn: &DatabaseTransaction,
    key: &str,
) -> Result<(u64, bool), DbErr> {
    retention::forget_chunk(txn, key).await
}
