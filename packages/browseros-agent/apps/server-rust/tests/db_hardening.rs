//! Data-safety regressions at the shared-file seam: independent SQLite writers, newer schemas,
//! crash recovery, and exclusive Rust ownership. Fixtures contain synthetic credentials only.
mod db_common;

use db_common::{open, raw, stage};
use sea_orm::sqlx::{self, Connection};
use server_rust::db::UpsertProvider;

#[tokio::test]
async fn newer_schema_is_readable_but_rejects_every_store_write() -> anyhow::Result<()> {
    let (_dir, paths) = stage(Some("v0014"))?;
    let mut conn = raw(&paths.db_file).await?;
    sqlx::query("INSERT INTO __drizzle_migrations(hash,created_at) VALUES('future',9999999999999)")
        .execute(&mut conn)
        .await?;
    conn.close().await?;
    let (db, _) = open(&paths).await?;
    assert!(!db.providers().list().await?.is_empty());
    assert!(
        db.providers().delete("llm-openai").await.is_err(),
        "ahead DB accepted a delete"
    );
    assert!(
        db.providers()
            .upsert_llm("new", UpsertProvider::default())
            .await
            .is_err()
    );
    assert!(db.oauth_tokens().delete("id", "provider").await.is_err());
    assert!(db.schedules().delete_job("id").await.is_err());
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn rust_instances_exclude_each_other_until_close() -> anyhow::Result<()> {
    let (_dir, paths) = stage(None)?;
    let (db, _) = open(&paths).await?;
    assert!(
        open(&paths).await.is_err(),
        "a second Rust instance opened the same DB"
    );
    db.close().await?;
    let (next, _) = open(&paths).await?;
    next.close().await?;
    Ok(())
}

#[tokio::test]
async fn crlf_hashes_at_known_timestamps_are_not_a_newer_schema() -> anyhow::Result<()> {
    let (_dir, paths) = stage(Some("v0014"))?;
    let mut conn = raw(&paths.db_file).await?;
    sqlx::query("UPDATE __drizzle_migrations SET hash='old-platform-line-endings'")
        .execute(&mut conn)
        .await?;
    conn.close().await?;
    let (db, outcome) = open(&paths).await?;
    assert!(!matches!(
        outcome,
        server_rust::db::MigrationOutcome::DatabaseAhead { .. }
    ));
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn schema_page_corruption_is_distinct_and_salvages_every_table() -> anyhow::Result<()> {
    use server_rust::db::MigrationOutcome;
    let (_dir, paths) = stage(Some("v0014"))?;
    let tables = [
        "providers",
        "conversations",
        "oauth_tokens",
        "scheduled_jobs",
        "scheduled_job_runs",
    ];
    let mut expected = Vec::new();
    for table in tables {
        expected.push(db_common::count(&paths.db_file, table).await?);
    }
    let mut bytes = std::fs::read(&paths.db_file)?;
    bytes[100] ^= 0xff; // B-tree header of sqlite_schema: ordinary table queries cannot open it.
    std::fs::write(&paths.db_file, &bytes)?;
    let (db, outcome) = open(&paths).await?;
    let MigrationOutcome::RecreatedAfterCorruption { aside, salvaged } = outcome else {
        anyhow::bail!("corruption was reported as a normal first launch");
    };
    assert_eq!(
        std::fs::read(aside)?,
        bytes,
        "preserve the original for manual recovery"
    );
    for (table, count) in tables.into_iter().zip(expected) {
        assert_eq!(
            db_common::count(&paths.db_file, table).await?,
            count,
            "{table}"
        );
        assert_eq!(salvaged.get(table).copied(), Some(count as u64), "{table}");
    }
    let dump = db_common::rust_dump(&db, &paths.db_file).await?;
    let expected = db_common::read_json(&db_common::fixture("v0014/expected.json"))?;
    for key in [
        "providers",
        "agents",
        "conversations",
        "oauthTokens",
        "jobs",
        "runs",
    ] {
        db_common::assert_same(&dump[key], &expected[key], key);
    }
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn ts_writer_is_waited_out_before_rust_reads_its_write_snapshot() -> anyhow::Result<()> {
    let (_dir, paths) = stage(None)?;
    let (db, _) = open(&paths).await?;
    db.providers()
        .upsert_llm(
            "a",
            UpsertProvider {
                name: "before".into(),
                model_id: Some("model".into()),
                context_window: Some(4096),
                ..Default::default()
            },
        )
        .await?;
    let mut ts = raw(&paths.db_file).await?;
    sqlx::query("BEGIN IMMEDIATE; UPDATE providers SET name='from TS' WHERE id='a'")
        .execute(&mut ts)
        .await?;
    let store = db.providers();
    let pending = tokio::spawn(async move { store.set_default("a").await });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    sqlx::query("COMMIT").execute(&mut ts).await?;
    let row = pending
        .await??
        .ok_or_else(|| anyhow::anyhow!("missing provider"))?;
    assert_eq!(row.name, "from TS");
    assert!(row.is_default);
    ts.close().await?;
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn canceled_writer_admission_does_not_poison_the_pool() -> anyhow::Result<()> {
    let (_dir, paths) = stage(None)?;
    let (db, _) = open(&paths).await?;
    let mut ts = raw(&paths.db_file).await?;
    sqlx::query("BEGIN IMMEDIATE").execute(&mut ts).await?;
    let store = db.providers();
    let pending = tokio::spawn(async move { store.delete("absent").await });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    pending.abort();
    let _ = pending.await;
    sqlx::query("COMMIT").execute(&mut ts).await?;
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        db.providers().upsert_llm(
            "after",
            UpsertProvider {
                model_id: Some("model".into()),
                context_window: Some(4096),
                ..Default::default()
            },
        ),
    )
    .await??;
    ts.close().await?;
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn busy_conversation_save_returns_an_error_and_keeps_history() -> anyhow::Result<()> {
    use server_rust::db::{ImportConversation, SaveConversation, TargetType};
    let (_dir, paths) = stage(None)?;
    let (db, _) = open(&paths).await?;
    let id = uuid::Uuid::new_v4();
    db.conversations()
        .import_if_absent(
            id,
            ImportConversation {
                messages: vec![serde_json::json!({"legacy":"keep"})],
                last_messaged_at: None,
                target_type: TargetType::BrowserOs,
                origin: None,
                agent_id: None,
            },
        )
        .await?;
    let mut ts = raw(&paths.db_file).await?;
    sqlx::query("BEGIN IMMEDIATE").execute(&mut ts).await?;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(4),
        db.conversations().save(SaveConversation {
            id,
            messages: vec![],
            target_type: TargetType::BrowserOs,
            origin: None,
            agent_id: None,
        }),
    )
    .await?;
    assert!(result.is_err(), "busy save was silently acknowledged");
    sqlx::query("ROLLBACK").execute(&mut ts).await?;
    assert_eq!(
        db.conversations()
            .get(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing history"))?
            .messages,
        vec![serde_json::json!({"legacy":"keep"})]
    );
    // Exhaustion must also leave the transaction tracker usable.
    assert!(!db.providers().delete("absent").await?);
    ts.close().await?;
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn lossless_reads_keep_unknown_types_and_exact_string_ids_addressable() -> anyhow::Result<()>
{
    use server_rust::db::{StoredAgentKind, StoredTargetType};
    let (_dir, paths) = stage(None)?;
    let (db, _) = open(&paths).await?;
    let mut ts = raw(&paths.db_file).await?;
    let uppercase_id = "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
    for id in [uppercase_id, "legacy-agent"] {
        sqlx::query("INSERT INTO providers(id,kind,type,name,created_at,updated_at) VALUES(?,'acp','future-agent','Agent',1,1)")
            .bind(id).execute(&mut ts).await?;
    }
    sqlx::query("INSERT INTO conversations(id,messages,target_type,last_messaged_at,created_at,updated_at) VALUES('legacy-conversation','[]','future-target',1,1,1)")
        .execute(&mut ts).await?;
    let rows = db.agents().list_stored().await?;
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|row| row.id == uppercase_id));
    let agent = db
        .agents()
        .get_stored(uppercase_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing agent"))?;
    assert_eq!(agent.kind, StoredAgentKind::Unknown("future-agent".into()));
    let rows = db.conversations().list_stored().await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].target_type,
        StoredTargetType::Unknown("future-target".into())
    );
    assert!(db.conversations().get_stored(&rows[0].id).await?.is_some());
    assert!(db.agents().delete(uppercase_id).await?);
    assert!(db.agents().delete("legacy-agent").await?);
    assert!(db.conversations().delete("legacy-conversation").await?);
    ts.close().await?;
    db.close().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backup_is_a_complete_snapshot_during_checkpointing_writes() -> anyhow::Result<()> {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let (_dir, paths) = stage(Some("v0014"))?;
    let mut ts = raw(&paths.db_file).await?;
    sqlx::query("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=1; CREATE TABLE snapshot_stress(id INTEGER PRIMARY KEY, payload TEXT)")
        .execute(&mut ts).await?;
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let writer = tokio::spawn(async move {
        while !stopped.load(Ordering::Relaxed) {
            sqlx::query("INSERT INTO snapshot_stress(payload) VALUES (hex(randomblob(4096)))")
                .execute(&mut ts)
                .await?;
            sqlx::query("PRAGMA wal_checkpoint(PASSIVE)")
                .execute(&mut ts)
                .await?;
            tokio::task::yield_now().await;
        }
        ts.close().await
    });
    for _ in 0..12 {
        let marker = paths
            .db_file
            .with_file_name("browseros.sqlite.pre-rust.bak");
        if marker.exists() {
            std::fs::remove_file(&marker)?;
        }
        let (db, _) = open(&paths).await?;
        db.close().await?;
        let mut snapshot = raw(&marker).await?;
        let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&mut snapshot)
            .await?;
        assert_eq!(integrity, "ok");
        assert_eq!(db_common::count(&marker, "providers").await?, 8);
        snapshot.close().await?;
    }
    stop.store(true, Ordering::Relaxed);
    writer.await??;
    Ok(())
}

#[tokio::test]
async fn future_timestamp_with_a_known_hash_is_still_ahead() -> anyhow::Result<()> {
    let (_dir, paths) = stage(Some("v0014"))?;
    let mut ts = raw(&paths.db_file).await?;
    sqlx::query("INSERT INTO __drizzle_migrations(hash,created_at) SELECT hash,9999999999999 FROM __drizzle_migrations LIMIT 1")
        .execute(&mut ts).await?;
    ts.close().await?;
    let (db, outcome) = open(&paths).await?;
    assert!(matches!(
        outcome,
        server_rust::db::MigrationOutcome::DatabaseAhead { .. }
    ));
    assert!(db.providers().delete("anything").await.is_err());
    // Even Rust-owned bookkeeping is not added to an ahead TS database.
    let mut raw_db = raw(&paths.db_file).await?;
    let ledgers: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE name='seaql_migrations'")
            .fetch_one(&mut raw_db)
            .await?;
    assert_eq!(ledgers, 0);
    raw_db.close().await?;
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn corrupt_table_does_not_prevent_other_tables_from_being_salvaged() -> anyhow::Result<()> {
    let (_dir, paths) = stage(Some("v0014"))?;
    let mut ts = raw(&paths.db_file).await?;
    let root: i64 =
        sqlx::query_scalar("SELECT rootpage FROM sqlite_master WHERE name='conversations'")
            .fetch_one(&mut ts)
            .await?;
    let page_size: i64 = sqlx::query_scalar("PRAGMA page_size")
        .fetch_one(&mut ts)
        .await?;
    let tokens = db_common::count(&paths.db_file, "oauth_tokens").await?;
    ts.close().await?;
    let mut bytes = std::fs::read(&paths.db_file)?;
    bytes[((root - 1) * page_size) as usize] ^= 0xff;
    std::fs::write(&paths.db_file, bytes)?;
    let (db, outcome) = open(&paths).await?;
    assert!(matches!(
        outcome,
        server_rust::db::MigrationOutcome::RecreatedAfterCorruption { .. }
    ));
    assert_eq!(db.providers().list().await?.len(), 8);
    assert_eq!(
        db_common::count(&paths.db_file, "oauth_tokens").await?,
        tokens
    );
    // The failed VACUUM must not publish its partial output as the completion marker.
    assert!(
        !paths
            .db_file
            .with_file_name("browseros.sqlite.pre-rust.bak")
            .exists()
    );
    db.close().await?;
    Ok(())
}
