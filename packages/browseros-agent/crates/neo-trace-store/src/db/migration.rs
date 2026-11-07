//! Neo: The store's schema as SeaORM migrations, on the store's own ledger.
//!
//! - **Its own ledger, `neo_trace_migrations`.** claw-server reads `seaql_migrations` by name and
//!   takes any row it does not know for a newer build (`cs/db/mod.rs:236-244`), and a neo
//!   migrator on that table would fail on claw's rows. A ledger of its own lets the store live in
//!   any directory, claw's included.
//! - **One transaction.** sea-orm-migration applies SQLite migrations on the bare connection
//!   (`sea-orm-migration-1.1.20/src/migrator.rs:252-273`), so a crash could leave a migration
//!   applied with no ledger row. SQLite DDL is transactional and `Migrator::up` takes a
//!   transaction, so `run_pending` applies every pending migration and its ledger rows in one
//!   (verified by `a_failing_second_migration_leaves_nothing_of_the_first`).
//! - **Guarded and frozen.** Every statement is `if_not_exists`, so replaying a migration whose
//!   ledger row was deleted is harmless, and each migration declares its own idens, never the
//!   entities', so editing an entity never edits history (claw's rule, `cs/db/migration.rs:564-593`).
//!   Builders only, no raw SQL.
//! - **Fixtures by construction.** `FIXTURES` has one seed per prefix of `MIGRATION_NAMES`, so a
//!   migration added without its upgrade fixture fails to compile.
//!
//! `down` is a no-op: dropping traces is retention's job, never a migration's.

use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, TransactionTrait};
use sea_orm_migration::prelude::*;
use tracing::warn;

/// The ledger table. Never `seaql_migrations`.
/// Neo: a private migration ledger prevents collisions with the host database.
pub(crate) const LEDGER: &str = "neo_trace_migrations";

/// Every migration this build knows, in order. Its length sizes [`Migrator::migrations`] and the
/// test fixtures, so the three cannot drift apart.
/// Neo: the ordered schema history also sizes the migration upgrade fixtures.
pub(crate) const MIGRATION_NAMES: [&str; 2] = ["m0001_trace_tables", "m0002_call_artifacts"];

/// One seed per prefix of [`MIGRATION_NAMES`]: `FIXTURES[k]` is data written by a build that knew
/// the first `k` migrations, in that build's schema (an empty file knows none). The upgrade test
/// migrates each prefix, seeds it, opens the store and reads the seed back. A build's data is the
/// previous build's plus what its migration added, so each seed extends the one before it.
#[cfg(test)]
/// Neo: one frozen database seed per migration prefix proves old files remain readable.
pub(crate) const FIXTURES: [&str; MIGRATION_NAMES.len() + 1] = [
    "",
    include_str!("fixtures/m0001.sql"),
    concat!(
        include_str!("fixtures/m0001.sql"),
        include_str!("fixtures/m0002.sql")
    ),
];

pub(crate) struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        let all: [Box<dyn MigrationTrait>; MIGRATION_NAMES.len()] = [
            Box::new(m0001_trace_tables::Migration),
            Box::new(m0002_call_artifacts::Migration),
        ];
        all.into()
    }

    fn migration_table_name() -> DynIden {
        Alias::new(LEDGER).into_iden()
    }
}

/// What the ledger says about a file, against a migrator's list.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Schema {
    /// Every known migration is applied (`run_pending` applied what was missing).
    Current,
    /// A newer build migrated the file: every migration this build knows is applied, plus these.
    /// The schema is a superset, so this build may read it, never write it.
    Ahead { unknown: Vec<String> },
}

/// Why a file cannot be opened at all.
#[derive(Debug)]
pub(crate) enum Refusal {
    Db(DbErr),
    /// The ledger and this build disagree in a way no build can repair: a gap, or a newer
    /// build's migrations alongside ones this build still has to apply.
    Mismatch(String),
}

