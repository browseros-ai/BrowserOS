//! The migrations of `browseros.sqlite`, and why the first one replays the old server's own.
//!
//! The TypeScript server owns this file's schema through Drizzle: fifteen SQL migrations,
//! `0000` to `0014`, recorded in `__drizzle_migrations` by `created_at` (the journal's `when`).
//! The browser falls back to that server after three failed health strikes, so for a while it
//! may open the same file again, and its migrator applies every migration newer than the last
//! recorded `created_at`. A file this server touched must therefore always be a file that
//! migrator accepts:
//!
//! - **Blank or older than 0014**: creating the final tables with `IF NOT EXISTS` would break the
//!   old migrator (`0001` fails with "table oauth_tokens already exists"), and leaving the old
//!   tables would break this server ("no such column: headers"). So the missing Drizzle
//!   migrations are replayed here as Drizzle runs them: the same statements split on
//!   `--> statement-breakpoint`, in one transaction, each recorded with the same hash and
//!   `created_at`. The file ends with the schema and bookkeeping the old server would leave, and
//!   it finds nothing left to do.
//!
//!   One deliberate difference: foreign keys are off during the replay. Drizzle runs inside
//!   `BEGIN`, where SQLite ignores `PRAGMA foreign_keys`, so 0011's `foreign_keys=OFF` block
//!   never took effect for the old server: its `DROP TABLE scheduled_jobs` cascaded and deleted
//!   every `scheduled_job_runs` row. 0011's own comment says the drops were meant to run with
//!   foreign keys off, and this server deletes no row (design rule 7), so here they are off.
//!   The schema is the same either way; only the run history survives.
//! - **At 0014**: nothing to replay; the tables are adopted as they are.
//! - **Newer than this build knows**: left alone and read forward, with a warning.
//!
//! Nothing is ever dropped or altered beyond what the old server's own migrations do, and
//! `__drizzle_migrations` stays: it is the old server's bookkeeping. `Store::open` runs
//! `replay_drizzle`, checks the result is the 0014 schema, and only then runs this server's own
//! migrator, SeaORM's, whose bookkeeping (`seaql_migrations`) the old server ignores. Its first
//! migration creates nothing: it records the adoption. Later migrations that only this server
//! needs go after it, and must stay compatible with the old server while it can still run.

use sea_orm::{ConnectionTrait, DbErr, Statement, TransactionTrait};
use sea_orm_migration::{MigrationName, MigrationTrait, MigratorTrait, SchemaManager};
use sha2::{Digest, Sha256};

/// This server's migrations, in order.
pub(super) struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(M20261121AdoptExisting)]
    }
}

/// Records that this server adopted the old server's tables. Creates nothing: `replay_drizzle`
/// already brought the file to 0014 before SeaORM's migrator runs.
pub(super) struct M20261121AdoptExisting;

