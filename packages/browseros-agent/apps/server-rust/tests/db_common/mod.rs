//! Shared support for the `db_*` integration tests: staging TS-written fixtures into a sandboxed
//! BrowserOS dir, reading a file through the Rust stores into exactly the JSON shape
//! `tools/server-compat/make-db-fixture.ts dump` produces from the TS stores, running that TS
//! dump, and comparing dumps without printing values (they hold credentials).

#![allow(dead_code)]

use anyhow::{Context, bail};
use sea_orm::sqlx::{self, Connection, Row, sqlite::SqliteConnectOptions};
use serde_json::{Map, Value, json};
use server_rust::{
    db::{
        AcpAgentRow, Database, MigrationOutcome, ProviderRow, ScheduledJobRow, ScheduledRunRow,
        StoredConversation,
    },
    paths::BrowserosPaths,
};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use tempfile::TempDir;

pub fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn fixture(relative: &str) -> PathBuf {
    manifest_dir().join("tests/fixtures/ts-db").join(relative)
}

pub fn read_json(path: &Path) -> anyhow::Result<Value> {
    let text = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    Ok(serde_json::from_str(&text)?)
}

/// A sandboxed BrowserOS dir; with `fixture`, its `browseros.sqlite` is copied into place.
pub fn stage(fixture_dir: Option<&str>) -> anyhow::Result<(TempDir, BrowserosPaths)> {
    let dir = tempfile::tempdir()?;
    let paths = BrowserosPaths::new(dir.path().to_path_buf());
    if let Some(name) = fixture_dir {
        std::fs::create_dir_all(paths.db_file.parent().context("db dir")?)?;
        std::fs::copy(fixture(&format!("{name}/browseros.sqlite")), &paths.db_file)?;
    }
    Ok((dir, paths))
}

pub async fn open(paths: &BrowserosPaths) -> anyhow::Result<(Database, MigrationOutcome)> {
    Ok(Database::open(paths).await?)
}

/// A plain sqlx connection beside the store's, for raw assertions and fixture surgery.
pub async fn raw(db_file: &Path) -> anyhow::Result<sqlx::SqliteConnection> {
    Ok(
        sqlx::SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(db_file))
            .await?,
    )
}

pub async fn count(db_file: &Path, table: &str) -> anyhow::Result<i64> {
    let mut connection = raw(db_file).await?;
    let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(&mut connection)
        .await?;
    Ok(count)
}

