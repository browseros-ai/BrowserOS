//! Neo: The schema and its ledger: an empty file, every older file, a file claw-server also uses, and
//! the ways a ledger can disagree with this build.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use neo_runtime::{Delivery, GetterField, IdKey, RunId, SessionKey};
use sea_orm::{ColumnTrait, DbErr, EntityTrait, IdenStatic, Iterable};
use sea_orm_migration::prelude::{
    Alias, ColumnDef, DynIden, MigrationName, MigrationTrait, MigratorTrait, SchemaManager, Table,
    async_trait,
};

use super::support::{count, exec, inspect, ledger, objects, scratch, texts};
use crate::db::entities::{
    trace_call_artifacts, trace_calls, trace_getters, trace_logs, trace_pages, trace_run_ids,
    trace_runs, trace_sessions,
};
use crate::db::migration::{
    FIXTURES, LEDGER, MIGRATION_NAMES, Migrator, Refusal, Schema, run_pending,
};
use crate::{
    CallQuery, CallState, GetterOutcome, Loss, RunQuery, RunState, SettledBy, StoreConfig,
    StoreError, Timeline, TraceStore, Unfinished, db,
};

/// Neo: the stable run id used by frozen migration-prefix fixtures.
const FIXTURE_RUN: &str = "01JC8Z3K4R0000000000000001";

async fn open(path: &Path) -> TraceStore {
    {
        let mut config = StoreConfig::new(path);
        config.retention.max_age = Duration::MAX;
        TraceStore::open(config).await.unwrap()
    }
}

async fn close(store: TraceStore) {
    store.close(Duration::from_secs(5)).await;
}

/// The table `entity` names has exactly the columns the entity declares, with the same
/// nullability.
async fn assert_columns<E: EntityTrait>(conn: &sea_orm::DatabaseConnection, entity: E) {
    let table = entity.table_name();
    let declared: BTreeMap<String, bool> = E::Column::iter()
        .map(|column| (column.as_str().to_string(), column.def().is_null()))
        .collect();
    let names = texts(
        conn,
        &format!("SELECT name FROM pragma_table_info('{table}')"),
    )
    .await;
    let mut actual = BTreeMap::new();
    for name in names {
        let not_null = count(
            conn,
            &format!("SELECT \"notnull\" FROM pragma_table_info('{table}') WHERE name = '{name}'"),
        )
        .await;
        actual.insert(name, not_null == 0);
    }
    assert_eq!(actual, declared, "{table}'s columns are its entity's");
}

#[tokio::test]
async fn an_empty_file_gets_the_entities_tables_and_an_ordered_ledger_of_its_own() {
    let (_dir, path) = scratch();
    close(open(&path).await).await;
    let conn = inspect(&path).await;
    assert_columns(&conn, trace_sessions::Entity).await;
    assert_columns(&conn, trace_runs::Entity).await;
    assert_columns(&conn, trace_calls::Entity).await;
    assert_columns(&conn, trace_getters::Entity).await;
    assert_columns(&conn, trace_pages::Entity).await;
    assert_columns(&conn, trace_logs::Entity).await;
    assert_columns(&conn, trace_run_ids::Entity).await;
    assert_columns(&conn, trace_call_artifacts::Entity).await;
    let tables = objects(&conn, "table").await;
    assert_eq!(
        tables,
        BTreeSet::from(
            [
                LEDGER,
                "trace_call_artifacts",
                "trace_calls",
                "trace_getters",
                "trace_logs",
                "trace_pages",
                "trace_run_ids",
                "trace_runs",
                "trace_sessions",
            ]
            .map(String::from)
        ),
        "the store's tables and its own ledger, and no seaql_migrations"
    );
    assert_eq!(
        objects(&conn, "index").await,
        BTreeSet::from(
            [
                "ix_trace_calls_page",
                "ix_trace_calls_tab",
                "ix_trace_run_ids_value",
                "ix_trace_runs_session",
                "ix_trace_runs_unsettled",
                "ix_trace_sessions_activity",
            ]
            .map(String::from)
        ),
        "the design's indexes"
    );
    assert_eq!(
        ledger(&conn).await,
        MIGRATION_NAMES,
        "every migration, in order"
    );
    assert_eq!(
        count(&conn, "PRAGMA auto_vacuum").await,
        2,
        "created incremental, so retention can hand pages back"
    );
}

#[tokio::test]
async fn every_older_file_migrates_to_the_end_and_reads_back_through_the_api() {
    for (applied, fixture) in FIXTURES.iter().enumerate() {
        let (_dir, path) = scratch();
        let conn = db::connect_writer(&path).await.unwrap();
        if applied > 0 {
            run_pending::<Migrator>(&conn, Some(u32::try_from(applied).unwrap()))
                .await
                .unwrap();
            assert_eq!(
                ledger(&conn).await,
                MIGRATION_NAMES[..applied],
                "the file holds the first {applied} migrations"
            );
            exec(&conn, fixture).await;
        }
        conn.close().await.unwrap();

        let store = open(&path).await;
        assert!(!store.health().read_only, "prefix {applied} opens to write");
        let runs = store.runs(RunQuery::recent(10)).await.unwrap();
        if applied == 0 {
            assert!(runs.items.is_empty(), "an empty file has no runs");
        } else {
            assert_fixture_reads_back(&store, applied).await;
        }
        close(store).await;
        assert_eq!(
            ledger(&inspect(&path).await).await,
            MIGRATION_NAMES,
            "prefix {applied} ends with every migration applied"
        );
    }
}

