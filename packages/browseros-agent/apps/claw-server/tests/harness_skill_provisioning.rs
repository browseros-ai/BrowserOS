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
    assert!(home.join(".claude/skills/browseros-neo/SKILL.md").is_file());
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
                    .maintain_skills(async {
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
            tokio::time::advance(Duration::from_secs(60)).await;
            wait_for_skill(&shared).await?;
            assert_eq!(fs::read_to_string(&shared)?, "background instructions\n");
            stop.send(())
                .map_err(|()| anyhow::anyhow!("worker stopped before shutdown"))?;
            worker.await?;
            fs::remove_file(&shared)?;
            tokio::time::advance(Duration::from_secs(120)).await;
            assert!(!shared.exists(), "shutdown must stop periodic repairs");
            assert!(!home.join("state/mcp/manifest.json").exists());
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
