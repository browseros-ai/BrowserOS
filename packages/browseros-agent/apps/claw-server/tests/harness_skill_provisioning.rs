//! Runs the production skill policy in a disposable process identity so discovery
//! and filesystem writes cannot read or change the developer's agent configuration.
use std::{
    env, fs,
    future::Future,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Duration,
};

use claw_server::{
    analytics::NoopAnalyticsSink,
    services::harness::{Harness, HarnessService},
};
use harness_integrations::{AgentId, SkillSpec, is_installed};

const CHILD: &str = "NEO_PROVISIONING_TEST_HOME";

#[test]
fn provisions_common_skills_without_apps_or_mcp_connections() -> anyhow::Result<()> {
    isolated(
        "provisions_common_skills_without_apps_or_mcp_connections",
        None,
        provision_fresh_home,
    )
}

fn isolated<F: Future<Output = anyhow::Result<()>>>(
    name: &str,
    claude_profile: Option<&str>,
    run: impl FnOnce(PathBuf) -> F,
) -> anyhow::Result<()> {
    if let Some(home) = env::var_os(CHILD) {
        return tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(run(PathBuf::from(home)));
    }
    let root = tempfile::tempdir()?;
    let output = Command::new(env::current_exe()?)
        .args(["--exact", name, "--nocapture"])
        .env_clear()
        .env(CHILD, root.path())
        .env("HOME", root.path())
        .env("USERPROFILE", root.path())
        .envs(claude_profile.map(|relative| ("CLAUDE_CONFIG_DIR", root.path().join(relative))))
        .env("PATH", "")
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "fixture failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
fn nested_provisioned_profiles_are_not_installed_apps() -> anyhow::Result<()> {
    isolated(
        "nested_provisioned_profiles_are_not_installed_apps",
        Some(".claude/profiles/work"),
        check_nested_profile_detection,
    )
}

#[cfg(unix)]
#[test]
fn linked_provisioned_profiles_are_not_installed_apps() -> anyhow::Result<()> {
    isolated(
        "linked_provisioned_profiles_are_not_installed_apps",
        Some(".claude/profiles/work"),
        |home| async move {
            let profiles = home.join("external-profiles");
            fs::create_dir_all(&profiles)?;
            fs::create_dir_all(home.join(".claude"))?;
            std::os::unix::fs::symlink(&profiles, home.join(".claude/profiles"))?;
            // A profile alias may loop back to an ancestor. Discovery must not
            // recurse forever when following canonical provisioning ancestors.
            std::os::unix::fs::symlink(&profiles, profiles.join("self"))?;
            check_nested_profile_detection(home).await
        },
    )
}

async fn check_nested_profile_detection(home: PathBuf) -> anyhow::Result<()> {
    let service = HarnessService::new_with_managed_skill(
        home.join("state/mcp"),
        home.join("state/skills"),
        home.clone(),
        SkillSpec::new("browseros-neo", "nested profile instructions")?,
        Arc::new(NoopAnalyticsSink),
    );
    service.run_skill_reconciliation().await?;
    assert!(
        home.join(".claude/profiles/work/skills/browseros-neo/SKILL.md")
            .is_file()
    );
    assert!(!is_installed(&[AgentId::ClaudeCode])?[&AgentId::ClaudeCode]);
    assert!(
        !service
            .connect_browseros(Harness::ClaudeCode, "http://127.0.0.1:9200/mcp")
            .await?
            .installed
    );
    assert!(!home.join(".claude/profiles/work/.claude.json").exists());
    fs::write(home.join(".claude/profiles/work/settings.json"), "{}")?;
    assert!(is_installed(&[AgentId::ClaudeCode])?[&AgentId::ClaudeCode]);
    assert!(
        service
            .connect_browseros(Harness::ClaudeCode, "http://127.0.0.1:9200/mcp")
            .await?
            .installed
    );
    Ok(())
}

#[test]
fn mcp_discovery_failure_does_not_block_provisioning_or_delete_connected_skills()
-> anyhow::Result<()> {
    isolated(
        "mcp_discovery_failure_does_not_block_provisioning_or_delete_connected_skills",
        None,
        |home| async move {
            let state = home.join("state");
            let service = HarnessService::new_with_managed_skill(
                state.join("mcp"),
                state.join("skills"),
                home.clone(),
                SkillSpec::new("browseros-neo", "independent instructions")?,
                Arc::new(NoopAnalyticsSink),
            );
            fs::create_dir_all(home.join(".cursor"))?;
            assert!(
                service
                    .connect_browseros(Harness::Cursor, "http://127.0.0.1:9200/mcp")
                    .await?
                    .installed
            );
            fs::write(home.join(".claude/settings.json"), "{}")?;
            service
                .connect_browseros(Harness::ClaudeCode, "http://127.0.0.1:9200/mcp")
                .await?;
            let native = home.join(".cursor/skills/browseros-neo/SKILL.md");
            let native_before = fs::read(&native)?;
            let skill_manifest_path = state.join("skills/skills.json");
            let skill_manifest_before = fs::read(&skill_manifest_path)?;
            let manifest_path = state.join("mcp/manifest.json");
            let manifest_before = fs::read(&manifest_path)?;
            fs::write(&manifest_path, "{ broken MCP manifest")?;
            for root in [
                ".agents/skills/browseros-neo",
                ".claude/skills/browseros-neo",
            ] {
                fs::remove_dir_all(home.join(root))?;
            }
            let repaired = service.run_skill_reconciliation().await?;
            assert_eq!(repaired.installed, 2);
            assert!(!repaired.warnings.is_empty());
            assert_eq!(fs::read(&native)?, native_before);
            // Unknown connection state must retain ownership and last-known
            // consumers for native targets and shared baseline targets alike.
            assert_eq!(fs::read(&skill_manifest_path)?, skill_manifest_before);
            assert_eq!(fs::read_to_string(&manifest_path)?, "{ broken MCP manifest");
            for root in [
                ".agents/skills/browseros-neo/SKILL.md",
                ".claude/skills/browseros-neo/SKILL.md",
            ] {
                assert_eq!(
                    fs::read_to_string(home.join(root))?,
                    "independent instructions"
                );
            }
            fs::write(&manifest_path, manifest_before)?;
            service.disconnect_browseros(Harness::Cursor).await?;
            service.run_skill_reconciliation().await?;
            assert!(!native.exists());
            assert!(home.join(".agents/skills/browseros-neo/SKILL.md").exists());
            Ok(())
        },
    )
}

async fn provision_fresh_home(home: PathBuf) -> anyhow::Result<()> {
    let state = home.join(".browserclaw");
    let service = HarnessService::new_with_managed_skill(
        state.join("mcp-manager"),
        state.join("harness-integrations"),
        home.clone(),
        SkillSpec::new("browseros-neo", "managed instructions v1\n")?,
        Arc::new(NoopAnalyticsSink),
    );
    let result = service.run_skill_reconciliation().await?;
    for relative in [
        ".agents/skills/browseros-neo/SKILL.md",
        ".claude/skills/browseros-neo/SKILL.md",
    ] {
        anyhow::ensure!(
            home.join(relative).is_file(),
            "missing proactive skill: {relative}"
        );
        assert_eq!(
            fs::read_to_string(home.join(relative))?,
            "managed instructions v1\n"
        );
    }
    assert_eq!(result.installed, 2);
    assert!(result.warnings.is_empty());
    assert!(!home.join(".codex").exists());
    assert!(!home.join(".claude.json").exists());
    assert!(!home.join(".config/opencode").exists());
    assert!(!state.join("mcp-manager/manifest.json").exists());
    assert!(
        !is_installed(&[AgentId::ClaudeCode])?[&AgentId::ClaudeCode],
        "the provisioned Claude skill directory is not an app installation"
    );
    // Both the connection list's probe and the config writer must reject this
    // false signal, including a native OpenCode skill root from another installer.
    fs::create_dir_all(home.join(".config/opencode/skills/browseros-neo"))?;
    assert!(!is_installed(&[AgentId::OpenCode])?[&AgentId::OpenCode]);
    for harness in [Harness::ClaudeCode, Harness::OpenCode] {
        assert!(
            !service
                .connect_browseros(harness, "http://127.0.0.1:9200/mcp")
                .await?
                .installed
        );
    }
    assert!(!home.join(".claude.json").exists());
    assert!(!home.join(".config/opencode/opencode.json").exists());

    // Once an app creates its own state, ordinary MCP setup still works. An
    // explicit disconnect persists through later skill-only repair passes.
    fs::write(home.join(".claude/settings.json"), "{}")?;
    assert!(is_installed(&[AgentId::ClaudeCode])?[&AgentId::ClaudeCode]);
    assert!(
        service
            .connect_browseros(Harness::ClaudeCode, "http://127.0.0.1:9200/mcp")
            .await?
            .installed
    );
    assert!(
        !service
            .disconnect_browseros(Harness::ClaudeCode)
            .await?
            .installed
    );
    service.run_skill_reconciliation().await?;
    assert!(!fs::read_to_string(home.join(".claude.json"))?.contains("browseros-neo"));
    assert!(!home.join(".claude/skills/browseros-neo/SKILL.md").exists());
    let reopened = HarnessService::new_with_managed_skill(
        state.join("mcp-manager"),
        state.join("harness-integrations"),
        home.clone(),
        SkillSpec::new("browseros-neo", "managed instructions v2\n")?,
        Arc::new(NoopAnalyticsSink),
    );
    reopened.run_skill_reconciliation().await?;
    assert!(!home.join(".claude/skills/browseros-neo/SKILL.md").exists());
    assert!(!fs::read_to_string(home.join(".claude.json"))?.contains("browseros-neo"));
    assert!(
        reopened
            .connect_browseros(Harness::ClaudeCode, "http://127.0.0.1:9200/mcp")
            .await?
            .installed
    );
    assert!(home.join(".claude/skills/browseros-neo/SKILL.md").exists());
    Ok(())
}

#[test]
fn background_worker_repairs_without_requests_and_stops_at_shutdown() -> anyhow::Result<()> {
    isolated(
        "background_worker_repairs_without_requests_and_stops_at_shutdown",
        None,
        |home| async move {
            tokio::time::pause();
            let service = Arc::new(HarnessService::new_with_managed_skill(
                home.join("state/mcp"),
                home.join("state/skills"),
                home.clone(),
                SkillSpec::new("browseros-neo", "background instructions\n")?,
                Arc::new(NoopAnalyticsSink),
            ));
            let (stop, stopped) = tokio::sync::oneshot::channel();
            let worker_service = service.clone();
            let worker = tokio::spawn(async move {
                worker_service
                    .maintain_integrations("http://127.0.0.1:9200/mcp", async {
                        let _ = stopped.await;
                    })
                    .await;
            });
            let shared = home.join(".agents/skills/browseros-neo/SKILL.md");
            let claude = home.join(".claude/skills/browseros-neo/SKILL.md");
            wait_for_skill(&shared).await?;
            wait_for_skill(&claude).await?;
            // Finish the in-flight pass before changing its result; this crosses the
            // same serialized interface used by Connect/Disconnect in production.
            service.run_skill_reconciliation().await?;
            fs::remove_file(&shared)?;
            fs::create_dir_all(home.join(".codex"))?;
            let codex = home.join(".codex/config.toml");
            tokio::time::advance(Duration::from_secs(60)).await;
            wait_for_skill(&shared).await?;
            wait_for_skill(&codex).await?;
            assert!(fs::read_to_string(&codex)?.contains(":9200/mcp"));
            service.disconnect_browseros(Harness::Codex).await?;
            tokio::time::advance(Duration::from_secs(60)).await;
            service
                .run_reconciliation("http://127.0.0.1:9200/mcp")
                .await?;
            assert!(!fs::read_to_string(&codex)?.contains("browseros-neo"));
            assert_eq!(fs::read_to_string(&shared)?, "background instructions\n");
            stop.send(())
                .map_err(|()| anyhow::anyhow!("worker stopped before shutdown"))?;
            worker.await?;
            fs::remove_file(&shared)?;
            tokio::time::advance(Duration::from_secs(120)).await;
            assert!(!shared.exists(), "shutdown must stop periodic repairs");
            assert!(!home.join(".claude.json").exists());
            assert!(!fs::read_to_string(&codex)?.contains("browseros-neo"));
            Ok(())
        },
    )
}

async fn wait_for_skill(path: &Path) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !path.is_file() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("background worker did not install {}", path.display()))
}

