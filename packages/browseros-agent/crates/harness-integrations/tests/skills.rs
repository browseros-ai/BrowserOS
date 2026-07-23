use std::{collections::BTreeSet, fs, thread, time::Duration};

use harness_integrations::{
    AgentId, SkillEnvironment, SkillReconciler, SkillSpec, TargetPlatform,
    resolve_agent_skill_target,
};
use serde_json::Value;
use tempfile::tempdir;

fn agents(values: &[AgentId]) -> BTreeSet<AgentId> {
    values.iter().copied().collect()
}

fn spec(content: &str) -> Result<SkillSpec, harness_integrations::Error> {
    SkillSpec::new("browserclaw", content)
}

#[test]
fn resolves_current_global_roots_and_groups_shared_targets()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    for platform in [
        TargetPlatform::Darwin,
        TargetPlatform::Linux,
        TargetPlatform::Windows,
    ] {
        let environment = SkillEnvironment::new(root.path(), platform)
            .with_variable("CLAUDE_CONFIG_DIR", root.path().join("claude-root"))
            .with_variable("XDG_CONFIG_HOME", root.path().join("xdg"));
        let expected = [
            (AgentId::ClaudeCode, "claude-root/skills/browserclaw"),
            (AgentId::Codex, ".agents/skills/browserclaw"),
            (AgentId::Cursor, ".cursor/skills/browserclaw"),
            (AgentId::OpenCode, "xdg/opencode/skills/browserclaw"),
            (AgentId::Antigravity, ".gemini/config/skills/browserclaw"),
            (AgentId::VsCode, ".copilot/skills/browserclaw"),
            (AgentId::Zed, ".agents/skills/browserclaw"),
        ];
        for (agent, relative) in expected {
            assert_eq!(
                resolve_agent_skill_target(agent, "browserclaw", &environment)?,
                root.path().join(relative)
            );
        }
        let fallback = SkillEnvironment::new(root.path(), platform);
        assert_eq!(
            resolve_agent_skill_target(AgentId::ClaudeCode, "browserclaw", &fallback)?,
            root.path().join(".claude/skills/browserclaw")
        );
        assert_eq!(
            resolve_agent_skill_target(AgentId::OpenCode, "browserclaw", &fallback)?,
            root.path().join(".config/opencode/skills/browserclaw")
        );
    }
    Ok(())
}

#[test]
fn installs_updates_repairs_and_preserves_true_no_ops() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let environment = SkillEnvironment::new(root.path().join("home"), TargetPlatform::Linux);
    let reconciler = SkillReconciler::new(root.path().join("state"));
    let target = resolve_agent_skill_target(AgentId::Cursor, "browserclaw", &environment)?;

    let installed =
        reconciler.reconcile(&spec("first\n")?, &agents(&[AgentId::Cursor]), &environment)?;
    assert_eq!(installed.installed, 1);
    assert!(installed.warnings.is_empty());
    assert_eq!(fs::read_to_string(target.join("SKILL.md"))?, "first\n");
    let marker: Value = serde_json::from_str(&fs::read_to_string(
        target.join(".browserclaw-managed.json"),
    )?)?;
    assert_eq!(marker["managedBy"], "browserclaw");
    assert_eq!(marker["skillName"], "browserclaw");

    let before = fs::metadata(target.join("SKILL.md"))?.modified()?;
    let manifest_before = fs::metadata(root.path().join("state/skills.json"))?.modified()?;
    thread::sleep(Duration::from_millis(20));
    let unchanged =
        reconciler.reconcile(&spec("first\n")?, &agents(&[AgentId::Cursor]), &environment)?;
    assert_eq!(unchanged.unchanged, 1);
    assert_eq!(fs::metadata(target.join("SKILL.md"))?.modified()?, before);
    assert_eq!(
        fs::metadata(root.path().join("state/skills.json"))?.modified()?,
        manifest_before
    );

    fs::write(target.join("stale.txt"), "stale")?;
    let updated = reconciler.reconcile(
        &spec("second\n")?,
        &agents(&[AgentId::Cursor]),
        &environment,
    )?;
    assert_eq!(updated.updated, 1);
    assert_eq!(fs::read_to_string(target.join("SKILL.md"))?, "second\n");
    assert!(!target.join("stale.txt").exists());

    fs::write(target.join("SKILL.md"), "user edit")?;
    let repaired = reconciler.reconcile(
        &spec("second\n")?,
        &agents(&[AgentId::Cursor]),
        &environment,
    )?;
    assert_eq!(repaired.updated, 1);
    assert_eq!(fs::read_to_string(target.join("SKILL.md"))?, "second\n");
    Ok(())
}

#[test]
fn controlled_invalid_filesystem_shapes_are_replaced() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let environment = SkillEnvironment::new(root.path().join("home"), TargetPlatform::Linux);
    let reconciler = SkillReconciler::new(root.path().join("state"));
    let target = resolve_agent_skill_target(AgentId::Cursor, "browserclaw", &environment)?;
    let desired = agents(&[AgentId::Cursor]);
    reconciler.reconcile(&spec("managed\n")?, &desired, &environment)?;

    fs::remove_dir_all(&target)?;
    fs::write(&target, "not a directory")?;
    let repaired_target = reconciler.reconcile(&spec("managed\n")?, &desired, &environment)?;
    assert_eq!(repaired_target.updated, 1);
    assert_eq!(fs::read_to_string(target.join("SKILL.md"))?, "managed\n");

    fs::remove_file(target.join("SKILL.md"))?;
    fs::create_dir(target.join("SKILL.md"))?;
    let repaired_skill = reconciler.reconcile(&spec("managed\n")?, &desired, &environment)?;
    assert_eq!(repaired_skill.updated, 1);
    assert_eq!(fs::read_to_string(target.join("SKILL.md"))?, "managed\n");
    Ok(())
}

