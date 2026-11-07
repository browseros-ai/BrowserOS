//! Neo: The writer's incremental cleanup cursor. Every turn deletes at most 10,000 child rows,
//! children before parents, using key-range subqueries instead of a parameter per run. Between
//! turns the writer can commit incoming events, even when one old run owns 50,000 calls.

use neo_runtime::RunId;
use sea_orm::sea_query::{
    Alias, DeleteStatement, Expr, Query, SelectStatement, SimpleExpr, SubQueryStatement,
};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend,
    DbErr, EntityTrait, QueryFilter, Statement, TransactionTrait,
};

use crate::Retention;
use crate::db::entities::{trace_runs, trace_sessions};
use crate::db::unsigned;

/// Neo: bounds child deletions per transaction so cleanup cannot monopolize the writer.
pub(crate) const CHUNK: u64 = 10_000;
/// Platform: a duration of one day is 86400000 milliseconds (`std::time::Duration::from_hours(24)`).
const DAY_MS: u64 = 86_400_000;
/// Neo: child tables and their secondary keys are deleted before their owning run rows.
const CHILDREN: [(&str, &str); 6] = [
    ("trace_calls", "call_no"),
    ("trace_getters", "getter_no"),
    ("trace_pages", "page_id"),
    ("trace_logs", "line_no"),
    ("trace_run_ids", "key"),
    ("trace_call_artifacts", "call_no"),
];

/// One sweep invocation advances by one bounded transaction. `true` requests another turn.
/// Age selection has two bound values regardless of backlog. Size selection uses the oldest
/// settled run as an upper range bound, then uses the same deletion machinery.
pub(crate) async fn step(
    db: &DatabaseConnection,
    policy: Retention,
    now: u64,
) -> Result<bool, DbErr> {
    let age = u64::try_from(policy.max_age.as_millis()).unwrap_or(u64::MAX);
    let cutoff = now.saturating_sub(age);
    let eligible = age_range(cutoff);
    if exists(db, eligible.clone()).await? {
        return prune(db, eligible).await;
    }
    if used_bytes(db).await? > policy.max_bytes {
        use sea_orm::QueryOrder;
        if let Some(oldest) = trace_runs::Entity::find()
            .filter(trace_runs::Column::SettledAtMs.is_not_null())
            .order_by_asc(trace_runs::Column::RunId)
            .one(db)
            .await?
        {
            return prune(
                db,
                Query::select()
                    .column(trace_runs::Column::RunId)
                    .from(trace_runs::Entity)
                    .and_where(trace_runs::Column::RunId.lte(oldest.run_id))
                    .and_where(trace_runs::Column::SettledAtMs.is_not_null())
                    .to_owned(),
            )
            .await;
        }
    }
    db.execute_unprepared("PRAGMA incremental_vacuum(2000)")
        .await?;
    Ok(false)
}

fn age_range(cutoff: u64) -> SelectStatement {
    Query::select()
        .column(trace_runs::Column::RunId)
        .from(trace_runs::Entity)
        .and_where(trace_runs::Column::RunId.lt(RunId::floor(cutoff).to_string()))
        .cond_where(
            Condition::any()
                .add(trace_runs::Column::SettledAtMs.is_not_null())
                .add(
                    Expr::col(trace_runs::Column::RunId).lt(SimpleExpr::Constant(
                        RunId::floor(cutoff.saturating_sub(DAY_MS))
                            .to_string()
                            .into(),
                    )),
                ),
        )
        .to_owned()
}

async fn exists(db: &DatabaseConnection, mut select: SelectStatement) -> Result<bool, DbErr> {
    select.limit(1);
    Ok(db
        .query_one(DbBackend::Sqlite.build(&select))
        .await?
        .is_some())
}

async fn prune(db: &DatabaseConnection, eligible: SelectStatement) -> Result<bool, DbErr> {
    let txn = db.begin().await?;
    delete_children(&txn, &eligible).await?;
    // A parent may be deleted only when every child table is empty for that key. This keeps
    // the database consistent and readable between cleanup turns; ON DELETE CASCADE would
    // defeat the row bound for a large run.
    let mut parents = eligible;
    for (table, _) in CHILDREN {
        parents.and_where(
            Expr::col(trace_runs::Column::RunId).not_in_subquery(
                Query::select()
                    .column(Alias::new("run_id"))
                    .from(Alias::new(table))
                    .to_owned(),
            ),
        );
    }
    parents.limit(CHUNK);
    trace_runs::Entity::delete_many()
        .filter(trace_runs::Column::RunId.in_subquery(parents))
        .exec(&txn)
        .await?;
    repair_sessions(&txn).await?;
    txn.commit().await?;
    Ok(true)
}

async fn delete_children(
    txn: &DatabaseTransaction,
    eligible: &SelectStatement,
) -> Result<u64, DbErr> {
    let mut left = CHUNK;
    for (table, key) in CHILDREN {
        if left == 0 {
            break;
        }
        let delete = child_delete(table, key, eligible.clone(), left);
        left -= txn
            .execute(DbBackend::Sqlite.build(&delete))
            .await?
            .rows_affected();
    }
    Ok(CHUNK - left)
}