#[test]
fn packaged_skills_replace_existing_content_and_repair_missing_files() -> anyhow::Result<()> {
    isolated(
        "packaged_skills_replace_existing_content_and_repair_missing_files",
        None,
        |home| async move {
            let make = |content| -> anyhow::Result<HarnessService> {
                Ok(HarnessService::new_with_managed_skill(
                    home.join("state/mcp"),
                    home.join("state/skills"),
                    home.clone(),
                    SkillSpec::new("browseros-neo", content)?,
                    Arc::new(NoopAnalyticsSink),
                ))
            };
            let shared = home.join(".agents/skills/browseros-neo/SKILL.md");
            fs::create_dir_all(
                shared
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("missing skill parent"))?,
            )?;
            fs::write(&shared, "v1")?;
            let original_modified = fs::metadata(&shared)?.modified()?;
            let sibling = shared.with_file_name("notes.txt");
            fs::write(&sibling, "keep")?;
            make("v1")?.run_skill_reconciliation().await?;
            assert_eq!(fs::read_to_string(&shared)?, "v1");
            assert_eq!(fs::metadata(&shared)?.modified()?, original_modified);
            assert_eq!(fs::read_to_string(&sibling)?, "keep");
            fs::write(&shared, "human or agent edits")?;
            let updated = make("v2")?;
            updated.run_skill_reconciliation().await?;
            assert_eq!(fs::read_to_string(&shared)?, "v2");
            assert_eq!(fs::read_to_string(&sibling)?, "keep");
            let modified = fs::metadata(&shared)?.modified()?;
            updated.run_skill_reconciliation().await?;
            assert_eq!(fs::metadata(&shared)?.modified()?, modified);
            fs::remove_file(&shared)?;
            updated.run_skill_reconciliation().await?;
            assert_eq!(fs::read_to_string(&shared)?, "v2");
            Ok(())
        },
    )
}

