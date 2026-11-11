//! Test-only fault injection and query-plan inspection. Keeping SQL here lets writer tests
//! verify rollback without granting the writer or reader modules a raw storage escape hatch.

use sea_orm::sea_query::Query;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, DbErr, EntityTrait,
    PaginatorTrait, QueryFilter, Statement,
};

use super::entities::{run_tabs, runs};

pub(crate) async fn break_table(db: &DatabaseConnection, table: &str) -> Result<(), DbErr> {
    assert!(matches!(table, "runs" | "operations" | "run_tabs"));
    db.execute_unprepared(&format!("DROP TABLE {table}"))
        .await?;
    Ok(())
}

/// A reversible failure that leaves the schema readable while a writer batch rolls back.
pub(crate) async fn reject_test_operation(
    db: &DatabaseConnection,
    reject: bool,
) -> Result<(), DbErr> {
    let sql = if reject {
        "CREATE TRIGGER reject_test_operation BEFORE INSERT ON operations          WHEN NEW.name = 'test.fail' BEGIN SELECT RAISE(ABORT, 'test batch failure'); END"
    } else {
        "DROP TRIGGER reject_test_operation"
    };
    db.execute_unprepared(sql).await?;
    Ok(())
}

pub(crate) async fn tab_link_counts(db: &DatabaseConnection) -> Result<(u64, u64), DbErr> {
    let orphans = run_tabs::Entity::find()
        .filter(
            run_tabs::Column::RunId.not_in_subquery(
                Query::select()
                    .column(runs::Column::Id)
                    .from(runs::Entity)
                    .to_owned(),
            ),
        )
        .count(db)
        .await?;
    Ok((orphans, run_tabs::Entity::find().count(db).await?))
}

pub(crate) async fn query_plan(
    db: &DatabaseConnection,
    statement: Statement,
) -> Result<String, DbErr> {
    let statement = Statement::from_sql_and_values(
        DbBackend::Sqlite,
        format!("EXPLAIN QUERY PLAN {}", statement.sql),
        statement.values.map(|values| values.0).unwrap_or_default(),
    );
    let rows = db.query_all(statement).await?;
    let details = rows
        .iter()
        .map(|row| row.try_get::<String>("", "detail"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(details.join("\n"))
}
