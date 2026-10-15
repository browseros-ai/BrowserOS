//! Session record persistence and the one-way `acpx.session.v1` importer, against a fixture shaped
//! exactly like acpx 0.13.2/0.19.3 writes it.

use std::path::PathBuf;

use acp_agents::{
    FileSessionStore, IMPORTED_LAUNCH_FINGERPRINT, SESSION_RECORD_SCHEMA, SessionKey,
    SessionRecord, SessionStore,
};
use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn key() -> SessionKey {
    SessionKey::new(Uuid::from_u128(7), Uuid::from_u128(8))
}

const ENCODED: &str =
    "acp%3A00000000-0000-0000-0000-000000000007%3A00000000-0000-0000-0000-000000000008.json";

fn record() -> SessionRecord {
    SessionRecord {
        schema: SESSION_RECORD_SCHEMA.into(),
        session_key: key().as_str().into(),
        agent_session_id: "native-session".into(),
        launch_fingerprint: "abc".into(),
        cwd: PathBuf::from("/work"),
        has_user_turn: true,
        reset_on_next_ensure: false,
        updated_at_ms: 42,
    }
}

fn write_acpx(dir: &TempDir, messages: serde_json::Value, reset: bool) -> TestResult {
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions)?;
    let fixture = json!({
        "schema":"acpx.session.v1",
        "acpx_record_id":key().as_str(),
        "acp_session_id":"0199-acp-session",
        "agent_session_id":"agent-internal",
        "agent_command":"env PATH=/x bun x --bun --silent --package @agentclientprotocol/claude-agent-acp@^0.75.1 claude-agent-acp",
        "agent_argv":["env", "PATH=/x", "bun", "x"],
        "cwd":"/Users/me/.browseros/agents/acp-workspace",
        "created_at":"2026-10-01T00:00:00.000Z",
        "last_used_at":"2026-10-02T00:00:00.000Z",
        "last_seq":3,
        "messages":messages,
        "acpx":{"reset_on_next_ensure":reset, "desired_mode_id":"bypassPermissions"},
        "cumulative_token_usage":{},
    });
    std::fs::write(sessions.join(ENCODED), serde_json::to_vec(&fixture)?)?;
    Ok(())
}

#[test]
fn records_round_trip_atomically_under_encoded_names() -> TestResult {
    let dir = TempDir::new()?;
    let store = FileSessionStore::new(dir.path().into());
    assert_eq!(store.load(&key())?, None);
    store.save(&record())?;
    assert_eq!(store.load(&key())?, Some(record()));
    let files: Vec<String> = std::fs::read_dir(dir.path().join("browseros-sessions"))?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()?;
    assert_eq!(files, [ENCODED], "no temp file may be left behind");
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(
        dir.path().join("browseros-sessions").join(ENCODED),
    )?)?;
    assert_eq!(saved["schema"], "browseros.acp-session.v1");
    assert_eq!(saved["agentSessionId"], "native-session");
    assert_eq!(saved["hasUserTurn"], true);
    Ok(())
}

#[test]
fn acpx_records_import_one_way() -> TestResult {
    let dir = TempDir::new()?;
    let store = FileSessionStore::new(dir.path().into());
    write_acpx(
        &dir,
        json!(["Resume", {"Agent":{"content":[]}}, {"User":{"id":"u1", "content":[{"Text":"hi"}]}}]),
        false,
    )?;
    let imported = store.load(&key())?.ok_or("import")?;
    assert_eq!(
        imported,
        SessionRecord {
            schema: SESSION_RECORD_SCHEMA.into(),
            session_key: key().as_str().into(),
            agent_session_id: "0199-acp-session".into(),
            launch_fingerprint: IMPORTED_LAUNCH_FINGERPRINT.into(),
            cwd: PathBuf::from("/Users/me/.browseros/agents/acp-workspace"),
            has_user_turn: true,
            reset_on_next_ensure: false,
            updated_at_ms: 0,
        }
    );
    assert_eq!(store.import_acpx(&key())?, Some(imported));
    assert!(
        !dir.path().join("browseros-sessions").exists(),
        "importing never writes"
    );
    Ok(())
}

#[test]
fn acpx_records_without_user_turns_or_marked_reset_import_as_such() -> TestResult {
    let dir = TempDir::new()?;
    let store = FileSessionStore::new(dir.path().into());
    write_acpx(&dir, json!([{"Agent":{"content":[]}}]), true)?;
    let imported = store.import_acpx(&key())?.ok_or("import")?;
    assert!(!imported.has_user_turn);
    assert!(imported.reset_on_next_ensure);
    Ok(())
}

#[test]
fn native_records_win_and_delete_removes_both() -> TestResult {
    let dir = TempDir::new()?;
    let store = FileSessionStore::new(dir.path().into());
    write_acpx(&dir, json!([]), false)?;
    store.save(&record())?;
    assert_eq!(store.load(&key())?, Some(record()));
    store.delete(&key())?;
    assert_eq!(
        store.load(&key())?,
        None,
        "a discarded session must not re-import"
    );
    store.delete(&key())?;
    Ok(())
}

#[test]
fn unreadable_or_foreign_records_load_as_missing() -> TestResult {
    let dir = TempDir::new()?;
    let store = FileSessionStore::new(dir.path().into());
    let native = dir.path().join("browseros-sessions");
    std::fs::create_dir_all(&native)?;
    std::fs::write(native.join(ENCODED), b"{not json")?;
    assert_eq!(store.load(&key())?, None);
    let mut other = record();
    other.session_key = "acp:someone:else".into();
    std::fs::write(native.join(ENCODED), serde_json::to_vec(&other)?)?;
    assert_eq!(store.load(&key())?, None);
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions)?;
    std::fs::write(sessions.join(ENCODED), br#"{"schema":"acpx.session.v2"}"#)?;
    assert_eq!(store.import_acpx(&key())?, None);
    Ok(())
}
