//! The proof that this server adopts the old server's database without breaking it: files at
//! each stage of the old server's life are built from the fifteen Drizzle migrations it
//! shipped (applied the way Drizzle applies them), seeded with rows of every kind, opened with
//! `Store::open`, and read back through the entities.

use sea_orm::{
    ConnectionTrait, Database, DatabaseConnection, EntityTrait, Statement, TransactionTrait,
};

use super::entities::{conversations, oauth_tokens, providers};
use super::migration::{DRIZZLE_MIGRATIONS, LegacySchema, replay_drizzle};
use super::{DATABASE_FILE, Store, StoreError};
use crate::turn::{MessageId, Role, UiMessage, UiPart};

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn raw(dir: &std::path::Path) -> Result<DatabaseConnection, sea_orm::DbErr> {
    std::fs::create_dir_all(dir.join("db")).map_err(|e| sea_orm::DbErr::Custom(e.to_string()))?;
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.join("db").join(DATABASE_FILE).display()
    );
    Database::connect(url).await
}

/// What the old server's Drizzle migrator leaves after shipping migrations `0..=last`.
async fn old_server_at(db: &DatabaseConnection, last: usize) -> Result<(), sea_orm::DbErr> {
    db.execute_unprepared(
        "CREATE TABLE IF NOT EXISTS \"__drizzle_migrations\" (id SERIAL PRIMARY KEY, hash text NOT NULL, created_at numeric)",
    )
    .await?;
    let transaction = db.begin().await?;
    for migration in &DRIZZLE_MIGRATIONS[..=last] {
        for statement in migration.sql.split("--> statement-breakpoint") {
            transaction.execute_unprepared(statement).await?;
        }
        transaction
            .execute(Statement::from_sql_and_values(
                db.get_database_backend(),
                "INSERT INTO \"__drizzle_migrations\" (\"hash\", \"created_at\") VALUES(?, ?)",
                [migration.hash().into(), migration.created_at.into()],
            ))
            .await?;
    }
    transaction.commit().await
}

async fn count(db: &DatabaseConnection, sql: &str) -> Result<i64, sea_orm::DbErr> {
    let row = db
        .query_one(Statement::from_string(db.get_database_backend(), sql))
        .await?
        .ok_or_else(|| sea_orm::DbErr::Custom("no row".into()))?;
    row.try_get_by_index::<i64>(0)
}

const CONVERSATION: &str = "5f0d7c2a-8f7e-4f7c-9a55-2f0a8c1b9d10";

/// An old-server transcript: the assistant id is empty, and a tool part names a tool this
/// server never had.
const OLD_TRANSCRIPT: &str = r#"[{"id":"u1","role":"user","parts":[{"type":"text","text":"Open example.com"}]},{"id":"","role":"assistant","parts":[{"type":"step-start"},{"type":"tool-navigate","toolCallId":"c1","state":"output-available","input":{"url":"https://example.com"},"output":"ok"},{"type":"text","text":"Done","state":"done"}]}]"#;

#[tokio::test]
async fn a_blank_file_ends_exactly_where_the_old_server_would_leave_it() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path()).await?;
    let db = store.db();
    assert_eq!(
        count(db, "SELECT count(*) FROM __drizzle_migrations").await?,
        15
    );
    for migration in &DRIZZLE_MIGRATIONS {
        let recorded = count(
            db,
            &format!(
                "SELECT count(*) FROM __drizzle_migrations WHERE hash = '{}' AND created_at = {}",
                migration.hash(),
                migration.created_at
            ),
        )
        .await?;
        assert_eq!(
            recorded, 1,
            "{} is recorded as Drizzle records it",
            migration.tag
        );
    }
    assert_eq!(
        count(db, "SELECT count(*) FROM seaql_migrations WHERE version = 'm20261121_000001_adopt_existing'").await?,
        1
    );
    // A second open finds nothing to do and changes nothing.
    assert_eq!(replay_drizzle(db).await?, LegacySchema::Current);
    store.close().await;
    let again = Store::open(dir.path()).await?;
    assert_eq!(
        count(again.db(), "SELECT count(*) FROM __drizzle_migrations").await?,
        15
    );
    Ok(())
}