async fn raw_dump(db_file: &Path) -> anyhow::Result<(Value, Value, Vec<(String, String)>)> {
    let mut connection = raw(db_file).await?;
    let migrations =
        sqlx::query("SELECT hash, created_at FROM __drizzle_migrations ORDER BY created_at")
            .fetch_all(&mut connection)
            .await?
            .iter()
            .map(|row| {
                Ok(json!({
                    "hash": row.try_get::<String, _>("hash")?,
                    "created_at": row.try_get::<i64, _>("created_at")?,
                }))
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
    let schema = sqlx::query(
        "SELECT type, name, tbl_name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
    )
    .fetch_all(&mut connection)
    .await?
    .iter()
    .map(|row| {
        Ok(json!({
            "type": row.try_get::<String, _>("type")?,
            "name": row.try_get::<String, _>("name")?,
            "tbl_name": row.try_get::<String, _>("tbl_name")?,
            "sql": row.try_get::<Option<String>, _>("sql")?,
        }))
    })
    .collect::<Result<Vec<_>, sqlx::Error>>()?;
    let keys = sqlx::query(
        "SELECT browseros_id, provider FROM oauth_tokens ORDER BY browseros_id, provider",
    )
    .fetch_all(&mut connection)
    .await?
    .iter()
    .map(|row| Ok((row.try_get("browseros_id")?, row.try_get("provider")?)))
    .collect::<Result<Vec<_>, sqlx::Error>>()?;
    Ok((Value::Array(migrations), Value::Array(schema), keys))
}

/// Everything the Rust stores read from `db`, shaped like the TS dump.
pub async fn rust_dump(db: &Database, db_file: &Path) -> anyhow::Result<Value> {
    let (drizzle_migrations, schema, oauth_keys) = raw_dump(db_file).await?;
    let providers = db.providers().list().await?;
    let summaries = db.conversations().list().await?;
    let mut conversations = Vec::new();
    for summary in &summaries {
        let detail = db
            .conversations()
            .get(summary.id)
            .await?
            .context("listed conversation missing")?;
        conversations.push(conversation_json(&detail));
    }
    let mut oauth = Vec::new();
    for (browseros_id, provider) in oauth_keys {
        let tokens = db.oauth_tokens().get(&browseros_id, &provider).await?;
        let mut status = Map::new();
        status.insert("authenticated".into(), json!(tokens.is_some()));
        if let Some(email) = tokens.as_ref().and_then(|tokens| tokens.email.clone()) {
            status.insert("email".into(), json!(email));
        }
        status.insert("provider".into(), json!(provider));
        let tokens = tokens.map(|tokens| {
            let mut value = Map::new();
            value.insert("accessToken".into(), json!(tokens.access_token));
            value.insert("refreshToken".into(), json!(tokens.refresh_token));
            value.insert("expiresAt".into(), json!(tokens.expires_at));
            insert_some(&mut value, "email", tokens.email);
            insert_some(&mut value, "accountId", tokens.account_id);
            Value::Object(value)
        });
        oauth.push(json!({
            "browserosId": browseros_id,
            "provider": provider,
            "tokens": tokens,
            "status": status,
        }));
    }
    Ok(json!({
        "drizzleMigrations": drizzle_migrations,
        "schema": schema,
        "providers": providers.iter().map(provider_json).collect::<Vec<_>>(),
        "publicProviders": providers.iter().map(public_provider_json).collect::<Vec<_>>(),
        "defaultProviderId": db.providers().default().await?.map(|row| row.id),
        "agents": db.agents().list().await?.iter().map(agent_json).collect::<Vec<_>>(),
        "conversationSummaries": conversations
            .iter()
            .map(|detail| {
                let mut summary = detail.clone();
                if let Some(object) = summary.as_object_mut() {
                    object.remove("messages");
                }
                summary
            })
            .collect::<Vec<_>>(),
        "conversations": conversations,
        "jobs": db.schedules().list_jobs().await?.iter().map(job_json).collect::<Vec<_>>(),
        "runs": db.schedules().list_runs().await?.iter().map(run_json).collect::<Vec<_>>(),
        "oauthTokens": oauth,
    }))
}

pub fn provider_json(row: &ProviderRow) -> Value {
    let mut value = public_provider_json(row);
    if let Some(object) = value.as_object_mut() {
        for key in [
            "hasApiKey",
            "hasAccessKeyId",
            "hasSecretAccessKey",
            "hasSessionToken",
        ] {
            object.remove(key);
        }
        object.insert("apiKey".into(), json!(row.api_key));
        object.insert("accessKeyId".into(), json!(row.access_key_id));
        object.insert("secretAccessKey".into(), json!(row.secret_access_key));
        object.insert("sessionToken".into(), json!(row.session_token));
    }
    value
}

pub fn public_provider_json(row: &ProviderRow) -> Value {
    json!({
        "id": row.id,
        "kind": row.kind,
        "type": row.provider_type,
        "name": row.name,
        "modelId": row.model_id,
        "reasoningEffort": row.reasoning_effort,
        "isDefault": row.is_default,
        "createdAt": row.created_at,
        "updatedAt": row.updated_at,
        "baseUrl": row.base_url,
        "headers": row.headers,
        "supportsImages": row.supports_images,
        "contextWindow": row.context_window,
        "temperature": row.temperature,
        "resourceName": row.resource_name,
        "region": row.region,
        "reasoningSummary": row.reasoning_summary,
        "workingDirectory": row.working_directory,
        "customConfig": row.custom_config,
        "hasApiKey": row.has_api_key(),
        "hasAccessKeyId": row.has_access_key_id(),
        "hasSecretAccessKey": row.has_secret_access_key(),
        "hasSessionToken": row.has_session_token(),
    })
}

/// `AcpAgentDefinition`: optional fields omitted when NULL, `customConfig` parsed.
pub fn agent_json(row: &AcpAgentRow) -> Value {
    let mut value = Map::new();
    value.insert("id".into(), json!(row.id.to_string()));
    value.insert("name".into(), json!(row.name));
    value.insert("type".into(), json!(row.kind.as_str()));
    insert_some(&mut value, "modelId", row.model_id.clone());
    insert_some(&mut value, "reasoningEffort", row.reasoning_effort.clone());
    insert_some(
        &mut value,
        "workingDirectory",
        row.working_directory
            .as_ref()
            .map(|path| path.display().to_string()),
    );
    if let Some(config) = row
        .custom_config
        .as_deref()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
    {
        value.insert("customConfig".into(), config);
    }
    value.insert("createdAt".into(), json!(row.created_at));
    value.insert("updatedAt".into(), json!(row.updated_at));
    Value::Object(value)
}

pub fn conversation_json(detail: &StoredConversation) -> Value {
    let summary = &detail.summary;
    let mut value = Map::new();
    value.insert("id".into(), json!(summary.id.to_string()));
    insert_some(
        &mut value,
        "lastUserMessage",
        summary.last_user_message.clone(),
    );
    insert_some(&mut value, "origin", summary.origin.clone());
    value.insert("targetType".into(), json!(summary.target_type.as_str()));
    insert_some(&mut value, "agentId", summary.agent_id.clone());
    value.insert("lastMessagedAt".into(), json!(summary.last_messaged_at));
    value.insert("createdAt".into(), json!(summary.created_at));
    value.insert("updatedAt".into(), json!(summary.updated_at));
    value.insert("messages".into(), Value::Array(detail.messages.clone()));
    Value::Object(value)
}

pub fn job_json(row: &ScheduledJobRow) -> Value {
    json!({
        "id": row.id,
        "name": row.name,
        "query": row.query,
        "scheduleType": row.schedule_type,
        "scheduleTime": row.schedule_time,
        "scheduleInterval": row.schedule_interval,
        "enabled": row.enabled,
        "providerId": row.provider_id,
        "lastRunAt": row.last_run_at,
        "createdAt": row.created_at,
        "updatedAt": row.updated_at,
    })
}

pub fn run_json(row: &ScheduledRunRow) -> Value {
    json!({
        "id": row.id,
        "jobId": row.job_id,
        "status": row.status,
        "startedAt": row.started_at,
        "completedAt": row.completed_at,
        "result": row.result,
        "finalResult": row.final_result,
        "executionLog": row.execution_log,
        "toolCalls": row.tool_calls,
        "error": row.error,
        "createdAt": row.created_at,
        "updatedAt": row.updated_at,
    })
}

fn insert_some(object: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        object.insert(key.into(), json!(value));
    }
}

/// `schema` without the Rust ledger's table, for comparing against a TS-only file.
pub fn without_rust_ledger(dump: &Value) -> Value {
    let mut dump = dump.clone();
    if let Some(schema) = dump.get_mut("schema").and_then(Value::as_array_mut) {
        schema.retain(|object| object["tbl_name"] != "seaql_migrations");
    }
    dump
}

/// Runs the TS server's own boot path (`initializeDb`, which migrates) and stores over
/// `db_file`. `None` when Bun or the workspace's node_modules are unavailable, unless
/// `BROWSEROS_REQUIRE_TS_ROUNDTRIP=1` makes that a failure.
pub fn ts_dump(db_file: &Path) -> anyhow::Result<Option<Value>> {
    let workspace = manifest_dir().join("../..");
    let required = std::env::var("BROWSEROS_REQUIRE_TS_ROUNDTRIP").as_deref() == Ok("1");
    if !workspace.join("node_modules").is_dir() {
        if required {
            bail!("node_modules missing; run bun install");
        }
        note_skip("node_modules missing");
        return Ok(None);
    }
    let out = db_file.with_extension("ts-dump.json");
    let output = match Command::new("bun")
        .arg("tools/server-compat/make-db-fixture.ts")
        .arg("dump")
        .arg(db_file)
        .arg(&out)
        .current_dir(&workspace)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !required => {
            note_skip("bun not on PATH");
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
    };
    if !output.status.success() {
        bail!(
            "TS dump failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(Some(read_json(&out)?))
}

#[allow(clippy::print_stderr)]
fn note_skip(reason: &str) {
    eprintln!("skipping the TS leg of the round trip: {reason}");
}

/// Fails listing the JSON paths where `actual` and `expected` differ (never the values).
/// Numbers compare as f64, the only number JS has.
pub fn assert_same(actual: &Value, expected: &Value, context: &str) {
    let mut differences = Vec::new();
    diff(actual, expected, "$", &mut differences);
    assert!(
        differences.is_empty(),
        "{context}: {} difference(s) at {}",
        differences.len(),
        differences
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    );
}

fn diff(actual: &Value, expected: &Value, path: &str, out: &mut Vec<String>) {
    match (actual, expected) {
        (Value::Object(actual), Value::Object(expected)) => {
            for key in actual
                .keys()
                .chain(expected.keys().filter(|key| !actual.contains_key(*key)))
            {
                let child = format!("{path}.{key}");
                match (actual.get(key), expected.get(key)) {
                    (Some(actual), Some(expected)) => diff(actual, expected, &child, out),
                    (Some(_), None) => out.push(format!("{child} (only in actual)")),
                    (None, _) => out.push(format!("{child} (only in expected)")),
                }
            }
        }
        (Value::Array(actual), Value::Array(expected)) => {
            if actual.len() != expected.len() {
                out.push(format!(
                    "{path} (length {} vs {})",
                    actual.len(),
                    expected.len()
                ));
            }
            for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                diff(actual, expected, &format!("{path}[{index}]"), out);
            }
        }
        (Value::Number(actual), Value::Number(expected)) => {
            if actual.as_f64() != expected.as_f64() {
                out.push(path.to_owned());
            }
        }
        (actual, expected) => {
            if actual != expected {
                out.push(path.to_owned());
            }
        }
    }
}
