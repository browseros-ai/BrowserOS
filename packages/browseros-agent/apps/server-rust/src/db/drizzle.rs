//! A port of drizzle-orm 0.45.3's SQLite `migrate()` over the TS server's 15 frozen migrations,
//! so either server can bring the shared file to the same schema and journal.
//!
//! Drizzle's algorithm, kept exactly: create `__drizzle_migrations` if absent, read the newest
//! row's `created_at`, apply every journal entry whose `when` is greater (all of them on a fresh
//! file), split each file on `--> statement-breakpoint`, and record `sha256(file text)` plus
//! `when` per entry, all in one BEGIN/COMMIT. Rows already present are never rerun or rehashed,
//! and a newer TS server's extra rows simply mean nothing here is pending.
//!
//! One deliberate difference: `PRAGMA foreign_keys = OFF` is set before BEGIN and
//! `PRAGMA foreign_key_check` runs before COMMIT. `0011_drop_split_provider_tables.sql` turns
//! foreign keys off itself, which is a no-op inside drizzle's transaction, so on TS its
//! `DROP TABLE scheduled_jobs` cascade-deletes every `scheduled_job_runs` row. Outside the
//! transaction the pragma does what the migration's author meant; schema and journal come out
//! identical. If the check finds violations, the batch is rolled back and replayed TS-identically.

use sea_orm::sqlx::{self, Connection, Row, sqlite::SqliteConnection};
use sha2::{Digest, Sha256};

/// One frozen migration: a byte-copy of `apps/server/src/lib/db/migrations/<tag>.sql` and its
/// journal `when` (`apps/server-rust/migrations/drizzle/`; tests keep both copies equal).
pub(super) struct FrozenMigration {
    pub(super) tag: &'static str,
    pub(super) when: i64,
    pub(super) sql: &'static str,
}

