//! TS → Rust → TS on the same file. The fixtures were written by the TS server's own migrations
//! and stores (`tools/server-compat/make-db-fixture.ts create`); after Rust reads and writes them,
//! the TS server's boot path (`initializeDb` + stores, via Bun) must migrate as a no-op and read
//! exactly what the Rust stores read. That is the rollback guarantee: either binary can run next.

mod db_common;

use db_common::{
    assert_same, count, fixture, open, read_json, rust_dump, stage, ts_dump, without_rust_ledger,
};
use serde_json::json;
use server_rust::db::{
    AgentKind, CreateAgent, Database, ImportConversation, MigrationOutcome, OAuthTokens,
    SaveConversation, TargetType, UpdateAgent, UpsertJob, UpsertProvider, UpsertRun,
};
use std::collections::BTreeMap;
use uuid::Uuid;

#[tokio::test]
async fn ts_written_rows_read_identically_in_rust() -> anyhow::Result<()> {
    let (_dir, paths) = stage(Some("v0014"))?;
    let (db, _) = open(&paths).await?;
    let dump = rust_dump(&db, &paths.db_file).await?;
    let expected = read_json(&fixture("v0014/expected.json"))?;
    assert_same(&without_rust_ledger(&dump), &expected, "TS-written rows");
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn rust_written_rows_read_identically_in_ts() -> anyhow::Result<()> {
    let (_dir, paths) = stage(Some("v0014"))?;
    let (db, _) = open(&paths).await?;
    write_everything(&db).await?;
    let dump = rust_dump(&db, &paths.db_file).await?;
    assert_eq!(count(&paths.db_file, "scheduled_job_runs").await?, 17);
    db.close().await?;

    if let Some(ts) = ts_dump(&paths.db_file)? {
        assert_same(
            &without_rust_ledger(&dump),
            &without_rust_ledger(&ts),
            "TS read-back",
        );
    }
    // Rust → TS → Rust: the TS boot added nothing for Rust to replay.
    let (db, outcome) = open(&paths).await?;
    assert_eq!(outcome, MigrationOutcome::UpToDate);
    assert_same(
        &rust_dump(&db, &paths.db_file).await?,
        &dump,
        "Rust re-read",
    );
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn rust_created_file_opens_in_ts_as_a_no_op() -> anyhow::Result<()> {
    let (_dir, paths) = stage(None)?;
    let (db, _) = open(&paths).await?;
    write_everything(&db).await?;
    let dump = rust_dump(&db, &paths.db_file).await?;
    db.close().await?;
    if let Some(ts) = ts_dump(&paths.db_file)? {
        assert_same(
            &without_rust_ledger(&dump),
            &without_rust_ledger(&ts),
            "TS read-back",
        );
        let fresh = read_json(&fixture("fresh/schema.json"))?;
        assert_same(
            &ts["drizzleMigrations"],
            &fresh["drizzleMigrations"],
            "no replay",
        );
    }
    Ok(())
}

#[tokio::test]
async fn ts_0010_file_after_the_rust_catch_up_reads_identically_in_ts() -> anyhow::Result<()> {
    let (_dir, paths) = stage(Some("v0010"))?;
    let (db, _) = open(&paths).await?;
    let dump = rust_dump(&db, &paths.db_file).await?;
    db.close().await?;
    if let Some(ts) = ts_dump(&paths.db_file)? {
        assert_same(
            &without_rust_ledger(&dump),
            &without_rust_ledger(&ts),
            "TS read-back",
        );
        assert_eq!(ts["runs"].as_array().map(Vec::len), Some(20));
    }
    Ok(())
}

/// The owner's real database, never committed and never opened in place:
/// `cp ~/.browseros/db/browseros.sqlite* $TMP/` then
/// `BROWSEROS_DB_FIXTURE=$TMP/browseros.sqlite cargo test -p server-rust --test db_ts_roundtrip -- --ignored`.
/// Reports only counts and JSON paths, never values.
#[tokio::test]
#[ignore = "needs BROWSEROS_DB_FIXTURE pointing at a copy of a real database"]
async fn owner_database_copy_round_trips() -> anyhow::Result<()> {
    let source = std::path::PathBuf::from(std::env::var("BROWSEROS_DB_FIXTURE")?);
    let stage_copy =
        || -> anyhow::Result<(tempfile::TempDir, server_rust::paths::BrowserosPaths)> {
            let (dir, paths) = stage(None)?;
            std::fs::create_dir_all(
                paths
                    .db_file
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("db dir"))?,
            )?;
            for suffix in ["", "-wal", "-shm"] {
                let mut from = source.as_os_str().to_owned();
                from.push(suffix);
                let mut to = paths.db_file.as_os_str().to_owned();
                to.push(suffix);
                if std::path::Path::new(&from).exists() {
                    std::fs::copy(&from, &to)?;
                }
            }
            Ok((dir, paths))
        };

    let (_ts_dir, ts_paths) = stage_copy()?;
    let ts_before = ts_dump(&ts_paths.db_file)?;

    let (_dir, paths) = stage_copy()?;
    let (db, outcome) = open(&paths).await?;
    let dump = rust_dump(&db, &paths.db_file).await?;
    db.close().await?;
    if let Some(ts_before) = &ts_before {
        assert_same(
            &without_rust_ledger(&dump),
            &without_rust_ledger(ts_before),
            "TS vs Rust read",
        );
    }
    if let Some(ts_after) = ts_dump(&paths.db_file)? {
        assert_same(
            &without_rust_ledger(&dump),
            &without_rust_ledger(&ts_after),
            "TS after Rust",
        );
    }
    report(&outcome, &dump);
    Ok(())
}

#[allow(clippy::print_stdout)]
fn report(outcome: &MigrationOutcome, dump: &serde_json::Value) {
    let length = |key: &str| dump[key].as_array().map_or(0, Vec::len);
    println!(
        "owner db: outcome={outcome:?} drizzle_rows={} providers={} agents={} conversations={} jobs={} runs={} oauth_tokens={}",
        length("drizzleMigrations"),
        length("providers"),
        length("agents"),
        length("conversations"),
        length("jobs"),
        length("runs"),
        length("oauthTokens"),
    );
}

/// One write through every store method that changes rows.
async fn write_everything(db: &Database) -> anyhow::Result<()> {
    let providers = db.providers();
    providers
        .upsert_llm(
            "rust-provider",
            UpsertProvider {
                provider_type: "openrouter".into(),
                name: "Rust".into(),
                model_id: Some("m".into()),
                context_window: Some(64_000),
                supports_images: Some(false),
                temperature: Some(1.0),
                api_key: Some(Some("sk-rust".into())),
                headers: Some(Some(BTreeMap::from([
                    ("X-Title".into(), "BrowserOS".into()),
                    ("HTTP-Referer".into(), "https://browseros.com".into()),
                ]))),
                reasoning_effort: Some(Some("low".into())),
                ..UpsertProvider::default()
            },
        )
        .await?;
    if providers.get("openai-main").await?.is_some() {
        providers
            .upsert_llm(
                "openai-main",
                UpsertProvider {
                    provider_type: "openai".into(),
                    name: "OpenAI renamed".into(),
                    model_id: Some("gpt-5".into()),
                    context_window: Some(128_000),
                    api_key: Some(Some(String::new())),
                    ..UpsertProvider::default()
                },
            )
            .await?;
    }
    providers.set_default("rust-provider").await?;
    providers.delete("ollama-local").await?;

    let agents = db.agents();
    let created = agents
        .create(CreateAgent {
            kind: AgentKind::Custom,
            name: " Rusty ".into(),
            model_id: Some("m1".into()),
            reasoning_effort: None,
            working_directory: None,
            custom_config: Some(r#"{"command":"rusty-acp","env":{"K":"V"}}"#.into()),
        })
        .await?;
    agents
        .update(
            created.id,
            UpdateAgent {
                model_id: Some(None),
                reasoning_effort: Some(Some("high".into())),
                ..UpdateAgent::default()
            },
        )
        .await?;

    let conversations = db.conversations();
    let message = |value: serde_json::Value| serde_json::from_value(value);
    conversations
        .save(SaveConversation {
            id: Uuid::parse_str("77777777-7777-4777-8777-777777777777")?,
            messages: vec![
                message(json!({"id": "u", "role": "user", "parts": [{"type": "text", "text": "hi"}]}))?,
                message(json!({
                    "id": "a",
                    "role": "assistant",
                    "parts": [
                        {"type": "step-start"},
                        {"type": "dynamic-tool", "toolName": "snapshot", "toolCallId": "c", "state": "output-available", "input": {}, "output": {"ok": true}},
                        {"type": "data-x", "data": [1, 2]},
                        {"type": "text", "text": "done", "state": "done"}
                    ]
                }))?,
            ],
            origin: Some("newtab".into()),
            target_type: TargetType::Custom,
            agent_id: Some(created.id.to_string()),
        })
        .await?;
    conversations
        .import_if_absent(
            Uuid::parse_str("88888888-8888-4888-8888-888888888888")?,
            ImportConversation {
                messages: vec![json!({"role": "user", "parts": [{"type": "text", "text": "imported"}], "x": 1})],
                last_messaged_at: Some(1_234),
                target_type: TargetType::BrowserOs,
                origin: Some("legacy".into()),
                agent_id: None,
            },
        )
        .await?;

    let schedules = db.schedules();
    schedules
        .upsert_job(
            "rust-job",
            UpsertJob {
                name: "Rust job".into(),
                query: "q".into(),
                schedule_type: "minutes".into(),
                schedule_interval: Some(Some(5)),
                provider_id: Some(Some("rust-provider".into())),
                ..UpsertJob::default()
            },
        )
        .await?;
    schedules
        .upsert_run(
            "rust-run",
            UpsertRun {
                job_id: "rust-job".into(),
                status: "running".into(),
                started_at: 1_770_000_000_000,
                tool_calls: Some(Some(
                    json!([{"id": "t", "name": "act", "input": null, "timestamp": "t"}]),
                )),
                ..UpsertRun::default()
            },
        )
        .await?;
    if schedules.get_job("job-daily").await?.is_some() {
        // 20 TS runs + 1 → pruned to 15 for this job.
        schedules
            .upsert_run(
                "run-new",
                UpsertRun {
                    job_id: "job-daily".into(),
                    status: "completed".into(),
                    started_at: 1_770_000_000_001,
                    ..UpsertRun::default()
                },
            )
            .await?;
    }

    let tokens = db.oauth_tokens();
    tokens
        .upsert(
            "install-fixture",
            "qwen-code",
            &OAuthTokens {
                access_token: "qa".into(),
                refresh_token: "qr".into(),
                expires_at: 1_800_000_000_000,
                email: None,
                account_id: Some("acct".into()),
            },
        )
        .await?;
    tokens.delete("install-fixture", "github-copilot").await?;
    Ok(())
}