impl From<DbErr> for Refusal {
    fn from(error: DbErr) -> Self {
        Self::Db(error)
    }
}

/// Compares the ledger with `M`'s migrations and, when nothing is unknown, applies the pending
/// ones (at most `steps` of them) and their ledger rows in one transaction.
///
/// A known name missing while a later one is applied is a gap: replaying it would run an old
/// migration over a newer schema, so the open is refused and both names are given. Unknown names
/// mean a newer build wrote the file: if this build still has migrations to apply the two have
/// diverged and the open is refused, otherwise the file opens read-only.
pub(crate) async fn run_pending<M: MigratorTrait>(
    conn: &DatabaseConnection,
    steps: Option<u32>,
) -> Result<Schema, Refusal> {
    let known: Vec<String> = M::migrations()
        .iter()
        .map(|migration| migration.name().to_string())
        .collect();
    let applied = applied_names(conn).await?;
    let pending: Vec<&String> = known
        .iter()
        .filter(|name| !applied.contains(*name))
        .collect();
    if let Some(missing) = pending.first() {
        let later = known
            .iter()
            .skip_while(|name| name != missing)
            .find(|name| applied.contains(*name));
        if let Some(later) = later {
            return Err(Refusal::Mismatch(format!(
                "migration {missing} is missing from {LEDGER} while the later {later} is applied"
            )));
        }
    }
    let unknown: Vec<String> = applied
        .into_iter()
        .filter(|name| !known.contains(name))
        .collect();
    if !unknown.is_empty() {
        if pending.is_empty() {
            return Ok(Schema::Ahead { unknown });
        }
        return Err(Refusal::Mismatch(format!(
            "a newer build applied {} while this build has yet to apply {}",
            unknown.join(", "),
            pending
                .iter()
                .map(|name| name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    if !pending.is_empty() {
        let txn = conn.begin().await?;
        if let Err(error) = M::up(&txn, steps).await {
            if let Err(rollback) = txn.rollback().await {
                warn!(%rollback, "rolling back the trace store's failed migration");
            }
            return Err(Refusal::Db(error));
        }
        txn.commit().await?;
    }
    Ok(Schema::Current)
}

/// The ledger's names, or none on a file the store has never migrated.
async fn applied_names(conn: &DatabaseConnection) -> Result<Vec<String>, DbErr> {
    if !SchemaManager::new(conn).has_table(LEDGER).await? {
        return Ok(Vec::new());
    }
    let select = Query::select()
        .column(Alias::new("version"))
        .from(Alias::new(LEDGER))
        .order_by(Alias::new("version"), Order::Asc)
        .to_owned();
    let rows = conn
        .query_all(conn.get_database_backend().build(&select))
        .await?;
    rows.iter()
        .map(|row| row.try_get::<String>("", "version"))
        .collect()
}

mod m0001_trace_tables {
    use sea_orm_migration::prelude::*;

    pub(super) struct Migration;

    impl MigrationName for Migration {
        fn name(&self) -> &str {
            super::MIGRATION_NAMES[0]
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            for table in [
                sessions(),
                runs(),
                calls(),
                getters(),
                pages(),
                logs(),
                run_ids(),
            ] {
                manager.create_table(table).await?;
            }
            for index in indexes() {
                manager.create_index(index).await?;
            }
            Ok(())
        }

        async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
            // Traces leave through retention, never through a rollback.
            Ok(())
        }
    }

    fn sessions() -> TableCreateStatement {
        Table::create()
            .table(TraceSessions::Table)
            .if_not_exists()
            .col(
                ColumnDef::new(TraceSessions::SessionKey)
                    .text()
                    .not_null()
                    .primary_key(),
            )
            .col(ColumnDef::new(TraceSessions::Client).text())
            .col(ColumnDef::new(TraceSessions::FirstRunId).text())
            .col(ColumnDef::new(TraceSessions::LastRunId).text())
            .col(
                ColumnDef::new(TraceSessions::Runs)
                    .big_integer()
                    .not_null()
                    .default(0),
            )
            .col(
                ColumnDef::new(TraceSessions::LastActivityMs)
                    .big_integer()
                    .not_null(),
            )
            .col(
                ColumnDef::new(TraceSessions::Withheld)
                    .boolean()
                    .not_null()
                    .default(false),
            )
            .to_owned()
    }

    fn runs() -> TableCreateStatement {
        Table::create()
            .table(TraceRuns::Table)
            .if_not_exists()
            .col(
                ColumnDef::new(TraceRuns::RunId)
                    .text()
                    .not_null()
                    .primary_key(),
            )
            .col(ColumnDef::new(TraceRuns::SessionKey).text().not_null())
            .col(ColumnDef::new(TraceRuns::Client).text())
            .col(ColumnDef::new(TraceRuns::StartPage).text())
            .col(ColumnDef::new(TraceRuns::TimeoutMs).big_integer())
            .col(ColumnDef::new(TraceRuns::CodeBytes).big_integer())
            .col(ColumnDef::new(TraceRuns::Script).text())
            .col(flag(TraceRuns::ScriptTruncated))
            .col(
                ColumnDef::new(TraceRuns::StartedAtMs)
                    .big_integer()
                    .not_null(),
            )
            .col(ColumnDef::new(TraceRuns::Status).text())
            .col(ColumnDef::new(TraceRuns::EndCause).text())
            .col(ColumnDef::new(TraceRuns::ErrorName).text())
            .col(ColumnDef::new(TraceRuns::ErrorMessage).text())
            .col(ColumnDef::new(TraceRuns::ValueJson).text())
            .col(flag(TraceRuns::ValueTruncated))
            .col(ColumnDef::new(TraceRuns::EndedAtMs).big_integer())
            .col(ColumnDef::new(TraceRuns::DurationUs).big_integer())
            .col(count(TraceRuns::CallsMade))
            .col(count(TraceRuns::CallsOverCap))
            .col(ColumnDef::new(TraceRuns::OverCapDelivery).text())
            .col(count(TraceRuns::UnreadableCalls))
            .col(count(TraceRuns::GettersOverCap))
            .col(ColumnDef::new(TraceRuns::SettledAtMs).big_integer())
            .col(ColumnDef::new(TraceRuns::SettledBy).text())
            .col(count(TraceRuns::DetailBytes))
            .col(count(TraceRuns::EventsStubbed))
            .col(count(TraceRuns::EventsDropped))
            .col(flag(TraceRuns::Incomplete))
            .to_owned()
    }

    fn calls() -> TableCreateStatement {
        Table::create()
            .table(TraceCalls::Table)
            .if_not_exists()
            .col(ColumnDef::new(TraceCalls::RunId).text().not_null())
            .col(ColumnDef::new(TraceCalls::CallNo).big_integer().not_null())
            .col(ColumnDef::new(TraceCalls::ParentNo).big_integer())
            .col(ColumnDef::new(TraceCalls::Object).text().not_null())
            .col(ColumnDef::new(TraceCalls::PageId).text())
            .col(ColumnDef::new(TraceCalls::TabId).big_integer())
            .col(ColumnDef::new(TraceCalls::Method).text().not_null())
            .col(ColumnDef::new(TraceCalls::Title).text())
            .col(ColumnDef::new(TraceCalls::Selector).text())
            .col(ColumnDef::new(TraceCalls::ParamsJson).text())
            .col(flag(TraceCalls::ParamsTruncated))
            .col(ColumnDef::new(TraceCalls::TimeoutMs).double())
            .col(ColumnDef::new(TraceCalls::LocLine).big_integer())
            .col(ColumnDef::new(TraceCalls::LocColumn).big_integer())
            .col(
                ColumnDef::new(TraceCalls::StartedAtMs)
                    .big_integer()
                    .not_null(),
            )
            .col(
                ColumnDef::new(TraceCalls::OffsetUs)
                    .big_integer()
                    .not_null(),
            )
            .col(ColumnDef::new(TraceCalls::DurationUs).big_integer())
            .col(ColumnDef::new(TraceCalls::InputOffsetUs).big_integer())
            .col(ColumnDef::new(TraceCalls::State).text().not_null())
            .col(ColumnDef::new(TraceCalls::ErrorName).text())
            .col(ColumnDef::new(TraceCalls::ErrorMessage).text())
            .col(ColumnDef::new(TraceCalls::UnfinishedReason).text())
            .col(ColumnDef::new(TraceCalls::Delivery).text())
            .col(flag(TraceCalls::DeliverySettled))
            .col(ColumnDef::new(TraceCalls::LogJson).text())
            .col(ColumnDef::new(TraceCalls::Detail).text().not_null())
            .primary_key(
                Index::create()
                    .col(TraceCalls::RunId)
                    .col(TraceCalls::CallNo),
            )
            .foreign_key(
                ForeignKey::create()
                    .name("fk_trace_calls_run")
                    .from(TraceCalls::Table, TraceCalls::RunId)
                    .to(TraceRuns::Table, TraceRuns::RunId),
            )
            .to_owned()
    }

    fn getters() -> TableCreateStatement {
        Table::create()
            .table(TraceGetters::Table)
            .if_not_exists()
            .col(ColumnDef::new(TraceGetters::RunId).text().not_null())
            .col(
                ColumnDef::new(TraceGetters::GetterNo)
                    .big_integer()
                    .not_null(),
            )
            .col(ColumnDef::new(TraceGetters::Object).text().not_null())
            .col(ColumnDef::new(TraceGetters::Field).text().not_null())
            .col(ColumnDef::new(TraceGetters::PageId).text())
            .col(ColumnDef::new(TraceGetters::TabId).big_integer())
            .col(ColumnDef::new(TraceGetters::Outcome).text().not_null())
            .col(ColumnDef::new(TraceGetters::SummaryJson).text())
            .col(ColumnDef::new(TraceGetters::ErrorName).text())
            .col(ColumnDef::new(TraceGetters::ErrorMessage).text())
            .col(
                ColumnDef::new(TraceGetters::OffsetUs)
                    .big_integer()
                    .not_null(),
            )
            .col(
                ColumnDef::new(TraceGetters::DurationUs)
                    .big_integer()
                    .not_null(),
            )
            .col(ColumnDef::new(TraceGetters::Detail).text().not_null())
            .primary_key(
                Index::create()
                    .col(TraceGetters::RunId)
                    .col(TraceGetters::GetterNo),
            )
            .foreign_key(
                ForeignKey::create()
                    .name("fk_trace_getters_run")
                    .from(TraceGetters::Table, TraceGetters::RunId)
                    .to(TraceRuns::Table, TraceRuns::RunId),
            )
            .to_owned()
    }

    fn pages() -> TableCreateStatement {
        Table::create()
            .table(TracePages::Table)
            .if_not_exists()
            .col(ColumnDef::new(TracePages::RunId).text().not_null())
            .col(ColumnDef::new(TracePages::PageId).text().not_null())
            .col(ColumnDef::new(TracePages::TabId).big_integer())
            .col(flag(TracePages::Created))
            .col(ColumnDef::new(TracePages::ByCall).big_integer())
            .col(ColumnDef::new(TracePages::OpenedOffsetUs).big_integer())
            .col(ColumnDef::new(TracePages::ClosedOffsetUs).big_integer())
            .primary_key(
                Index::create()
                    .col(TracePages::RunId)
                    .col(TracePages::PageId),
            )
            .foreign_key(
                ForeignKey::create()
                    .name("fk_trace_pages_run")
                    .from(TracePages::Table, TracePages::RunId)
                    .to(TraceRuns::Table, TraceRuns::RunId),
            )
            .to_owned()
    }

    fn logs() -> TableCreateStatement {
        Table::create()
            .table(TraceLogs::Table)
            .if_not_exists()
            .col(ColumnDef::new(TraceLogs::RunId).text().not_null())
            .col(ColumnDef::new(TraceLogs::LineNo).big_integer().not_null())
            .col(ColumnDef::new(TraceLogs::OffsetUs).big_integer().not_null())
            .col(ColumnDef::new(TraceLogs::Text).text().not_null())
            .primary_key(Index::create().col(TraceLogs::RunId).col(TraceLogs::LineNo))
            .foreign_key(
                ForeignKey::create()
                    .name("fk_trace_logs_run")
                    .from(TraceLogs::Table, TraceLogs::RunId)
                    .to(TraceRuns::Table, TraceRuns::RunId),
            )
            .to_owned()
    }

    fn run_ids() -> TableCreateStatement {
        Table::create()
            .table(TraceRunIds::Table)
            .if_not_exists()
            .col(ColumnDef::new(TraceRunIds::RunId).text().not_null())
            .col(ColumnDef::new(TraceRunIds::Key).text().not_null())
            .col(ColumnDef::new(TraceRunIds::Value).text().not_null())
            .primary_key(
                Index::create()
                    .col(TraceRunIds::RunId)
                    .col(TraceRunIds::Key),
            )
            .foreign_key(
                ForeignKey::create()
                    .name("fk_trace_run_ids_run")
                    .from(TraceRunIds::Table, TraceRunIds::RunId)
                    .to(TraceRuns::Table, TraceRuns::RunId),
            )
            .to_owned()
    }

    /// The design's partial index on unsettled runs becomes a plain index on `settled_at_ms`:
    /// the schema builder has no `WHERE` for an index, and a NULL lookup on it is still a range.
    fn indexes() -> [IndexCreateStatement; 6] {
        [
            Index::create()
                .name("ix_trace_runs_session")
                .table(TraceRuns::Table)
                .col(TraceRuns::SessionKey)
                .col(TraceRuns::RunId)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("ix_trace_runs_unsettled")
                .table(TraceRuns::Table)
                .col(TraceRuns::SettledAtMs)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("ix_trace_calls_page")
                .table(TraceCalls::Table)
                .col(TraceCalls::PageId)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("ix_trace_calls_tab")
                .table(TraceCalls::Table)
                .col(TraceCalls::TabId)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("ix_trace_sessions_activity")
                .table(TraceSessions::Table)
                .col(TraceSessions::LastActivityMs)
                .if_not_exists()
                .to_owned(),
            Index::create()
                .name("ix_trace_run_ids_value")
                .table(TraceRunIds::Table)
                .col(TraceRunIds::Key)
                .col(TraceRunIds::Value)
                .if_not_exists()
                .to_owned(),
        ]
    }

    fn flag<T: IntoIden>(column: T) -> ColumnDef {
        ColumnDef::new(column)
            .boolean()
            .not_null()
            .default(false)
            .to_owned()
    }

    fn count<T: IntoIden>(column: T) -> ColumnDef {
        ColumnDef::new(column)
            .big_integer()
            .not_null()
            .default(0)
            .to_owned()
    }

    #[derive(DeriveIden)]
    enum TraceSessions {
        Table,
        SessionKey,
        Client,
        FirstRunId,
        LastRunId,
        Runs,
        LastActivityMs,
        Withheld,
    }

    #[derive(DeriveIden)]
    enum TraceRuns {
        Table,
        RunId,
        SessionKey,
        Client,
        StartPage,
        TimeoutMs,
        CodeBytes,
        Script,
        ScriptTruncated,
        StartedAtMs,
        Status,
        EndCause,
        ErrorName,
        ErrorMessage,
        ValueJson,
        ValueTruncated,
        EndedAtMs,
        DurationUs,
        CallsMade,
        CallsOverCap,
        OverCapDelivery,
        UnreadableCalls,
        GettersOverCap,
        SettledAtMs,
        SettledBy,
        DetailBytes,
        EventsStubbed,
        EventsDropped,
        Incomplete,
    }

    #[derive(DeriveIden)]
    enum TraceCalls {
        Table,
        RunId,
        CallNo,
        ParentNo,
        Object,
        PageId,
        TabId,
        Method,
        Title,
        Selector,
        ParamsJson,
        ParamsTruncated,
        TimeoutMs,
        LocLine,
        LocColumn,
        StartedAtMs,
        OffsetUs,
        DurationUs,
        InputOffsetUs,
        State,
        ErrorName,
        ErrorMessage,
        UnfinishedReason,
        Delivery,
        DeliverySettled,
        LogJson,
        Detail,
    }

    #[derive(DeriveIden)]
    enum TraceGetters {
        Table,
        RunId,
        GetterNo,
        Object,
        Field,
        PageId,
        TabId,
        Outcome,
        SummaryJson,
        ErrorName,
        ErrorMessage,
        OffsetUs,
        DurationUs,
        Detail,
    }

    #[derive(DeriveIden)]
    enum TracePages {
        Table,
        RunId,
        PageId,
        TabId,
        Created,
        ByCall,
        OpenedOffsetUs,
        ClosedOffsetUs,
    }

    #[derive(DeriveIden)]
    enum TraceLogs {
        Table,
        RunId,
        LineNo,
        OffsetUs,
        Text,
    }

    #[derive(DeriveIden)]
    enum TraceRunIds {
        Table,
        RunId,
        Key,
        Value,
    }
}

mod m0002_call_artifacts {
    use sea_orm_migration::prelude::*;

    pub(super) struct Migration;

    impl MigrationName for Migration {
        fn name(&self) -> &str {
            super::MIGRATION_NAMES[1]
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for Migration {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            // neo-feeds' after-call captures, keyed like the call they follow, so one retention
            // owns them. A table of its own keeps the upgrade test's second step real.
            manager
                .create_table(
                    Table::create()
                        .table(TraceCallArtifacts::Table)
                        .if_not_exists()
                        .col(ColumnDef::new(TraceCallArtifacts::RunId).text().not_null())
                        .col(
                            ColumnDef::new(TraceCallArtifacts::CallNo)
                                .big_integer()
                                .not_null(),
                        )
                        .col(ColumnDef::new(TraceCallArtifacts::TabId).big_integer())
                        .col(ColumnDef::new(TraceCallArtifacts::Url).text())
                        .col(ColumnDef::new(TraceCallArtifacts::Title).text())
                        .col(ColumnDef::new(TraceCallArtifacts::Jpeg).blob())
                        .col(
                            ColumnDef::new(TraceCallArtifacts::CapturedAtMs)
                                .big_integer()
                                .not_null(),
                        )
                        .primary_key(
                            Index::create()
                                .col(TraceCallArtifacts::RunId)
                                .col(TraceCallArtifacts::CallNo),
                        )
                        .foreign_key(
                            ForeignKey::create()
                                .name("fk_trace_call_artifacts_run")
                                .from(TraceCallArtifacts::Table, TraceCallArtifacts::RunId)
                                .to(TraceRuns::Table, TraceRuns::RunId),
                        )
                        .to_owned(),
                )
                .await
        }

        async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
            // Traces leave through retention, never through a rollback.
            Ok(())
        }
    }

    #[derive(DeriveIden)]
    enum TraceRuns {
        Table,
        RunId,
    }

    #[derive(DeriveIden)]
    enum TraceCallArtifacts {
        Table,
        RunId,
        CallNo,
        TabId,
        Url,
        Title,
        Jpeg,
        CapturedAtMs,
    }
}