#[test]
fn reconciliation_discovers_apps_and_respects_disconnect_after_restart() -> anyhow::Result<()> {
    isolated(
        "reconciliation_discovers_apps_and_respects_disconnect_after_restart",
        None,
        |home| async move {
            let make = || -> anyhow::Result<HarnessService> {
                Ok(HarnessService::new_with_managed_skill(
                    home.join("state/mcp"),
                    home.join("state/skills"),
                    home.clone(),
                    SkillSpec::new("browseros-neo", "instructions")?,
                    Arc::new(NoopAnalyticsSink),
                ))
            };
            let service = make()?;
            let url = "http://127.0.0.1:9200/mcp";
            service.run_reconciliation(url).await?;
            assert!(!home.join(".codex").exists());
            fs::create_dir_all(home.join(".codex"))?;
            fs::write(home.join(".codex/config.toml"), "model = \"custom\"\n")?;
            fs::create_dir_all(home.join(".cursor"))?;
            service.run_reconciliation(url).await?;
            assert!(fs::read_to_string(home.join(".codex/config.toml"))?.contains(url));
            assert!(fs::read_to_string(home.join(".cursor/mcp.json"))?.contains(url));
            service.disconnect_browseros(Harness::Codex).await?;
            let shared = home.join(".agents/skills/browseros-neo/SKILL.md");
            assert!(shared.exists(), "disconnect must preserve shared skills");
            let shared_modified = fs::metadata(&shared)?.modified()?;
            let reopened = make()?;
            reopened
                .run_reconciliation("http://127.0.0.1:9300/mcp")
                .await?;
            assert!(
                !fs::read_to_string(home.join(".codex/config.toml"))?.contains("browseros-neo")
            );
            assert!(fs::read_to_string(home.join(".cursor/mcp.json"))?.contains(":9300/mcp"));
            assert_eq!(fs::metadata(&shared)?.modified()?, shared_modified);
            assert!(
                reopened
                    .connect_browseros(Harness::Codex, url)
                    .await?
                    .installed
            );
            reopened
                .run_reconciliation("http://127.0.0.1:9400/mcp")
                .await?;
            assert!(fs::read_to_string(home.join(".codex/config.toml"))?.contains(":9400/mcp"));
            Ok(())
        },
    )
}

