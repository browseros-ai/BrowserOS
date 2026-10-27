//! The scope file: what survives, what is discarded, and what two processes do.

use std::path::Path;

use claw_server::cli::state::{Scope, State, Store, scope, scope_key, scopes_in, worktree_root};

fn scope_named(label: &str) -> Scope {
    scope(Path::new(label), None)
}

#[test]
fn a_round_trip_keeps_every_field() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let store = Store::open_in(dir.path(), &scope_named("/repo"))?;

    store.update(|state| {
        state.endpoint = Some("http://127.0.0.1:9200".into());
        state.mcp_session_id = Some("sess".into());
        state.group_id = Some("GROUP".into());
        state.task = Some("price check".into());
        state.page = Some(3);
        state.page_url = Some("https://x.test".into());
    })?;

    let read = store.load();
    assert_eq!(read.page, Some(3));
    assert_eq!(read.group_id.as_deref(), Some("GROUP"));
    assert_eq!(read.page_url.as_deref(), Some("https://x.test"));
    // The scope path is filled in on first write so `session list` can print it.
    assert_eq!(read.scope_path.as_deref(), Some("/repo"));
    Ok(())
}

#[test]
fn a_corrupt_file_is_replaced_rather_than_fatal() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let scope = scope_named("/repo");
    let store = Store::open_in(dir.path(), &scope)?;
    std::fs::write(
        dir.path().join(format!("{}.json", scope_key(&scope))),
        "{ this is not json",
    )?;

    // It is a cache of three recoverable facts. Refusing to run because of it
    // would turn a cosmetic problem into an outage.
    let read = store.load();
    assert_eq!(read.page, None);
    store.update(|state| state.page = Some(9))?;
    assert_eq!(store.load().page, Some(9));
    Ok(())
}

#[test]
fn a_changed_endpoint_discards_what_cannot_survive_it() {
    let mut state = State {
        endpoint: Some("http://127.0.0.1:9200".into()),
        mcp_session_id: Some("sess".into()),
        page: Some(3),
        page_url: Some("https://x.test".into()),
        group_id: Some("GROUP".into()),
        task: Some("price check".into()),
        tools: vec!["tabs".into()],
        server_version: Some("1.0.0".into()),
        ..State::default()
    };
    state.rebind("http://127.0.0.1:9739");

    // Meaningless on another server.
    assert_eq!(state.mcp_session_id, None);
    assert_eq!(state.page, None);
    assert_eq!(state.page_url, None);
    assert!(state.tools.is_empty());
    assert_eq!(state.server_version, None);
    // The server-independent half of the identity, which a reconnect reclaims.
    assert_eq!(state.group_id.as_deref(), Some("GROUP"));
    assert_eq!(state.task.as_deref(), Some("price check"));
}

#[test]
fn a_second_writer_does_not_silently_revert_the_first() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let scope = scope_named("/repo");
    let one = Store::open_in(dir.path(), &scope)?;
    let two = Store::open_in(dir.path(), &scope)?;

    one.update(|state| state.page = Some(1))?;
    two.update(|state| state.task = Some("second".into()))?;

    // The lock makes the second write a read-modify-write rather than an
    // overwrite, so the page set by the first process survives.
    let read = one.load();
    assert_eq!(read.page, Some(1));
    assert_eq!(read.task.as_deref(), Some("second"));
    Ok(())
}

#[test]
fn a_stale_lock_left_by_a_dead_process_is_stolen_rather_than_waited_on() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let scope = scope_named("/repo");
    let store = Store::open_in(dir.path(), &scope)?;
    let lock = dir.path().join(format!("{}.lock", scope_key(&scope)));
    std::fs::write(&lock, "")?;
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
    filetime(&lock, old)?;

    let started = std::time::Instant::now();
    store.update(|state| state.page = Some(4))?;
    assert_eq!(store.load().page, Some(4));
    // Stolen, not waited out: a crashed process must never make the browser
    // unusable for longer than the staleness window.
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "took {:?}",
        started.elapsed()
    );
    Ok(())
}

#[test]
fn two_directories_never_share_a_file_and_a_subdirectory_shares_its_root() {
    let a = scope(Path::new("/one"), None);
    let b = scope(Path::new("/two"), None);
    assert_ne!(scope_key(&a), scope_key(&b));

    let root = Path::new("/repo");
    let deep = scope(Path::new("/repo/apps/web"), Some(root));
    let shallow = scope(root, Some(root));
    assert_eq!(
        scope_key(&deep),
        scope_key(&shallow),
        "an agent that cds into a subdirectory must keep the same session"
    );
}

#[test]
fn listing_finds_every_scope_and_ignores_anything_else() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    Store::open_in(dir.path(), &scope_named("/one"))?.update(|s| s.task = Some("one".into()))?;
    Store::open_in(dir.path(), &scope_named("/two"))?.update(|s| s.task = Some("two".into()))?;
    std::fs::write(dir.path().join("notes.txt"), "ignore me")?;
    std::fs::write(dir.path().join("broken.json"), "{")?;

    let found = scopes_in(dir.path());
    assert_eq!(found.len(), 2, "{found:?}");
    let tasks: Vec<_> = found.values().filter_map(|s| s.task.clone()).collect();
    assert!(tasks.contains(&"one".to_string()));
    assert!(tasks.contains(&"two".to_string()));
    Ok(())
}

#[test]
fn clearing_a_scope_that_is_already_gone_is_not_an_error() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let store = Store::open_in(dir.path(), &scope_named("/repo"))?;
    store.clear()?;
    store.update(|state| state.page = Some(2))?;
    store.clear()?;
    assert_eq!(store.load().page, None);
    Ok(())
}

#[test]
fn an_unasked_tool_cache_never_blocks_the_first_command() {
    let mut state = State::default();
    assert!(state.offers("pursue"), "an empty cache means not yet asked");
    state.tools = vec!["tabs".into(), "run".into()];
    assert!(state.offers("run"));
    assert!(!state.offers("pursue"));
}

#[test]
fn the_worktree_root_is_found_from_inside_a_repository() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let status = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status();
    if !status.map(|status| status.success()).unwrap_or(false) {
        return Ok(()); // no git on this machine; the fallback path is tested above
    }
    let nested = dir.path().join("apps").join("web");
    std::fs::create_dir_all(&nested)?;
    let found = worktree_root(&nested);
    assert!(
        found.is_some(),
        "a nested directory should resolve to the root"
    );
    Ok(())
}

/// Backdates a file, so the stale-lock path can be exercised without sleeping.
fn filetime(path: &Path, when: std::time::SystemTime) -> anyhow::Result<()> {
    let file = std::fs::File::options().write(true).open(path)?;
    file.set_times(std::fs::FileTimes::new().set_modified(when))?;
    Ok(())
}