#[tokio::test]
async fn a_file_at_0009_gets_0010_to_0014_and_its_rows_move_to_the_unified_table() -> TestResult {
    let dir = tempfile::tempdir()?;
    let db = raw(dir.path()).await?;
    old_server_at(&db, 9).await?;
    db.execute_unprepared(&format!(
        "INSERT INTO llm_providers (id, type, name, model_id, context_window, api_key, created_at, updated_at) \
           VALUES ('p1', 'openrouter', 'Router', 'openai/gpt-4.1-mini', 128000, 'sk-test', 1, 2); \
         INSERT INTO acp_agents (id, name, type, created_at, updated_at) VALUES ('a1', 'Codex', 'codex', 3, 4); \
         INSERT INTO conversations (id, messages, target_type, last_messaged_at, created_at, updated_at) \
           VALUES ('{CONVERSATION}', '{OLD_TRANSCRIPT}', 'browseros', 5, 5, 5); \
         INSERT INTO oauth_tokens (browseros_id, provider, access_token, refresh_token, expires_at, updated_at) \
           VALUES ('install', 'chatgpt-pro', 'at', 'rt', 0, 6); \
         INSERT INTO scheduled_jobs (id, name, query, schedule_type, provider_id, created_at, updated_at) \
           VALUES ('j1', 'Daily', 'news', 'daily', 'p1', 7, 7); \
         INSERT INTO scheduled_job_runs (id, job_id, status, started_at, created_at, updated_at) \
           VALUES ('r1', 'j1', 'completed', 8, 8, 8);"
    ))
    .await?;
    db.close().await?;

    let store = Store::open(dir.path()).await?;
    let db = store.db();
    assert_eq!(
        count(db, "SELECT count(*) FROM __drizzle_migrations").await?,
        15
    );
    let rows = providers::Entity::find().all(db).await?;
    let llm = rows
        .iter()
        .find(|row| row.id == "p1")
        .ok_or("the llm row moved")?;
    assert_eq!(
        (llm.kind.as_str(), llm.api_key.as_deref()),
        ("llm", Some("sk-test"))
    );
    let acp = rows
        .iter()
        .find(|row| row.id == "a1")
        .ok_or("the acp row moved")?;
    assert_eq!((acp.kind.as_str(), acp.r#type.as_str()), ("acp", "codex"));
    assert_eq!(oauth_tokens::Entity::find().all(db).await?.len(), 1);
    // 0011 rebuilds scheduled_jobs; with the old migrator the drop cascaded into the runs.
    assert_eq!(
        count(
            db,
            "SELECT count(*) FROM scheduled_jobs WHERE provider_id = 'p1'"
        )
        .await?,
        1
    );
    assert_eq!(
        count(db, "SELECT count(*) FROM scheduled_job_runs").await?,
        1,
        "run history survives"
    );
    let pragma = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            "PRAGMA foreign_keys",
        ))
        .await?
        .ok_or("pragma")?;
    assert_eq!(
        pragma.try_get_by_index::<i64>(0)?,
        1,
        "foreign keys are back on after the replay"
    );

    let id = uuid::Uuid::parse_str(CONVERSATION)?;
    let conversation = store
        .conversations()
        .load(id)
        .await?
        .ok_or("the conversation reads")?;
    assert_eq!(conversation.messages[1].id, MessageId("legacy-1".into()));
    assert!(
        matches!(conversation.messages[1].parts[1], UiPart::Tool(ref tool) if tool.name == "navigate")
    );
    Ok(())
}