/// `FIXTURES[applied]`, as a host reads it.
async fn assert_fixture_reads_back(store: &TraceStore, applied: usize) {
    let id: RunId = FIXTURE_RUN.parse().unwrap();
    let task = (IdKey::new("zuzu.task").unwrap(), "t_1".to_string());
    let runs = store.runs(RunQuery::recent(10)).await.unwrap();
    assert_eq!(runs.items.len(), 1, "prefix {applied}: the seeded run");
    let run = &runs.items[0];
    assert_eq!(run.id, id, "prefix {applied}");
    assert_eq!(run.session, SessionKey::new("task-42"), "prefix {applied}");
    assert_eq!(run.state, RunState::Timeout, "prefix {applied}");
    assert_eq!(run.calls_made, 2, "prefix {applied}");
    assert_eq!(
        run.settled.map(|settled| settled.by),
        Some(SettledBy::Runtime),
        "prefix {applied}"
    );
    assert_eq!(
        run.loss,
        Loss {
            stubbed: 1,
            dropped: 0,
            incomplete: false
        },
        "prefix {applied}"
    );
    assert_eq!(
        run.ids,
        BTreeMap::from([task.clone()]),
        "prefix {applied}: the host's id"
    );
    let by_task = store
        .runs(RunQuery {
            id: Some(task),
            ..RunQuery::recent(10)
        })
        .await
        .unwrap();
    assert_eq!(
        by_task.items.iter().map(|run| run.id).collect::<Vec<_>>(),
        [id],
        "prefix {applied}: found by the host's id"
    );

    let calls = store
        .calls(
            &id,
            CallQuery {
                after: None,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!(calls.items.len(), 2, "prefix {applied}: both calls");
    let fill = &calls.items[1];
    assert_eq!(
        (fill.method.as_str(), fill.state, fill.unfinished),
        ("fill", CallState::Unfinished, Some(Unfinished::Timeout)),
        "prefix {applied}"
    );
    assert_eq!(fill.delivery, Some(Delivery::Unknown), "prefix {applied}");
    assert!(fill.settled, "prefix {applied}: the delivery is final");
    assert_eq!(fill.params["value"], "[redacted 9]", "prefix {applied}");
    assert_eq!(fill.log.len(), 2, "prefix {applied}: the folded log");
    let jpeg = fill.after.as_ref().and_then(|after| after.jpeg.clone());
    if applied >= 2 {
        assert_eq!(
            jpeg,
            Some(vec![0xff, 0xd8, 0xff, 0xe0]),
            "prefix {applied}: neo-feeds' capture"
        );
    } else {
        assert_eq!(jpeg, None, "prefix {applied}: no capture before m0002");
    }

    assert_fixture_detail(store, id, applied).await;
}

async fn assert_fixture_detail(store: &TraceStore, id: RunId, applied: usize) {
    let getters = store.getters(&id).await.unwrap();
    assert_eq!(
        getters
            .iter()
            .map(|getter| (getter.field, getter.outcome.clone()))
            .collect::<Vec<_>>(),
        [(
            GetterField::Url,
            GetterOutcome::Ok {
                summary: "https://example.com/login?next=[redacted]".into()
            }
        )],
        "prefix {applied}"
    );
    let trace = store.run(&id).await.unwrap().unwrap();
    assert_eq!(
        trace
            .logs
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["filling the password"],
        "prefix {applied}"
    );
    assert_eq!(
        trace
            .pages
            .iter()
            .map(|page| page.page.tab_id)
            .collect::<Vec<_>>(),
        [Some(7)],
        "prefix {applied}"
    );
    let timeline = store
        .session(&SessionKey::new("task-42"), Timeline::All)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (timeline.runs.len(), timeline.calls.len()),
        (1, 2),
        "prefix {applied}: the session's timeline"
    );
}

#[tokio::test]
async fn claw_servers_ledger_in_the_same_file_is_left_untouched() {
    let (_dir, path) = scratch();
    let conn = inspect(&path).await;
    exec(
        &conn,
        "CREATE TABLE seaql_migrations (version VARCHAR NOT NULL PRIMARY KEY, \
         applied_at BIGINT NOT NULL); \
         INSERT INTO seaql_migrations VALUES ('m0001_baseline', 1), \
         ('m0022_add_mode_to_skill_runs', 2);",
    )
    .await;
    conn.close().await.unwrap();

    let store = open(&path).await;
    assert!(
        !store.health().read_only,
        "claw's ledger is not the store's"
    );
    close(store).await;
    let conn = inspect(&path).await;
    assert_eq!(
        texts(
            &conn,
            "SELECT version FROM seaql_migrations ORDER BY version"
        )
        .await,
        ["m0001_baseline", "m0022_add_mode_to_skill_runs"],
        "claw's ledger is as it was"
    );
    assert_eq!(ledger(&conn).await, MIGRATION_NAMES, "the store's own");
}

#[tokio::test]
async fn deleted_ledger_rows_replay_without_harm() {
    for deleted in [
        format!("DELETE FROM {LEDGER}"),
        format!(
            "DELETE FROM {LEDGER} WHERE version = '{}'",
            MIGRATION_NAMES[MIGRATION_NAMES.len() - 1]
        ),
    ] {
        let (_dir, path) = scratch();
        close(open(&path).await).await;
        let conn = inspect(&path).await;
        exec(&conn, FIXTURES[MIGRATION_NAMES.len()]).await;
        exec(&conn, &deleted).await;
        conn.close().await.unwrap();

        let store = open(&path).await;
        assert!(!store.health().read_only, "after {deleted:?}");
        assert_fixture_reads_back(&store, MIGRATION_NAMES.len()).await;
        close(store).await;
        assert_eq!(
            ledger(&inspect(&path).await).await.len(),
            MIGRATION_NAMES.len(),
            "after {deleted:?} the ledger is whole again"
        );
    }
}

/// `m0001` followed by a migration that creates a table and then fails.
struct FailingSecond;

#[async_trait::async_trait]
impl MigratorTrait for FailingSecond {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        let mut migrations = Migrator::migrations();
        migrations.truncate(1);
        migrations.push(Box::new(Fails));
        migrations
    }

    fn migration_table_name() -> DynIden {
        Migrator::migration_table_name()
    }
}