impl MigrationName for M20261121AdoptExisting {
    fn name(&self) -> &str {
        "m20261121_000001_adopt_existing"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for M20261121AdoptExisting {
    async fn up(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }

    /// Irreversible on purpose: nothing this migration did is safe to remove from a user's
    /// database, and the old server still needs every table it made.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

/// One of the old server's migrations, compiled in byte for byte: its hash is the SHA-256 of
/// the file, as Drizzle computes it.
pub(super) struct DrizzleMigration {
    pub tag: &'static str,
    /// The journal's `when`, which Drizzle records as `created_at` and compares by.
    pub created_at: i64,
    pub sql: &'static str,
}

macro_rules! drizzle {
    ($tag:literal, $created_at:literal) => {
        DrizzleMigration {
            tag: $tag,
            created_at: $created_at,
            sql: include_str!(concat!("legacy/", $tag, ".sql")),
        }
    };
}

/// The old server's migrations, in journal order (`legacy/meta/_journal.json`; a test checks
/// that this table and the journal agree).
pub(super) const DRIZZLE_MIGRATIONS: [DrizzleMigration; 15] = [
    drizzle!("0000_zippy_psylocke", 1777750582590),
    drizzle!("0001_lazy_orphan", 1777752799806),
    drizzle!("0002_chemical_whirlwind", 1777902205667),
    drizzle!("0003_scrub_hermes_credentials", 1781916712443),
    drizzle!("0004_sparkling_carnage", 1785893663690),
    drizzle!("0005_yellow_riptide", 1785900211901),
    drizzle!("0006_add_conversations", 1786538823114),
    drizzle!("0007_add_custom_acp_agents", 1787580067090),
    drizzle!("0008_add_llm_providers_and_scheduled_jobs", 1788319873053),
    drizzle!("0009_add_scheduled_job_runs", 1788413695569),
    drizzle!("0010_add_unified_providers_table", 1788426799725),
    drizzle!("0011_drop_split_provider_tables", 1788426855683),
    drizzle!("0012_add_provider_headers", 1788724664440),
    drizzle!("0013_drop_browseros_provider", 1790095966768),
    drizzle!("0014_drop_profile_id", 1790235209541),
];

impl DrizzleMigration {
    pub fn hash(&self) -> String {
        Sha256::digest(self.sql.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// The statements Drizzle runs, one per `--> statement-breakpoint` piece.
    fn statements(&self) -> impl Iterator<Item = &'static str> {
        self.sql.split("--> statement-breakpoint")
    }
}

/// Drizzle's bookkeeping table, created with Drizzle's own text so the stored schema matches a
/// file the old server made.
const CREATE_DRIZZLE_TABLE: &str = "CREATE TABLE IF NOT EXISTS \"__drizzle_migrations\" (\n\t\t\t\tid SERIAL PRIMARY KEY,\n\t\t\t\thash text NOT NULL,\n\t\t\t\tcreated_at numeric\n\t\t\t)";

/// Where the file stood relative to the old server's migrations, before the replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LegacySchema {
    /// Some or all of the fifteen were replayed; the tags, in order.
    Replayed(Vec<&'static str>),
    /// Already at `0014`.
    Current,
    /// Drizzle recorded a migration newer than `0014`: a newer old server wrote this file.
    Ahead { last_created_at: i64 },
}

/// Applies the Drizzle migrations the file is missing, exactly as Drizzle's SQLite migrator
/// does (`drizzle-orm` 0.45.3, `sqlite-core/dialect.js`): create the bookkeeping table, read the
/// last `created_at`, then in ONE transaction for all of them (Drizzle's, not one per file) run
/// every newer migration's statements and record its hash and `created_at`. A failure rolls the
/// whole replay back, as Drizzle's does. Idempotent: a second call finds nothing newer to run.
pub(super) async fn replay_drizzle<C>(connection: &C) -> Result<LegacySchema, DbErr>
where
    C: ConnectionTrait + TransactionTrait,
{
    connection.execute_unprepared(CREATE_DRIZZLE_TABLE).await?;
    let last = last_drizzle_created_at(connection).await?;
    let newest_known = DRIZZLE_MIGRATIONS[DRIZZLE_MIGRATIONS.len() - 1].created_at;
    if let Some(last) = last
        && last > newest_known
    {
        return Ok(LegacySchema::Ahead {
            last_created_at: last,
        });
    }
    let pending: Vec<&DrizzleMigration> = DRIZZLE_MIGRATIONS
        .iter()
        .filter(|migration| last.is_none_or(|last| last < migration.created_at))
        .collect();
    if pending.is_empty() {
        return Ok(LegacySchema::Current);
    }

    // Outside the transaction, where SQLite honours it; the connection pool has one
    // connection, so the pragma and the transaction share it. Restored whatever happens.
    connection
        .execute_unprepared("PRAGMA foreign_keys = OFF")
        .await?;
    let replayed = apply(connection, &pending).await;
    connection
        .execute_unprepared("PRAGMA foreign_keys = ON")
        .await?;
    replayed?;
    Ok(LegacySchema::Replayed(
        pending.iter().map(|migration| migration.tag).collect(),
    ))
}

/// One transaction for every pending migration, as Drizzle's: all of them, or none.
async fn apply<C>(connection: &C, pending: &[&DrizzleMigration]) -> Result<(), DbErr>
where
    C: ConnectionTrait + TransactionTrait,
{
    let backend = connection.get_database_backend();
    let transaction = connection.begin().await?;
    for migration in pending {
        for statement in migration.statements() {
            transaction.execute_unprepared(statement).await?;
        }
        transaction
            .execute(Statement::from_sql_and_values(
                backend,
                "INSERT INTO \"__drizzle_migrations\" (\"hash\", \"created_at\") VALUES(?, ?)",
                [migration.hash().into(), migration.created_at.into()],
            ))
            .await?;
    }
    transaction.commit().await
}

/// The newest `created_at` Drizzle recorded, or `None` on a file it never migrated. The column
/// is `numeric`, so a value may read back as an integer or a real.
async fn last_drizzle_created_at<C: ConnectionTrait>(connection: &C) -> Result<Option<i64>, DbErr> {
    let row = connection
        .query_one(Statement::from_string(
            connection.get_database_backend(),
            "SELECT created_at FROM \"__drizzle_migrations\" ORDER BY created_at DESC LIMIT 1",
        ))
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if let Ok(value) = row.try_get::<Option<i64>>("", "created_at") {
        return Ok(value);
    }
    let value: Option<f64> = row.try_get("", "created_at")?;
    // Journal times are whole milliseconds; a real here only ever holds an integral value.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "integral epoch milliseconds"
    )]
    Ok(value.map(|value| value as i64))
}

/// The tables and the columns of each that the entities name: the 0014 schema this server reads
/// and writes. `Store::open` refuses to start on a file without them, rather than failing later
/// on the first query.
pub(super) const REQUIRED_COLUMNS: [(&str, &[&str]); 3] = [
    (
        "providers",
        &[
            "id",
            "kind",
            "type",
            "name",
            "model_id",
            "reasoning_effort",
            "is_default",
            "created_at",
            "updated_at",
            "base_url",
            "headers",
            "supports_images",
            "context_window",
            "temperature",
            "api_key",
            "access_key_id",
            "secret_access_key",
            "session_token",
            "resource_name",
            "region",
            "reasoning_summary",
            "working_directory",
            "custom_config",
        ],
    ),
    (
        "conversations",
        &[
            "id",
            "messages",
            "last_user_message",
            "origin",
            "target_type",
            "agent_id",
            "last_messaged_at",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "oauth_tokens",
        &[
            "browseros_id",
            "provider",
            "access_token",
            "refresh_token",
            "expires_at",
            "email",
            "account_id",
            "updated_at",
        ],
    ),
];

/// The first table or column of `REQUIRED_COLUMNS` the file lacks, as `table` or `table.column`.
pub(super) async fn missing_column<C: ConnectionTrait>(
    connection: &C,
) -> Result<Option<String>, DbErr> {
    for (table, columns) in REQUIRED_COLUMNS {
        let rows = connection
            .query_all(Statement::from_string(
                connection.get_database_backend(),
                format!("SELECT name FROM pragma_table_info('{table}')"),
            ))
            .await?;
        if rows.is_empty() {
            return Ok(Some(table.to_owned()));
        }
        let present: Vec<String> = rows
            .iter()
            .map(|row| row.try_get::<String>("", "name"))
            .collect::<Result<_, _>>()?;
        if let Some(column) = columns
            .iter()
            .find(|column| !present.iter().any(|name| name == *column))
        {
            return Ok(Some(format!("{table}.{column}")));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_table_is_the_journal_in_order() -> Result<(), serde_json::Error> {
        let journal: serde_json::Value =
            serde_json::from_str(include_str!("legacy/meta/_journal.json"))?;
        let entries: Vec<(String, i64)> = journal["entries"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|entry| {
                (
                    entry["tag"].as_str().unwrap_or_default().to_owned(),
                    entry["when"].as_i64().unwrap_or_default(),
                )
            })
            .collect();
        let embedded: Vec<(String, i64)> = DRIZZLE_MIGRATIONS
            .iter()
            .map(|migration| (migration.tag.to_owned(), migration.created_at))
            .collect();
        assert_eq!(embedded, entries);
        Ok(())
    }

    #[test]
    fn the_files_are_byte_for_byte_the_ones_the_old_server_recorded() {
        // From the old server's own migration history (apps/server/src/lib/db/client.ts).
        assert_eq!(
            DRIZZLE_MIGRATIONS[0].hash(),
            "aadfc2e86410febb11a974d25d99d5f7196aa797d9635ced9a18cd4eeb503b61"
        );
        assert_eq!(
            DRIZZLE_MIGRATIONS[14].hash(),
            "bc71831be007936b7d04aa24c4496a06517e204a78d35af93935f3fc97eb49f9"
        );
    }
}