impl FrozenMigration {
    /// What drizzle records in `__drizzle_migrations.hash`.
    pub(super) fn hash(&self) -> String {
        Sha256::digest(self.sql.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

macro_rules! frozen {
    ($tag:literal, $when:literal) => {
        FrozenMigration {
            tag: $tag,
            when: $when,
            sql: include_str!(concat!("../../migrations/drizzle/", $tag, ".sql")),
        }
    };
}

/// Journal order (`meta/_journal.json`).
pub(super) const MIGRATIONS: &[FrozenMigration] = &[
    frozen!("0000_zippy_psylocke", 1777750582590),
    frozen!("0001_lazy_orphan", 1777752799806),
    frozen!("0002_chemical_whirlwind", 1777902205667),
    frozen!("0003_scrub_hermes_credentials", 1781916712443),
    frozen!("0004_sparkling_carnage", 1785893663690),
    frozen!("0005_yellow_riptide", 1785900211901),
    frozen!("0006_add_conversations", 1786538823114),
    frozen!("0007_add_custom_acp_agents", 1787580067090),
    frozen!("0008_add_llm_providers_and_scheduled_jobs", 1788319873053),
    frozen!("0009_add_scheduled_job_runs", 1788413695569),
    frozen!("0010_add_unified_providers_table", 1788426799725),
    frozen!("0011_drop_split_provider_tables", 1788426855683),
    frozen!("0012_add_provider_headers", 1788724664440),
    frozen!("0013_drop_browseros_provider", 1790095966768),
    frozen!("0014_drop_profile_id", 1790235209541),
];

const STATEMENT_BREAKPOINT: &str = "--> statement-breakpoint";

/// drizzle's DDL verbatim (whitespace included), so `sqlite_master` matches a TS-created file.
const CREATE_JOURNAL: &str = "CREATE TABLE IF NOT EXISTS \"__drizzle_migrations\" (\n\t\t\t\tid SERIAL PRIMARY KEY,\n\t\t\t\thash text NOT NULL,\n\t\t\t\tcreated_at numeric\n\t\t\t)";

/// What the catch-up did.
pub(super) struct CatchUp {
    pub(super) applied: usize,
    /// Hashes recorded in the journal that are not among [`MIGRATIONS`]: a newer TS server ran.
    pub(super) unknown: Vec<String>,
}

/// Brings the file to migration 0014 on `connection`, which must be dedicated to this call
/// (foreign-key enforcement is switched per connection and restored before returning).
pub(super) async fn catch_up(connection: &mut SqliteConnection) -> Result<CatchUp, sqlx::Error> {
    sqlx::query(CREATE_JOURNAL)
        .execute(&mut *connection)
        .await?;
    let newest = newest_created_at(connection).await?;
    let unknown = unknown_hashes(connection).await?;
    let pending: Vec<&FrozenMigration> = MIGRATIONS
        .iter()
        // `!lastDbMigration || Number(created_at) < when`; NaN applies nothing, as in JS.
        .filter(|migration| newest.is_none_or(|newest| newest < migration.when as f64))
        .collect();
    if pending.is_empty() {
        return Ok(CatchUp {
            applied: 0,
            unknown,
        });
    }

    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *connection)
        .await?;
    let checked = apply(connection, &pending, true).await;
    let restored = sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *connection)
        .await;
    let violations = checked?;
    restored?;
    if let Some(violations) = violations {
        tracing::warn!(
            violations,
            "foreign key violations after the drizzle catch-up; replaying it exactly as the TS \
             server would, which drops scheduled job runs while migration 0011 rebuilds jobs"
        );
        apply(connection, &pending, false).await?;
    }
    tracing::info!(
        applied = pending.len(),
        from = pending.first().map_or("", |migration| migration.tag),
        "applied drizzle migrations"
    );
    Ok(CatchUp {
        applied: pending.len(),
        unknown,
    })
}

/// One transaction for the whole batch, like drizzle; a failure leaves the file untouched.
/// `Some(n)`: the foreign key check found `n` violations and the batch was rolled back.
async fn apply(
    connection: &mut SqliteConnection,
    pending: &[&FrozenMigration],
    check_foreign_keys: bool,
) -> Result<Option<usize>, sqlx::Error> {
    let mut transaction = connection.begin().await?;
    for migration in pending {
        for statement in migration.sql.split(STATEMENT_BREAKPOINT) {
            if statement.trim().is_empty() {
                continue;
            }
            sqlx::raw_sql(statement).execute(&mut *transaction).await?;
        }
        sqlx::query(r#"INSERT INTO "__drizzle_migrations" ("hash", "created_at") VALUES(?, ?)"#)
            .bind(migration.hash())
            .bind(migration.when)
            .execute(&mut *transaction)
            .await?;
    }
    if check_foreign_keys {
        let violations = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut *transaction)
            .await?;
        if !violations.is_empty() {
            transaction.rollback().await?;
            return Ok(Some(violations.len()));
        }
    }
    transaction.commit().await?;
    Ok(None)
}

/// `Number(created_at)` of drizzle's `ORDER BY created_at DESC LIMIT 1` row; `None` when empty.
async fn newest_created_at(connection: &mut SqliteConnection) -> Result<Option<f64>, sqlx::Error> {
    let Some(row) = sqlx::query(
        r#"SELECT typeof(created_at) AS storage, created_at FROM "__drizzle_migrations" ORDER BY created_at DESC LIMIT 1"#,
    )
    .fetch_optional(&mut *connection)
    .await?
    else {
        return Ok(None);
    };
    let storage: String = row.try_get("storage")?;
    Ok(Some(match storage.as_str() {
        "integer" => row.try_get::<i64, _>("created_at")? as f64,
        "real" => row.try_get::<f64, _>("created_at")?,
        "null" => 0.0,
        "text" => {
            let text: String = row.try_get("created_at")?;
            let text = super::js::trim(&text);
            if text.is_empty() {
                0.0
            } else {
                text.parse().unwrap_or(f64::NAN)
            }
        }
        _ => f64::NAN,
    }))
}

async fn unknown_hashes(connection: &mut SqliteConnection) -> Result<Vec<String>, sqlx::Error> {
    let known: Vec<String> = MIGRATIONS.iter().map(FrozenMigration::hash).collect();
    let recorded: Vec<String> =
        sqlx::query_scalar(r#"SELECT hash FROM "__drizzle_migrations" ORDER BY created_at"#)
            .fetch_all(&mut *connection)
            .await?;
    Ok(recorded
        .into_iter()
        .filter(|hash| !known.contains(hash))
        .collect())
}