struct Fails;

impl MigrationName for Fails {
    fn name(&self) -> &'static str {
        "m0002_fails_halfway"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Fails {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("half_done"))
                    .col(ColumnDef::new(Alias::new("id")).integer())
                    .to_owned(),
            )
            .await?;
        Err(DbErr::Migration("the second migration fails".to_string()))
    }
}

#[tokio::test]
async fn a_failing_second_migration_leaves_nothing_of_the_first() {
    let (_dir, path) = scratch();
    let conn = db::connect_writer(&path).await.unwrap();
    let outcome = run_pending::<FailingSecond>(&conn, None).await;
    assert!(
        matches!(outcome, Err(Refusal::Db(_))),
        "the failure surfaces: {outcome:?}"
    );
    assert_eq!(
        objects(&conn, "table").await,
        BTreeSet::new(),
        "the first migration's tables, the half-made one and the ledger all rolled back"
    );
}

#[tokio::test]
async fn a_newer_builds_migration_opens_the_file_read_only() {
    let (_dir, path) = scratch();
    close(open(&path).await).await;
    let conn = inspect(&path).await;
    exec(&conn, FIXTURES[MIGRATION_NAMES.len()]).await;
    exec(
        &conn,
        &format!(
            "INSERT INTO {LEDGER} (version, applied_at) VALUES ('m9999_from_a_newer_build', 0)"
        ),
    )
    .await;
    conn.close().await.unwrap();

    let store = open(&path).await;
    assert!(
        store.health().read_only,
        "a newer build's file is read here, never written"
    );
    assert_fixture_reads_back(&store, MIGRATION_NAMES.len()).await;
    close(store).await;
}

#[tokio::test]
async fn a_newer_builds_migration_beside_a_pending_one_refuses_to_open() {
    let (_dir, path) = scratch();
    let conn = db::connect_writer(&path).await.unwrap();
    assert_eq!(
        run_pending::<Migrator>(&conn, Some(1)).await.unwrap(),
        Schema::Current,
        "the first migration only"
    );
    exec(
        &conn,
        &format!(
            "INSERT INTO {LEDGER} (version, applied_at) VALUES ('m9999_from_a_newer_build', 0)"
        ),
    )
    .await;
    conn.close().await.unwrap();

    let refused = TraceStore::open(StoreConfig::new(&path)).await;
    assert!(
        matches!(&refused, Err(StoreError::Schema { reason, .. })
            if reason.contains("m9999_from_a_newer_build") && reason.contains(MIGRATION_NAMES[1])),
        "diverged: names the newer build's migration and this build's pending one: {:?}",
        refused.err()
    );
}

#[tokio::test]
async fn a_gap_in_the_ledger_refuses_to_open_and_names_both_migrations() {
    let (_dir, path) = scratch();
    close(open(&path).await).await;
    let conn = inspect(&path).await;
    exec(
        &conn,
        &format!(
            "DELETE FROM {LEDGER} WHERE version = '{}'",
            MIGRATION_NAMES[0]
        ),
    )
    .await;
    conn.close().await.unwrap();

    let refused = TraceStore::open(StoreConfig::new(&path)).await;
    assert!(
        matches!(&refused, Err(StoreError::Schema { reason, .. })
            if reason.contains(MIGRATION_NAMES[0]) && reason.contains(MIGRATION_NAMES[1])),
        "a gap names the missing migration and the later applied one: {:?}",
        refused.err()
    );
}