/// The cutoff and LIMIT are the two bound values. The ancient threshold is an internally
/// generated ULID constant, rendered by `SeaQuery`; no input or backlog id list enters SQL text.
fn child_delete(table: &str, key: &str, eligible: SelectStatement, limit: u64) -> DeleteStatement {
    let run = Alias::new("run_id");
    let key = Alias::new(key);
    let selected = Query::select()
        .columns([run.clone(), key.clone()])
        .from(Alias::new(table))
        .and_where(Expr::col(run.clone()).in_subquery(eligible))
        .limit(limit)
        .to_owned();
    Query::delete()
        .from_table(Alias::new(table))
        .and_where(
            Expr::tuple([Expr::col(run).into(), Expr::col(key).into()]).in_subquery(selected),
        )
        .to_owned()
}

/// Retention's projection repair is set-based, with no list of ids bound into SQL. This work
/// occurs only during cleanup; browser events never rebuild session projections.
async fn repair_sessions(txn: &DatabaseTransaction) -> Result<(), DbErr> {
    let session = Expr::col((trace_sessions::Entity, trace_sessions::Column::SessionKey));
    let related = || {
        Query::select()
            .from(trace_runs::Entity)
            .and_where(
                Expr::col((trace_runs::Entity, trace_runs::Column::SessionKey)).eq(session.clone()),
            )
            .to_owned()
    };
    let mut count = related();
    count.expr(Expr::col(trace_runs::Column::RunId).count());
    let mut first = related();
    first.expr(Expr::col(trace_runs::Column::RunId).min());
    let mut last = related();
    last.expr(Expr::col(trace_runs::Column::RunId).max());
    // Delete only sessions that previously owned runs. A pre-run privacy tombstone has no
    // first_run_id and must survive cleanup until the labelled run actually arrives.
    trace_sessions::Entity::delete_many()
        .filter(trace_sessions::Column::FirstRunId.is_not_null())
        .filter(
            trace_sessions::Column::SessionKey.not_in_subquery(
                Query::select()
                    .column(trace_runs::Column::SessionKey)
                    .from(trace_runs::Entity)
                    .to_owned(),
            ),
        )
        .exec(txn)
        .await?;
    trace_sessions::Entity::update_many()
        .col_expr(
            trace_sessions::Column::Runs,
            SimpleExpr::SubQuery(None, Box::new(SubQueryStatement::SelectStatement(count))),
        )
        .col_expr(
            trace_sessions::Column::FirstRunId,
            SimpleExpr::SubQuery(None, Box::new(SubQueryStatement::SelectStatement(first))),
        )
        .col_expr(
            trace_sessions::Column::LastRunId,
            SimpleExpr::SubQuery(None, Box::new(SubQueryStatement::SelectStatement(last))),
        )
        .exec(txn)
        .await?;
    Ok(())
}

/// Explicit erasure uses the same child bound. The privacy tombstone is retained until every
/// run is removed so an asynchronous capture queued before the fence cannot resurrect a JPEG.
pub(crate) async fn forget_chunk(
    txn: &DatabaseTransaction,
    key: &str,
) -> Result<(u64, bool), DbErr> {
    let selected = Query::select()
        .column(trace_runs::Column::RunId)
        .from(trace_runs::Entity)
        .and_where(trace_runs::Column::SessionKey.eq(key))
        .to_owned();
    if delete_children(txn, &selected).await? == CHUNK {
        return Ok((0, true));
    }
    let count = trace_runs::Entity::delete_many()
        .filter(trace_runs::Column::RunId.in_subquery(selected))
        .exec(txn)
        .await?
        .rows_affected;
    trace_sessions::Entity::delete_by_id(key).exec(txn).await?;
    Ok((count, false))
}

async fn used_bytes(db: &DatabaseConnection) -> Result<u64, DbErr> {
    let row = db.query_one(Statement::from_string(DbBackend::Sqlite,
        "SELECT (page_count - freelist_count) * page_size AS used FROM pragma_page_count(), pragma_freelist_count(), pragma_page_size()"))
        .await?.ok_or_else(|| DbErr::Custom("SQLite returned no page usage".into()))?;
    Ok(unsigned(row.try_get("", "used")?))
}

#[cfg(test)]
mod tests {
    use super::{CHILDREN, CHUNK, age_range, child_delete};
    use sea_orm::DbBackend;

    #[test]
    fn backlog_selection_has_two_parameters_for_every_child_table() {
        for (table, key) in CHILDREN {
            let statement =
                DbBackend::Sqlite.build(&child_delete(table, key, age_range(100_000_000), CHUNK));
            assert_eq!(
                statement.values.unwrap().0.len(),
                2,
                "cutoff and ancient cutoff only for {table}"
            );
        }
    }
}