#[test]
fn upgrade_preserves_old_disconnects_but_discovers_later_apps() -> anyhow::Result<()> {
    isolated(
        "upgrade_preserves_old_disconnects_but_discovers_later_apps",
        None,
        |home| async move {
            fs::create_dir_all(home.join(".codex"))?;
            fs::write(home.join(".codex/config.toml"), "model = \"custom\"\n")?;
            fs::create_dir_all(home.join(".cursor"))?;
            // An existing unrecorded connection is consent to keep it, even with
            // custom fields. A missing Codex entry on an old profile is ambiguous.
            fs::write(
                home.join(".cursor/mcp.json"),
                r#"{"mcpServers":{"browseros-neo":{"url":"http://127.0.0.1:9100/mcp","headers":{"X":"keep"}}}}"#,
            )?;
            fs::create_dir_all(home.join("state"))?;
            fs::write(
                home.join("state/first-run-connect.json"),
                r#"{"firstRunConnectDone":true}"#,
            )?;
            let service = HarnessService::new_with_managed_skill(
                home.join("state/mcp"),
                home.join("state/skills"),
                home.clone(),
                SkillSpec::new("browseros-neo", "instructions")?,
                Arc::new(NoopAnalyticsSink),
            );
            service
                .run_reconciliation("http://127.0.0.1:9200/mcp")
                .await?;
            assert!(
                !fs::read_to_string(home.join(".codex/config.toml"))?.contains("browseros-neo")
            );
            assert!(fs::read_to_string(home.join(".cursor/mcp.json"))?.contains(":9200/mcp"));
            fs::create_dir_all(home.join(".config/opencode"))?;
            fs::write(home.join(".config/opencode/opencode.json"), "{}")?;
            service
                .run_reconciliation("http://127.0.0.1:9200/mcp")
                .await?;
            assert!(
                fs::read_to_string(home.join(".config/opencode/opencode.json"))?
                    .contains(":9200/mcp")
            );
            assert!(
                service
                    .connect_browseros(Harness::Codex, "http://127.0.0.1:9200/mcp")
                    .await?
                    .installed
            );
            Ok(())
        },
    )
}

