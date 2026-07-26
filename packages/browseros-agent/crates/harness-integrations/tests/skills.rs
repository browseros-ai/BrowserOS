use std::{collections::BTreeSet, fs, thread, time::Duration};

#[cfg(unix)]
use std::os::unix::fs::symlink;

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
    assert_eq!(one_left.unchanged, 1);
    assert!(target.exists());

    let removed = reconciler.reconcile(&spec("shared\n")?, &BTreeSet::new(), &environment)?;
    assert_eq!(removed.removed, 1);
    assert!(!target.exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn aliased_harness_roots_share_one_physical_installation() -> Result<(), Box<dyn std::error::Error>>
{
    let root = tempdir()?;
    let home = root.path().join("home");
    let shared_root = home.join(".skills");
    fs::create_dir_all(home.join(".claude"))?;
    fs::create_dir_all(home.join(".agents"))?;
    fs::create_dir_all(&shared_root)?;
    symlink(&shared_root, home.join(".claude/skills"))?;
    symlink(&shared_root, home.join(".agents/skills"))?;

    let environment = SkillEnvironment::new(&home, TargetPlatform::Linux);
    let state = root.path().join("state");
    let reconciler = SkillReconciler::new(&state);
    let physical_target = fs::canonicalize(&shared_root)?.join("browserclaw");

    let installed = reconciler.reconcile(
        &spec("shared\n")?,
        &agents(&[AgentId::ClaudeCode]),
        &environment,
    )?;
    assert_eq!(installed.installed, 1);
    assert_eq!(installed.removed, 0);
    assert!(installed.warnings.is_empty());
    assert_eq!(
        fs::read_to_string(physical_target.join("SKILL.md"))?,
        "shared\n"
    );

    let manifest: Value = serde_json::from_str(&fs::read_to_string(state.join("skills.json"))?)?;
    assert_eq!(manifest["targets"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        manifest["targets"][0]["targetPath"],
        physical_target.to_string_lossy().as_ref()
    );
    assert_eq!(
        manifest["targets"][0]["consumers"],
        serde_json::json!(["claude-code"])
    );

    let modified = fs::metadata(physical_target.join("SKILL.md"))?.modified()?;
    thread::sleep(Duration::from_millis(20));
    let shared = reconciler.reconcile(
        &spec("shared\n")?,
        &agents(&[AgentId::ClaudeCode, AgentId::Codex]),
        &environment,
    )?;
    assert_eq!(shared.unchanged, 1);
    assert_eq!(shared.updated, 0);
    assert_eq!(
        fs::metadata(physical_target.join("SKILL.md"))?.modified()?,
        modified
    );
    let manifest: Value = serde_json::from_str(&fs::read_to_string(state.join("skills.json"))?)?;
    assert_eq!(
        manifest["targets"][0]["consumers"],
        serde_json::json!(["claude-code", "codex"])
    );

    let one_left =
        reconciler.reconcile(&spec("shared\n")?, &agents(&[AgentId::Codex]), &environment)?;
    assert_eq!(one_left.unchanged, 1);
    assert!(physical_target.exists());

    fs::remove_file(state.join("skills.json"))?;
    let marker_only = reconciler.reconcile(
        &spec("shared\n")?,
        &agents(&[AgentId::ClaudeCode]),
        &environment,
    )?;
    assert_eq!(marker_only.unchanged, 1);
    assert_eq!(marker_only.removed, 0);
    assert!(physical_target.exists());

    let removed = reconciler.reconcile(&spec("shared\n")?, &BTreeSet::new(), &environment)?;
    assert_eq!(removed.removed, 1);
    assert!(!physical_target.exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn legacy_alias_records_migrate_and_conflicts_fail_before_mutation()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let home = root.path().join("home");
    let shared_root = home.join(".skills");
    fs::create_dir_all(home.join(".claude"))?;
    fs::create_dir_all(home.join(".agents"))?;
    fs::create_dir_all(&shared_root)?;
    symlink(&shared_root, home.join(".claude/skills"))?;
    symlink(&shared_root, home.join(".agents/skills"))?;

    let environment = SkillEnvironment::new(&home, TargetPlatform::Linux);
    let state = root.path().join("state");
    let manifest_path = state.join("skills.json");
    let reconciler = SkillReconciler::new(&state);
    let desired = agents(&[AgentId::ClaudeCode, AgentId::Codex]);
    let physical_target = fs::canonicalize(&shared_root)?.join("browserclaw");
    reconciler.reconcile(&spec("managed\n")?, &desired, &environment)?;

    let mut manifest: Value = serde_json::from_str(&fs::read_to_string(&manifest_path)?)?;
    let canonical_entry = manifest["targets"][0].clone();
    manifest["targets"][0]["targetPath"] = Value::String(
        home.join(".claude/skills/browserclaw")
            .display()
            .to_string(),
    );
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    fs::remove_dir_all(&physical_target)?;

    let repaired = reconciler.reconcile(&spec("managed\n")?, &desired, &environment)?;
    assert_eq!(repaired.installed, 1);
    let migrated: Value = serde_json::from_str(&fs::read_to_string(&manifest_path)?)?;
    assert_eq!(migrated["targets"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        migrated["targets"][0]["targetPath"],
        physical_target.to_string_lossy().as_ref()
    );

    let modified = fs::metadata(physical_target.join("SKILL.md"))?.modified()?;
    let mut duplicate_manifest = migrated.clone();
    let mut claude_entry = canonical_entry.clone();
    claude_entry["targetPath"] = Value::String(
        home.join(".claude/skills/browserclaw")
            .display()
            .to_string(),
    );
    claude_entry["consumers"] = serde_json::json!(["claude-code"]);
    let mut codex_entry = canonical_entry;
    codex_entry["targetPath"] = Value::String(
        home.join(".agents/skills/browserclaw")
            .display()
            .to_string(),
    );
    codex_entry["consumers"] = serde_json::json!(["codex"]);
    duplicate_manifest["targets"] = Value::Array(vec![claude_entry, codex_entry]);
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&duplicate_manifest)?,
    )?;

    thread::sleep(Duration::from_millis(20));
    let collapsed = reconciler.reconcile(&spec("managed\n")?, &desired, &environment)?;
    assert_eq!(collapsed.unchanged, 1);
    assert_eq!(
        fs::metadata(physical_target.join("SKILL.md"))?.modified()?,
        modified
    );
    let collapsed_manifest: Value = serde_json::from_str(&fs::read_to_string(&manifest_path)?)?;
    assert_eq!(
        collapsed_manifest["targets"].as_array().map(Vec::len),
        Some(1)
    );

    let mut conflicting_manifest = duplicate_manifest;
    conflicting_manifest["targets"][1]["contentHash"] = Value::String("conflict".to_string());
    let conflicting_bytes = serde_json::to_vec_pretty(&conflicting_manifest)?;
    fs::write(&manifest_path, &conflicting_bytes)?;
    let skill_before = fs::read(physical_target.join("SKILL.md"))?;

    let error = reconciler
        .reconcile(&spec("managed\n")?, &desired, &environment)
        .err()
        .ok_or("conflicting aliases unexpectedly reconciled")?;
    assert!(error.to_string().contains("conflicting records"));
    assert_eq!(fs::read(&manifest_path)?, conflicting_bytes);
    assert_eq!(fs::read(physical_target.join("SKILL.md"))?, skill_before);
    Ok(())
}

#[test]
fn retargeted_root_installs_new_destination_and_removes_old_one()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let state = root.path().join("state");
    let reconciler = SkillReconciler::new(&state);
    let home = root.path().join("home");
    let old_root = root.path().join("old-claude");
    let old_environment = SkillEnvironment::new(&home, TargetPlatform::Linux)
        .with_variable("CLAUDE_CONFIG_DIR", &old_root);
    let desired = agents(&[AgentId::ClaudeCode]);
    reconciler.reconcile(&spec("managed\n")?, &desired, &old_environment)?;
    let old_target = old_root.join("skills/browserclaw");
    assert!(old_target.exists());

    let new_root = root.path().join("new-claude");
    let new_environment = SkillEnvironment::new(&home, TargetPlatform::Linux)
        .with_variable("CLAUDE_CONFIG_DIR", &new_root);
    let moved = reconciler.reconcile(&spec("managed\n")?, &desired, &new_environment)?;

    assert_eq!(moved.installed, 1);
    assert_eq!(moved.removed, 1);
    assert!(!old_target.exists());
    assert_eq!(
        fs::read_to_string(new_root.join("skills/browserclaw/SKILL.md"))?,
        "managed\n"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn foreign_final_symlink_is_not_followed_or_replaced() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let home = root.path().join("home");
    let skill_root = home.join(".claude/skills");
    let foreign = root.path().join("foreign");
    fs::create_dir_all(&skill_root)?;
    fs::create_dir_all(&foreign)?;
    fs::write(foreign.join("SKILL.md"), "foreign\n")?;
    fs::write(foreign.join("keep.txt"), "keep\n")?;
    let target = skill_root.join("browserclaw");
    symlink(&foreign, &target)?;

    let environment = SkillEnvironment::new(&home, TargetPlatform::Linux);
    let reconciler = SkillReconciler::new(root.path().join("state"));
    let outcome = reconciler.reconcile(
        &spec("managed\n")?,
        &agents(&[AgentId::ClaudeCode]),
        &environment,
    )?;

    assert_eq!(outcome.warnings.len(), 1);
    assert!(fs::symlink_metadata(&target)?.file_type().is_symlink());
    assert_eq!(fs::read_to_string(foreign.join("SKILL.md"))?, "foreign\n");
    assert_eq!(fs::read_to_string(foreign.join("keep.txt"))?, "keep\n");
    Ok(())
}

#[cfg(unix)]
#[test]
fn legacy_alias_record_does_not_claim_a_retargeted_foreign_destination()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let home = root.path().join("home");
    let claude_root = home.join(".claude");
    let old_root = home.join("old-skills");
    let new_root = home.join("new-skills");
    fs::create_dir_all(&claude_root)?;
    fs::create_dir_all(&old_root)?;
    fs::create_dir_all(&new_root)?;
    let alias = claude_root.join("skills");
    symlink(&old_root, &alias)?;

    let environment = SkillEnvironment::new(&home, TargetPlatform::Linux);
    let state = root.path().join("state");
    let manifest_path = state.join("skills.json");
    let reconciler = SkillReconciler::new(&state);
    let desired = agents(&[AgentId::ClaudeCode]);
    reconciler.reconcile(&spec("managed\n")?, &desired, &environment)?;

    let mut manifest: Value = serde_json::from_str(&fs::read_to_string(&manifest_path)?)?;
    manifest["targets"][0]["targetPath"] =
        Value::String(alias.join("browserclaw").display().to_string());
    let legacy_manifest = serde_json::to_vec_pretty(&manifest)?;
    fs::write(&manifest_path, &legacy_manifest)?;

    fs::remove_file(&alias)?;
    symlink(&new_root, &alias)?;
    let foreign_target = new_root.join("browserclaw");
    fs::create_dir_all(&foreign_target)?;
    fs::write(foreign_target.join("SKILL.md"), "foreign\n")?;
    fs::write(foreign_target.join("keep.txt"), "keep\n")?;

    let outcome = reconciler.reconcile(&spec("managed\n")?, &desired, &environment)?;

    assert_eq!(outcome.warnings.len(), 1);
    assert_eq!(
        fs::read_to_string(foreign_target.join("SKILL.md"))?,
        "foreign\n"
    );
    assert_eq!(
        fs::read_to_string(foreign_target.join("keep.txt"))?,
        "keep\n"
    );
    assert_eq!(fs::read(&manifest_path)?, legacy_manifest);
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
