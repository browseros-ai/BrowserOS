//! Store behaviour on a fresh file, each case a quirk of the TS store it ports
//! (`apps/server/src/lib/{providers,agents/storage,conversations,schedules,clients/oauth}`).

mod db_common;

use db_common::{open, raw, stage};
use sea_orm::sqlx;
use serde_json::json;
use server_rust::db::{
    AgentKind, CreateAgent, ImportConversation, OAuthTokens, SaveConversation, TargetType,
    UpdateAgent, UpsertJob, UpsertProvider, UpsertRun,
};
use std::{collections::BTreeMap, path::PathBuf};
use ui_message_stream::UiMessage;
use uuid::Uuid;

fn provider(name: &str) -> UpsertProvider {
    UpsertProvider {
        provider_type: "openai".into(),
        name: name.into(),
        model_id: Some("gpt-5".into()),
        context_window: Some(128_000),
        ..UpsertProvider::default()
    }
}

#[tokio::test]
async fn provider_upsert_follows_drizzle_conflict_rules() -> anyhow::Result<()> {
    let (_dir, paths) = stage(None)?;
    let (db, _) = open(&paths).await?;
    let store = db.providers();

    let inserted = store
        .upsert_llm(
            "p1",
            UpsertProvider {
                api_key: Some(Some("sk-1".into())),
                session_token: Some(Some(String::new())),
                headers: Some(Some(BTreeMap::from([("X-A".into(), "1".into())]))),
                created_at: Some(1_000),
                ..provider("First")
            },
        )
        .await?;
    assert_eq!(inserted.kind, "llm");
    assert!(inserted.supports_images, "column default");
    assert_eq!(inserted.temperature, 0.2);
    assert!(!inserted.is_default);
    assert_eq!(inserted.created_at, 1_000);
    assert_eq!(
        inserted.session_token.as_deref(),
        Some(""),
        "'' is inserted"
    );
    assert!(!inserted.has_session_token(), "but reads as unset");
    assert!(inserted.has_api_key());
    assert!(!format!("{inserted:?}").contains("sk-1"), "Debug redacts");

    store.set_default("p1").await?;
    let updated = store
        .upsert_llm(
            "p1",
            UpsertProvider {
                api_key: Some(Some(String::new())),
                base_url: Some(Some("https://x".into())),
                created_at: Some(5),
                temperature: Some(0.9),
                ..provider("Renamed")
            },
        )
        .await?;
    assert_eq!(updated.name, "Renamed");
    assert_eq!(updated.api_key.as_deref(), Some("sk-1"), "'' keeps the key");
    assert_eq!(updated.created_at, 1_000, "created_at survives");
    assert!(updated.is_default, "a save keeps the selection");
    assert_eq!(updated.headers, inserted.headers, "absent keeps headers");
    assert_eq!(updated.temperature, 0.9);

    let cleared = store
        .upsert_llm(
            "p1",
            UpsertProvider {
                api_key: Some(None),
                headers: Some(None),
                ..provider("Renamed")
            },
        )
        .await?;
    assert_eq!(cleared.api_key, None, "null clears");
    assert_eq!(cleared.headers, None);
    assert_eq!(cleared.base_url.as_deref(), Some("https://x"));
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn providers_keep_rowid_order_and_one_default() -> anyhow::Result<()> {
    let (_dir, paths) = stage(None)?;
    let (db, _) = open(&paths).await?;
    let store = db.providers();
    for id in ["b", "a", "c"] {
        store.upsert_llm(id, provider(id)).await?;
    }
    store.upsert_llm("b", provider("b2")).await?;
    let agent = db
        .agents()
        .create(CreateAgent {
            kind: AgentKind::Codex,
            name: "codex".into(),
            model_id: None,
            reasoning_effort: None,
            working_directory: None,
            custom_config: None,
        })
        .await?;
    let ids: Vec<String> = store.list().await?.into_iter().map(|row| row.id).collect();
    assert_eq!(ids, ["b", "a", "c", agent.id.to_string().as_str()]);
    assert_eq!(store.list_llm().await?.len(), 3);

    assert!(store.default().await?.is_none());
    assert_eq!(
        store.set_default("a").await?.map(|row| row.id).as_deref(),
        Some("a")
    );
    let agent_id = agent.id.to_string();
    let selected = store.set_default(&agent_id).await?;
    assert_eq!(
        selected.map(|row| row.kind).as_deref(),
        Some("acp"),
        "any kind"
    );
    assert!(store.set_default("missing").await?.is_none());
    assert_eq!(store.default().await?.map(|row| row.id), Some(agent_id));
    let defaults = store
        .list()
        .await?
        .iter()
        .filter(|row| row.is_default)
        .count();
    assert_eq!(defaults, 1);

    db.schedules()
        .upsert_job(
            "job",
            UpsertJob {
                provider_id: Some(Some("c".into())),
                ..job("job")
            },
        )
        .await?;
    assert!(store.delete("c").await?);
    assert!(!store.delete("c").await?);
    let job = db.schedules().get_job("job").await?;
    assert_eq!(
        job.and_then(|job| job.provider_id),
        None,
        "ON DELETE SET NULL"
    );
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn agents_trim_order_and_update_field_by_field() -> anyhow::Result<()> {
    let (_dir, paths) = stage(None)?;
    let (db, _) = open(&paths).await?;
    let store = db.agents();
    let claude = store
        .create(CreateAgent {
            kind: AgentKind::Claude,
            name: "  Claude  ".into(),
            model_id: Some(" opus ".into()),
            reasoning_effort: Some("   ".into()),
            working_directory: Some(PathBuf::from(" /tmp/x ")),
            custom_config: None,
        })
        .await?;
    assert_eq!(claude.name, "Claude");
    assert_eq!(claude.model_id.as_deref(), Some("opus"));
    assert_eq!(claude.reasoning_effort, None, "blank becomes NULL");
    assert_eq!(claude.working_directory, Some(PathBuf::from("/tmp/x")));
    let custom = store
        .create(CreateAgent {
            kind: AgentKind::Custom,
            name: "Mine".into(),
            model_id: None,
            reasoning_effort: None,
            working_directory: None,
            custom_config: Some(r#"{"command":"x"}"#.into()),
        })
        .await?;
    let provider_row = db.providers().get(&custom.id.to_string()).await?;
    assert_eq!(
        provider_row.and_then(|row| row.custom_config).as_deref(),
        Some(r#"{"command":"x"}"#),
        "/providers sees the raw text"
    );

    tokio::time::sleep(std::time::Duration::from_millis(3)).await;
    let updated = store
        .update(
            claude.id,
            UpdateAgent {
                name: Some(" New ".into()),
                model_id: Some(None),
                ..UpdateAgent::default()
            },
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("agent missing"))?;
    assert_eq!(updated.name, "New");
    assert_eq!(updated.model_id, None, "null clears");
    assert_eq!(
        updated.working_directory, claude.working_directory,
        "absent keeps"
    );
    assert!(updated.updated_at > claude.updated_at);
    let order: Vec<Uuid> = store.list().await?.into_iter().map(|row| row.id).collect();
    assert_eq!(order, [claude.id, custom.id], "newest updated_at first");

    assert!(
        store
            .update(Uuid::new_v4(), UpdateAgent::default())
            .await?
            .is_none()
    );
    db.providers().upsert_llm("llm", provider("llm")).await?;
    let mut connection = raw(&paths.db_file).await?;
    sqlx::query(
        "INSERT INTO providers (id, kind, type, name, created_at, updated_at) \
         VALUES ('legacy', 'acp', 'gemini', 'Old', 1, 1)",
    )
    .execute(&mut connection)
    .await?;
    drop(connection);
    assert_eq!(
        store.list().await?.len(),
        2,
        "llm rows and unknown agents skipped"
    );
    assert!(store.delete(custom.id).await?);
    assert!(!store.delete(custom.id).await?);
    db.close().await?;
    Ok(())
}

fn ui_message(id: &str, role: &str, text: &str) -> anyhow::Result<UiMessage> {
    Ok(serde_json::from_value(json!({
        "id": id,
        "role": role,
        "parts": [{"type": "text", "text": text}],
    }))?)
}

#[tokio::test]
async fn conversations_save_import_and_list() -> anyhow::Result<()> {
    let (_dir, paths) = stage(None)?;
    let (db, _) = open(&paths).await?;
    let store = db.conversations();
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    store
        .save(SaveConversation {
            id: first,
            messages: vec![
                ui_message("u", "user", "  what is new?  ")?,
                ui_message("a", "assistant", "nothing")?,
            ],
            origin: Some("sidepanel".into()),
            target_type: TargetType::BrowserOs,
            agent_id: None,
        })
        .await?;
    let saved = store
        .get(first)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing"))?;
    assert_eq!(
        saved.summary.last_user_message.as_deref(),
        Some("what is new?")
    );
    assert_eq!(saved.ui_messages()?.len(), 2);

    let legacy = vec![
        json!({"id": "l", "role": "user", "parts": [{"type": "text", "text": "old"}], "legacy": true}),
        json!({"role": "assistant", "content": "free-form"}),
    ];
    let imported = store
        .import_if_absent(
            second,
            ImportConversation {
                messages: legacy.clone(),
                last_messaged_at: Some(10),
                target_type: TargetType::Codex,
                origin: None,
                agent_id: Some("agent".into()),
            },
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("not imported"))?;
    assert_eq!(imported.last_messaged_at, 10);
    assert_eq!(imported.last_user_message.as_deref(), Some("old"));
    let again = store
        .import_if_absent(
            second,
            ImportConversation {
                messages: Vec::new(),
                last_messaged_at: None,
                target_type: TargetType::BrowserOs,
                origin: None,
                agent_id: None,
            },
        )
        .await?;
    assert!(again.is_none(), "insert-if-absent, never an upsert");
    let stored = store
        .get(second)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing"))?;
    assert_eq!(stored.messages, legacy, "verbatim, unknown keys included");
    assert!(
        stored.ui_messages().is_err(),
        "not every import is a UIMessage"
    );

    let ids: Vec<Uuid> = store.list().await?.into_iter().map(|row| row.id).collect();
    assert_eq!(ids, [first, second], "newest last_messaged_at first");

    tokio::time::sleep(std::time::Duration::from_millis(3)).await;
    store
        .save(SaveConversation {
            id: first,
            messages: vec![ui_message("u", "user", "again")?],
            origin: None,
            target_type: TargetType::Claude,
            agent_id: Some("a1".into()),
        })
        .await?;
    let resaved = store
        .get(first)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing"))?;
    assert_eq!(resaved.summary.created_at, saved.summary.created_at);
    assert_eq!(resaved.summary.target_type, TargetType::Claude);
    assert_eq!(resaved.summary.origin, None, "a save replaces origin");
    assert!(store.delete(first).await?);
    assert!(!store.delete(first).await?);
    assert!(store.get(first).await?.is_none());
    db.close().await?;
    Ok(())
}

fn job(name: &str) -> UpsertJob {
    UpsertJob {
        name: name.into(),
        query: "q".into(),
        schedule_type: "hourly".into(),
        schedule_interval: Some(Some(2)),
        ..UpsertJob::default()
    }
}

fn run(job_id: &str, started_at: i64) -> UpsertRun {
    UpsertRun {
        job_id: job_id.into(),
        status: "completed".into(),
        started_at,
        ..UpsertRun::default()
    }
}

#[tokio::test]
async fn schedules_upsert_prune_and_cascade() -> anyhow::Result<()> {
    let (_dir, paths) = stage(None)?;
    let (db, _) = open(&paths).await?;
    let store = db.schedules();
    let created = store
        .upsert_job(
            "j",
            UpsertJob {
                last_run_at: Some(Some(7)),
                created_at: Some(3),
                ..job("Job")
            },
        )
        .await?;
    assert!(created.enabled, "column default");
    assert_eq!(created.created_at, 3);
    let updated = store
        .upsert_job(
            "j",
            UpsertJob {
                enabled: Some(false),
                schedule_interval: Some(None),
                created_at: Some(99),
                ..job("Job 2")
            },
        )
        .await?;
    assert_eq!(updated.last_run_at, Some(7), "absent keeps");
    assert_eq!(updated.schedule_interval, None, "null clears");
    assert!(!updated.enabled);
    assert_eq!(updated.created_at, 3);
    assert!(
        store
            .upsert_job(
                "bad",
                UpsertJob {
                    provider_id: Some(Some("nope".into())),
                    ..job("bad")
                }
            )
            .await
            .is_err(),
        "foreign key"
    );

    for index in 0..20 {
        store
            .upsert_run(&format!("r{index:02}"), run("j", 1_000 + index))
            .await?;
    }
    let runs = store.list_runs().await?;
    assert_eq!(runs.len(), 15, "pruned to the newest 15");
    assert_eq!(runs.first().map(|run| run.id.as_str()), Some("r19"));
    assert_eq!(runs.last().map(|run| run.id.as_str()), Some("r05"));

    let calls = json!([{"id": "t", "name": "navigate", "input": {"url": "x"}, "timestamp": "now"}]);
    let saved = store
        .upsert_run(
            "r19",
            UpsertRun {
                tool_calls: Some(Some(calls.clone())),
                error: Some(Some("boom".into())),
                ..run("j", 1_019)
            },
        )
        .await?;
    assert_eq!(saved.tool_calls, Some(calls));
    assert!(store.upsert_run("orphan", run("missing", 1)).await.is_err());
    assert!(store.delete_run("r19").await?);
    assert!(store.delete_job("j").await?);
    assert!(store.list_runs().await?.is_empty(), "ON DELETE CASCADE");
    assert!(!store.delete_job("j").await?);
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn oauth_tokens_round_trip() -> anyhow::Result<()> {
    let (_dir, paths) = stage(None)?;
    let (db, _) = open(&paths).await?;
    let store = db.oauth_tokens();
    let tokens = OAuthTokens {
        access_token: "a".into(),
        refresh_token: "r".into(),
        expires_at: 5,
        email: Some("e@x".into()),
        account_id: None,
    };
    store.upsert("install", "chatgpt-pro", &tokens).await?;
    assert_eq!(
        store.get("install", "chatgpt-pro").await?,
        Some(tokens.clone())
    );
    let refreshed = OAuthTokens {
        access_token: "b".into(),
        email: None,
        ..tokens
    };
    store.upsert("install", "chatgpt-pro", &refreshed).await?;
    assert_eq!(store.get("install", "chatgpt-pro").await?, Some(refreshed));
    assert!(store.get("other-install", "chatgpt-pro").await?.is_none());
    store.delete("install", "chatgpt-pro").await?;
    assert!(store.get("install", "chatgpt-pro").await?.is_none());
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn rows_written_with_odd_storage_classes_still_read() -> anyhow::Result<()> {
    let (_dir, paths) = stage(None)?;
    let (db, _) = open(&paths).await?;
    db.providers().upsert_llm("p", provider("p")).await?;
    db.schedules().upsert_job("j", job("j")).await?;
    let mut connection = raw(&paths.db_file).await?;
    for statement in [
        "UPDATE providers SET context_window = 1.5, headers = 'not json', is_default = 2",
        "UPDATE scheduled_jobs SET schedule_interval = 2.5",
    ] {
        sqlx::query(statement).execute(&mut connection).await?;
    }
    drop(connection);
    let row = db
        .providers()
        .get("p")
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing"))?;
    assert_eq!(row.context_window, Some(1));
    assert_eq!(row.headers, None);
    assert!(!row.is_default, "Number(2) === 1 is false");
    let jobs = db.schedules().list_jobs().await?;
    assert_eq!(jobs.first().and_then(|job| job.schedule_interval), Some(2));
    db.close().await?;
    Ok(())
}