#[test]
fn corrupt_preferences_block_automatic_writes_and_failed_connect_keeps_opt_out()
-> anyhow::Result<()> {
    isolated(
        "corrupt_preferences_block_automatic_writes_and_failed_connect_keeps_opt_out",
        None,
        |home| async move {
            let service = HarnessService::new_with_managed_skill(
                home.join("state/mcp"),
                home.join("state/skills"),
                home.clone(),
                SkillSpec::new("browseros-neo", "instructions")?,
                Arc::new(NoopAnalyticsSink),
            );
            fs::create_dir_all(home.join(".codex"))?;
            fs::write(home.join(".codex/config.toml"), "model = \"custom\"\n")?;
            service.disconnect_browseros(Harness::Codex).await?;
            let preferences = home.join("state/mcp/connection-preferences.json");
            let saved = fs::read(&preferences)?;
            fs::write(home.join(".codex/config.toml"), "not [toml")?;
            assert!(
                !service
                    .connect_browseros(Harness::Codex, "http://127.0.0.1:9200/mcp")
                    .await?
                    .installed
            );
            assert_eq!(fs::read(&preferences)?, saved);
            fs::write(&preferences, "{bad")?;
            let shared = home.join(".agents/skills/browseros-neo/SKILL.md");
            fs::remove_file(&shared)?;
            assert!(
                service
                    .run_reconciliation("http://127.0.0.1:9200/mcp")
                    .await
                    .is_err()
            );
            assert!(!shared.exists());
            assert_eq!(fs::read_to_string(&preferences)?, "{bad");
            assert_eq!(
                fs::read_to_string(home.join(".codex/config.toml"))?,
                "not [toml"
            );
            Ok(())
        },
    )
}