#[tokio::test]
async fn a_file_at_0014_with_rows_of_every_kind_reads_back_through_the_entities() -> TestResult {
    let dir = tempfile::tempdir()?;
    let db = raw(dir.path()).await?;
    old_server_at(&db, 14).await?;
    db.execute_unprepared(&format!(
        "INSERT INTO providers (id, kind, type, name, model_id, context_window, is_default, api_key, headers, created_at, updated_at) \
           VALUES ('p1', 'llm', 'openai', 'OpenAI', 'gpt-5', 400000, 1, 'sk-a', '{{\"x-id\":\"{{{{conversationId}}}}\"}}', 1, 1); \
         INSERT INTO providers (id, kind, type, name, model_id, context_window, created_at, updated_at) \
           VALUES ('p2', 'llm', 'github-copilot', 'Copilot', 'gpt-4o', 64000, 1, 1); \
         INSERT INTO providers (id, kind, type, name, working_directory, created_at, updated_at) \
           VALUES ('a1', 'acp', 'claude', 'Claude', '/tmp/work', 1, 1); \
         INSERT INTO providers (id, kind, type, name, custom_config, created_at, updated_at) \
           VALUES ('a2', 'acp', 'custom', 'Mine', '{{\"command\":\"x\"}}', 1, 1); \
         INSERT INTO conversations (id, messages, target_type, agent_id, last_messaged_at, created_at, updated_at) \
           VALUES ('{CONVERSATION}', '{OLD_TRANSCRIPT}', 'claude', 'a1', 9, 1, 9); \
         INSERT INTO conversations (id, messages, target_type, last_messaged_at, created_at, updated_at) \
           VALUES ('0b7d1a52-1b8e-4c3f-8d9a-6c5e4f3a2b10', '[]', 'custom', 3, 1, 3); \
         INSERT INTO oauth_tokens (browseros_id, provider, access_token, refresh_token, expires_at, email, account_id, updated_at) \
           VALUES ('install', 'chatgpt-pro', 'at', 'rt', 1790000000000, 'me@example.com', 'acct', 1); \
         INSERT INTO scheduled_jobs (id, name, query, schedule_type, provider_id, created_at, updated_at) \
           VALUES ('j1', 'Daily', 'news', 'daily', 'p1', 1, 1); \
         INSERT INTO scheduled_job_runs (id, job_id, status, started_at, created_at, updated_at) \
           VALUES ('r1', 'j1', 'completed', 1, 1, 1);"
    ))
    .await?;
    db.close().await?;

    let store = Store::open(dir.path()).await?;
    let db = store.db();
    assert_eq!(
        count(db, "SELECT count(*) FROM __drizzle_migrations").await?,
        15
    );
    let rows = providers::Entity::find().all(db).await?;
    assert_eq!(rows.len(), 4, "legacy types stay in the table");
    let default = rows
        .iter()
        .find(|row| row.is_default)
        .ok_or("one default")?;
    assert_eq!(
        default.headers.as_deref(),
        Some(r#"{"x-id":"{{conversationId}}"}"#)
    );
    assert_eq!(conversations::Entity::find().all(db).await?.len(), 2);
    let token = oauth_tokens::Entity::find()
        .one(db)
        .await?
        .ok_or("the token reads")?;
    assert_eq!(token.email.as_deref(), Some("me@example.com"));
    assert_eq!(
        count(
            db,
            "SELECT count(*) FROM scheduled_jobs WHERE provider_id = 'p1'"
        )
        .await?,
        1
    );
    assert_eq!(
        count(db, "SELECT count(*) FROM scheduled_job_runs").await?,
        1
    );

    let summaries = store.conversations().list().await?;
    assert_eq!(summaries.len(), 2);
    assert_eq!(
        summaries[0].id,
        uuid::Uuid::parse_str(CONVERSATION)?,
        "newest first"
    );
    Ok(())
}

#[tokio::test]
async fn a_file_ahead_of_this_build_is_read_forward_untouched() -> TestResult {
    let dir = tempfile::tempdir()?;
    let db = raw(dir.path()).await?;
    old_server_at(&db, 14).await?;
    db.execute_unprepared(
        "INSERT INTO __drizzle_migrations (hash, created_at) VALUES ('future', 1890000000000)",
    )
    .await?;
    db.close().await?;
    let store = Store::open(dir.path()).await?;
    assert_eq!(
        replay_drizzle(store.db()).await?,
        LegacySchema::Ahead {
            last_created_at: 1_890_000_000_000
        }
    );
    assert_eq!(
        count(store.db(), "SELECT count(*) FROM __drizzle_migrations").await?,
        16
    );
    Ok(())
}

#[tokio::test]
async fn a_file_without_the_0014_tables_is_refused_not_rewritten() -> TestResult {
    let dir = tempfile::tempdir()?;
    let db = raw(dir.path()).await?;
    db.execute_unprepared(
        "CREATE TABLE \"__drizzle_migrations\" (id SERIAL PRIMARY KEY, hash text NOT NULL, created_at numeric)",
    )
    .await?;
    for migration in &DRIZZLE_MIGRATIONS {
        db.execute_unprepared(&format!(
            "INSERT INTO __drizzle_migrations (hash, created_at) VALUES ('{}', {})",
            migration.hash(),
            migration.created_at
        ))
        .await?;
    }
    db.close().await?;
    let result = Store::open(dir.path()).await;
    assert!(matches!(result, Err(StoreError::Schema(ref missing)) if missing == "providers"));
    Ok(())
}

#[tokio::test]
async fn a_corrupt_file_is_moved_aside_with_its_wal_and_a_fresh_one_made() -> TestResult {
    let dir = tempfile::tempdir()?;
    let db_dir = dir.path().join("db");
    std::fs::create_dir_all(&db_dir)?;
    std::fs::write(
        db_dir.join(DATABASE_FILE),
        b"this is not a database, it is a sentence",
    )?;
    let store = Store::open(dir.path()).await?;
    assert_eq!(
        count(store.db(), "SELECT count(*) FROM __drizzle_migrations").await?,
        15
    );
    store.close().await;
    assert_eq!(
        std::fs::read(db_dir.join(format!("{DATABASE_FILE}.1.bak")))?,
        b"this is not a database, it is a sentence"
    );
    // A second corruption keeps the first backup and takes the next number.
    std::fs::write(db_dir.join(DATABASE_FILE), b"broken again")?;
    drop(Store::open(dir.path()).await?);
    assert_eq!(
        std::fs::read(db_dir.join(format!("{DATABASE_FILE}.1.bak")))?,
        b"this is not a database, it is a sentence"
    );
    assert_eq!(
        std::fs::read(db_dir.join(format!("{DATABASE_FILE}.2.bak")))?,
        b"broken again"
    );
    Ok(())
}

#[tokio::test]
async fn a_file_that_needs_writing_while_locked_is_reported_busy_and_left_in_place() -> TestResult {
    let dir = tempfile::tempdir()?;
    let holder = raw(dir.path()).await?;
    old_server_at(&holder, 9).await?;
    // WAL lets an adopted file open while another process writes; a file that still needs
    // the replay has to wait for the writer.
    let lock = holder.begin().await?;
    lock.execute_unprepared("DELETE FROM oauth_tokens").await?;
    let result = Store::open(dir.path()).await;
    assert!(
        matches!(&result, Err(error) if error.is_busy()),
        "{:?}",
        result.err()
    );
    lock.rollback().await?;
    assert!(
        !dir.path()
            .join("db")
            .join(format!("{DATABASE_FILE}.1.bak"))
            .exists()
    );
    Ok(())
}

#[tokio::test]
async fn a_saved_transcript_loads_as_saved_and_feeds_the_history_list() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path()).await?;
    let id = uuid::Uuid::new_v4();
    let messages: Vec<UiMessage> = serde_json::from_str(OLD_TRANSCRIPT)?;
    let row = super::ConversationRow {
        id,
        messages: vec![
            messages[0].clone(),
            UiMessage {
                id: MessageId("a1".into()),
                role: Role::Assistant,
                ..messages[1].clone()
            },
        ],
        target_type: "browseros".into(),
        agent_id: None,
    };
    store.conversations().save(&row).await?;
    store.conversations().save(&row).await?;
    assert_eq!(store.conversations().load(id).await?, Some(row));
    let list = store.conversations().list().await?;
    assert_eq!(list.len(), 1);
    assert_eq!(
        list[0].last_user_message.as_deref(),
        Some("Open example.com")
    );
    assert!(store.conversations().delete(id).await?);
    assert!(!store.conversations().delete(id).await?);
    Ok(())
}