#[test]
fn either_manifest_or_marker_recovers_ownership() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let environment = SkillEnvironment::new(root.path().join("home"), TargetPlatform::Darwin);
    let state = root.path().join("state");
    let reconciler = SkillReconciler::new(&state);
    let target = resolve_agent_skill_target(AgentId::ClaudeCode, "browserclaw", &environment)?;
    let desired = agents(&[AgentId::ClaudeCode]);
    reconciler.reconcile(&spec("managed\n")?, &desired, &environment)?;

    fs::remove_file(target.join(".browserclaw-managed.json"))?;
    let from_manifest = reconciler.reconcile(&spec("managed\n")?, &desired, &environment)?;
    assert_eq!(from_manifest.updated, 1);
    assert!(target.join(".browserclaw-managed.json").exists());

    fs::remove_file(state.join("skills.json"))?;
    let content_before = fs::metadata(target.join("SKILL.md"))?.modified()?;
    let from_marker = reconciler.reconcile(&spec("managed\n")?, &desired, &environment)?;
    assert_eq!(from_marker.unchanged, 1);
    assert_eq!(
        fs::metadata(target.join("SKILL.md"))?.modified()?,
        content_before
    );
    assert!(state.join("skills.json").exists());
    Ok(())
}

#[test]
fn marker_only_target_is_removed_after_the_last_disconnect()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let environment = SkillEnvironment::new(root.path().join("home"), TargetPlatform::Linux);
    let state = root.path().join("state");
    let reconciler = SkillReconciler::new(&state);
    let target = resolve_agent_skill_target(AgentId::Cursor, "browserclaw", &environment)?;

    reconciler.reconcile(
        &spec("managed\n")?,
        &agents(&[AgentId::Cursor]),
        &environment,
    )?;
    fs::remove_file(state.join("skills.json"))?;

    let removed = reconciler.reconcile(&spec("managed\n")?, &BTreeSet::new(), &environment)?;

    assert_eq!(removed.removed, 1);
    assert!(!target.exists());
    Ok(())
}

#[test]
fn shared_consumers_keep_the_target_until_the_last_disconnect()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let environment = SkillEnvironment::new(root.path().join("home"), TargetPlatform::Linux);
    let state = root.path().join("state");
    let reconciler = SkillReconciler::new(&state);
    let target = resolve_agent_skill_target(AgentId::Codex, "browserclaw", &environment)?;

    reconciler.reconcile(
        &spec("shared\n")?,
        &agents(&[AgentId::Codex, AgentId::Zed]),
        &environment,
    )?;
    let manifest: Value = serde_json::from_str(&fs::read_to_string(state.join("skills.json"))?)?;
    assert_eq!(
        manifest["targets"][0]["consumers"],
        serde_json::json!(["codex", "zed"])
    );

    let one_left =
        reconciler.reconcile(&spec("shared\n")?, &agents(&[AgentId::Zed]), &environment)?;
    assert_eq!(one_left.updated, 1);
    assert!(target.exists());

    let removed = reconciler.reconcile(&spec("shared\n")?, &BTreeSet::new(), &environment)?;
    assert_eq!(removed.removed, 1);
    assert!(!target.exists());
    Ok(())
}

#[test]
fn foreign_targets_and_corrupt_manifests_are_preserved() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let environment = SkillEnvironment::new(root.path().join("home"), TargetPlatform::Linux);
    let state = root.path().join("state");
    let reconciler = SkillReconciler::new(&state);
    let target = resolve_agent_skill_target(AgentId::Cursor, "browserclaw", &environment)?;
    fs::create_dir_all(&target)?;
    fs::write(target.join("SKILL.md"), "foreign")?;
    fs::write(target.join("keep.txt"), "keep")?;

    let outcome = reconciler.reconcile(
        &spec("managed\n")?,
        &agents(&[AgentId::Cursor]),
        &environment,
    )?;
    assert_eq!(outcome.warnings.len(), 1);
    assert_eq!(fs::read_to_string(target.join("SKILL.md"))?, "foreign");
    assert_eq!(fs::read_to_string(target.join("keep.txt"))?, "keep");

    fs::create_dir_all(&state)?;
    let corrupt = "{ definitely not json";
    fs::write(state.join("skills.json"), corrupt)?;
    let error = reconciler
        .reconcile(
            &spec("managed\n")?,
            &agents(&[AgentId::Cursor]),
            &environment,
        )
        .err()
        .ok_or("corrupt manifest unexpectedly reconciled")?;
    assert!(error.to_string().contains("is not valid JSON"));
    assert_eq!(fs::read_to_string(state.join("skills.json"))?, corrupt);
    Ok(())
}