#[test]
fn upgrade_isolates_broken_app_and_disconnect_removes_unrecorded_entries() -> anyhow::Result<()> {
    isolated(
        "upgrade_isolates_broken_app_and_disconnect_removes_unrecorded_entries",
        None,
        |home| async move {
            fs::create_dir_all(home.join(".codex"))?;
            fs::write(home.join(".codex/config.toml"), "invalid [toml")?;
            fs::create_dir_all(home.join(".cursor/skills/browseros-neo"))?;
            let skill = home.join(".cursor/skills/browseros-neo");
            fs::write(skill.join("SKILL.md"), "old instructions")?;
            fs::write(skill.join("notes.txt"), "keep")?;
            let cursor = home.join(".cursor/mcp.json");
            fs::write(
                &cursor,
                r#"{"mcpServers":{"browseros-neo":{"url":"http://127.0.0.1:9100/mcp","headers":{"X":"keep"}},"BrowserClaw":{"url":"http://127.0.0.1:9100/mcp","disabled":false},"other":{"url":"https://example.com/mcp"}}}"#,
            )?;
            fs::create_dir_all(home.join("state"))?;
            fs::write(
                home.join("state/first-run-connect.json"),
                r#"{"firstRunConnectDone":true}"#,
            )?;
            let service = HarnessService::new_with_managed_skill(
                home.join("state/mcp"),
                home.join("state/skills"),
                home.clone(),
                SkillSpec::new("browseros-neo", "instructions")?,
                Arc::new(NoopAnalyticsSink),
            );
            // Explicit Disconnect before the first maintenance pass must work without
            // manifest records, and another app's malformed TOML must not block it.
            let state = service.disconnect_browseros(Harness::Cursor).await?;
            assert!(!state.installed, "{}", state.message);
            let raw = fs::read_to_string(&cursor)?;
            assert!(!raw.contains("browseros-neo") && !raw.contains("BrowserClaw"));
            assert!(raw.contains("https://example.com/mcp"));
            assert!(!skill.join("SKILL.md").exists());
            assert_eq!(fs::read_to_string(skill.join("notes.txt"))?, "keep");
            service
                .run_reconciliation("http://127.0.0.1:9200/mcp")
                .await?;
            assert_eq!(fs::read_to_string(&cursor)?, raw);
            assert_eq!(
                fs::read_to_string(home.join(".codex/config.toml"))?,
                "invalid [toml"
            );
            assert!(home.join(".agents/skills/browseros-neo/SKILL.md").exists());
            Ok(())
        },
    )
}
