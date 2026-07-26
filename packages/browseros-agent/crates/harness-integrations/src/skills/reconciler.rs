use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use tempfile::Builder;

use crate::{
    catalog::{AgentId, PerOsPaths, resolve_harness_definition},
    error::Error,
};

use super::{
    manifest::{
        MARKER_FILE, OwnershipMarker, SkillManifest, SkillManifestEntry, marker_content,
        read_manifest, read_marker, write_manifest,
    },
    types::{SkillEnvironment, SkillReconcileOutcome, SkillSpec, SkillWarning, TargetPlatform},
};

/// Reconciles product-supplied skills into catalog-defined global harness roots.
#[derive(Debug, Clone)]
pub struct SkillReconciler {
    workspace_dir: PathBuf,
}

impl SkillReconciler {
    #[must_use]
    pub fn new(workspace_dir: impl Into<PathBuf>) -> Self {
        Self {
            workspace_dir: workspace_dir.into(),
        }
    }

    /// Converges every desired physical target and removes stale controlled targets.
    pub fn reconcile(
        &self,
        spec: &SkillSpec,
        consumers: &BTreeSet<AgentId>,
        environment: &SkillEnvironment,
    ) -> Result<SkillReconcileOutcome, Error> {
        self.reconcile_with(spec, consumers, environment, replace_managed_directory)
    }

    fn reconcile_with(
        &self,
        spec: &SkillSpec,
        consumers: &BTreeSet<AgentId>,
        environment: &SkillEnvironment,
        mut replace: impl FnMut(&Path, &SkillSpec, &str) -> std::io::Result<()>,
    ) -> Result<SkillReconcileOutcome, Error> {
        self.reconcile_with_identity(
            spec,
            consumers,
            environment,
            physical_skill_target,
            &mut replace,
        )
    }

    fn reconcile_with_identity(
        &self,
        spec: &SkillSpec,
        consumers: &BTreeSet<AgentId>,
        environment: &SkillEnvironment,
        mut identity: impl FnMut(&Path) -> Result<PathBuf, Error>,
        mut replace: impl FnMut(&Path, &SkillSpec, &str) -> std::io::Result<()>,
    ) -> Result<SkillReconcileOutcome, Error> {
        let original = read_manifest(&self.workspace_dir)?;
        let plan = plan_reconciliation(&original, spec, consumers, environment, &mut identity)?;
        let mut records = plan.records;
        let desired_hash = content_hash(spec.content.as_bytes());
        let mut outcome = SkillReconcileOutcome::default();
        let mut preserve_original_records = BTreeSet::new();

        for (target, target_consumers) in &plan.desired {
            let record_controls = records
                .get(target)
                .is_some_and(|record| record.skill_name == spec.name)
                && plan.manifest_controlled_targets.contains(target);
            let metadata = match fs::symlink_metadata(target) {
                Ok(metadata) => Some(metadata),
                Err(error) if error.kind() == ErrorKind::NotFound => None,
                Err(error) => {
                    preserve_original_records.insert(target.clone());
                    outcome.warnings.push(SkillWarning {
                        target: target.clone(),
                        message: format!("Could not inspect managed skill target: {error}"),
                    });
                    continue;
                }
            };
            let marker = if metadata.as_ref().is_some_and(|value| value.is_dir()) {
                match read_marker(target) {
                    Ok(marker) => marker,
                    Err(error) => {
                        preserve_original_records.insert(target.clone());
                        outcome.warnings.push(SkillWarning {
                            target: target.clone(),
                            message: error.to_string(),
                        });
                        continue;
                    }
                }
            } else {
                None
            };
            let marker_controls = marker
                .as_ref()
                .is_some_and(|marker| marker.controls(&spec.name));
            if metadata.is_some() && !record_controls && !marker_controls {
                preserve_original_records.insert(target.clone());
                outcome.warnings.push(SkillWarning {
                    target: target.clone(),
                    message: format!(
                        "Existing {} directory is not managed by BrowserClaw; left unchanged",
                        spec.name
                    ),
                });
                continue;
            }

            let desired_consumers = target_consumers.iter().copied().collect::<Vec<_>>();
            let entry = SkillManifestEntry {
                target_path: target.clone(),
                skill_name: spec.name.clone(),
                content_hash: desired_hash.clone(),
                consumers: desired_consumers,
            };
            let actual_hash = if metadata.as_ref().is_some_and(|value| value.is_dir()) {
                match read_content_hash(&target.join("SKILL.md")) {
                    Ok(hash) => hash,
                    Err(error) => {
                        preserve_original_records.insert(target.clone());
                        outcome.warnings.push(SkillWarning {
                            target: target.clone(),
                            message: error.to_string(),
                        });
                        continue;
                    }
                }
            } else {
                None
            };
            let marker_matches = marker.as_ref().is_some_and(|marker| {
                marker.controls(&spec.name) && marker.content_hash == desired_hash
            });
            let needs_replace = metadata.is_none()
                || actual_hash.as_deref() != Some(desired_hash.as_str())
                || !marker_matches;

            if needs_replace {
                match replace(target, spec, &desired_hash) {
                    Ok(()) => {
                        if metadata.is_some() {
                            outcome.updated += 1;
                        } else {
                            outcome.installed += 1;
                        }
                        records.insert(target.clone(), entry);
                    }
                    Err(error) => {
                        preserve_original_records.insert(target.clone());
                        outcome.warnings.push(SkillWarning {
                            target: target.clone(),
                            message: format!("Could not replace managed skill: {error}"),
                        });
                    }
                }
            } else {
                outcome.unchanged += 1;
                records.insert(target.clone(), entry);
            }
        }

        for target in plan.cleanup_targets {
            if plan.desired.contains_key(&target) {
                continue;
            }
            let record_matches_skill = records
                .get(&target)
                .is_some_and(|record| record.skill_name == spec.name);
            if records.contains_key(&target) && !record_matches_skill {
                preserve_original_records.insert(target);
                continue;
            }
            let record_controls =
                record_matches_skill && plan.manifest_controlled_targets.contains(&target);
            let metadata = match fs::symlink_metadata(&target) {
                Ok(metadata) => Some(metadata),
                Err(error) if error.kind() == ErrorKind::NotFound => None,
                Err(error) => {
                    preserve_original_records.insert(target.clone());
                    outcome.warnings.push(SkillWarning {
                        target: target.clone(),
                        message: format!("Could not inspect stale managed skill: {error}"),
                    });
                    continue;
                }
            };
            if metadata.is_none() && record_matches_skill {
                records.remove(&target);
                continue;
            }
            let marker_controls = if record_controls {
                false
            } else if metadata.as_ref().is_some_and(|value| value.is_dir()) {
                match read_marker(&target) {
                    Ok(marker) => marker
                        .as_ref()
                        .is_some_and(|marker| marker.controls(&spec.name)),
                    Err(error) => {
                        preserve_original_records.insert(target.clone());
                        outcome.warnings.push(SkillWarning {
                            target: target.clone(),
                            message: error.to_string(),
                        });
                        continue;
                    }
                }
            } else {
                false
            };
            if !record_controls && !marker_controls {
                continue;
            }
            match metadata {
                Some(_) => match remove_path(&target) {
                    Ok(()) => {
                        outcome.removed += 1;
                        records.remove(&target);
                    }
                    Err(error) => {
                        preserve_original_records.insert(target.clone());
                        outcome.warnings.push(SkillWarning {
                            target: target.clone(),
                            message: format!("Could not remove managed skill: {error}"),
                        });
                    }
                },
                None => {
                    records.remove(&target);
                }
            }
        }

        let mut targets = Vec::new();
        for (target, record) in records {
            if preserve_original_records.contains(&target)
                && let Some(original_records) = plan.original_records.get(&target)
            {
                targets.extend(original_records.iter().cloned());
                continue;
            }
            targets.push(record);
        }
        targets.sort_by(|left, right| {
            left.target_path
                .cmp(&right.target_path)
                .then_with(|| left.skill_name.cmp(&right.skill_name))
        });
        let next = SkillManifest {
            version: 1,
            targets,
        };
        if next != original {
            write_manifest(&self.workspace_dir, &next)?;
        }
        Ok(outcome)
    }
}

struct ReconciliationPlan {
    desired: BTreeMap<PathBuf, BTreeSet<AgentId>>,
    records: BTreeMap<PathBuf, SkillManifestEntry>,
    original_records: BTreeMap<PathBuf, Vec<SkillManifestEntry>>,
    /// Migrated aliases need a marker before they can control an existing destination.
    manifest_controlled_targets: BTreeSet<PathBuf>,
    cleanup_targets: BTreeSet<PathBuf>,
}

fn plan_reconciliation(
    original: &SkillManifest,
    spec: &SkillSpec,
    consumers: &BTreeSet<AgentId>,
    environment: &SkillEnvironment,
    identity: &mut impl FnMut(&Path) -> Result<PathBuf, Error>,
) -> Result<ReconciliationPlan, Error> {
    let desired = desired_targets(consumers, &spec.name, environment, identity)?;
    let mut records = BTreeMap::<PathBuf, SkillManifestEntry>::new();
    let mut original_records = BTreeMap::<PathBuf, Vec<SkillManifestEntry>>::new();
    let mut manifest_controlled_targets = BTreeSet::new();
    for original_entry in &original.targets {
        let target = identity(&original_entry.target_path)?;
        if original_entry.target_path == target {
            manifest_controlled_targets.insert(target.clone());
        }
        original_records
            .entry(target.clone())
            .or_default()
            .push(original_entry.clone());

        let mut entry = original_entry.clone();
        entry.target_path = target.clone();
        entry.consumers.sort();
        entry.consumers.dedup();
        if let Some(existing) = records.get_mut(&target) {
            if existing.skill_name != entry.skill_name
                || existing.content_hash != entry.content_hash
            {
                return Err(Error::Manifest {
                    message: format!(
                        "Skill manifest contains conflicting records for physical target {}.",
                        target.display()
                    ),
                });
            }
            let consumers = existing
                .consumers
                .iter()
                .chain(&entry.consumers)
                .copied()
                .collect::<BTreeSet<_>>();
            existing.consumers = consumers.into_iter().collect();
        } else {
            records.insert(target, entry);
        }
    }

    for target in desired.keys() {
        if let Some(record) = records.get(target)
            && record.skill_name != spec.name
        {
            return Err(Error::Manifest {
                message: format!(
                    "Skill manifest target {} belongs to skill {}; cannot reconcile {} there.",
                    target.display(),
                    record.skill_name,
                    spec.name
                ),
            });
        }
    }

    let mut cleanup_targets = records.keys().cloned().collect::<BTreeSet<_>>();
    for agent in AgentId::ALL {
        if resolve_harness_definition(agent).skill.is_some() {
            let target = resolve_agent_skill_target(agent, &spec.name, environment)?;
            cleanup_targets.insert(identity(&target)?);
        }
    }
    Ok(ReconciliationPlan {
        desired,
        records,
        original_records,
        manifest_controlled_targets,
        cleanup_targets,
    })
}

/// Resolves one harness's preferred global directory for a named skill.
pub fn resolve_agent_skill_target(
    agent: AgentId,
    skill_name: &str,
    environment: &SkillEnvironment,
) -> Result<PathBuf, Error> {
    SkillSpec::new(skill_name, "")?;
    let surface =
        resolve_harness_definition(agent)
            .skill
            .ok_or_else(|| Error::UnresolvedSkillTarget {
                agent,
                reason: "catalog has no global skill surface".to_string(),
            })?;
    selected_paths(&surface.global_roots, environment.platform)
        .iter()
        .find_map(|candidate| expand_root(candidate, environment))
        .map(|root| root.join(skill_name))
        .ok_or_else(|| Error::UnresolvedSkillTarget {
            agent,
            reason: "no global skill root resolves from the available environment".to_string(),
        })
}

fn desired_targets(
    consumers: &BTreeSet<AgentId>,
    skill_name: &str,
    environment: &SkillEnvironment,
    identity: &mut impl FnMut(&Path) -> Result<PathBuf, Error>,
) -> Result<BTreeMap<PathBuf, BTreeSet<AgentId>>, Error> {
    let mut targets = BTreeMap::<PathBuf, BTreeSet<AgentId>>::new();
    for consumer in consumers {
        let target = resolve_agent_skill_target(*consumer, skill_name, environment)?;
        let target = identity(&target)?;
        targets.entry(target).or_default().insert(*consumer);
    }
    Ok(targets)
}

/// Follows aliases in the harness skill root without following the managed skill entry itself.
fn physical_skill_target(target: &Path) -> Result<PathBuf, Error> {
    let skill_name = target.file_name().ok_or_else(|| {
        Error::io(
            "resolve filesystem identity for",
            target,
            std::io::Error::new(
                ErrorKind::InvalidInput,
                "skill target has no final component",
            ),
        )
    })?;
    let root = target.parent().ok_or_else(|| {
        Error::io(
            "resolve filesystem identity for",
            target,
            std::io::Error::new(ErrorKind::InvalidInput, "skill target has no parent"),
        )
    })?;
    let mut ancestor = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| Error::io("resolve filesystem identity for", target, error))?
            .join(root)
    };
    let mut missing = Vec::new();

    loop {
        match fs::symlink_metadata(&ancestor) {
            Ok(_) => {
                let mut physical_root = fs::canonicalize(&ancestor)
                    .map_err(|error| Error::io("resolve filesystem identity for", target, error))?;
                for component in missing.iter().rev() {
                    physical_root.push(component);
                }
                physical_root.push(skill_name);
                return Ok(physical_root);
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let component = ancestor.file_name().map(ToOwned::to_owned).ok_or_else(|| {
                    Error::io(
                        "resolve filesystem identity for",
                        target,
                        std::io::Error::new(
                            ErrorKind::NotFound,
                            "no existing ancestor for skill root",
                        ),
                    )
                })?;
                missing.push(component);
                if !ancestor.pop() {
                    return Err(Error::io("resolve filesystem identity for", target, error));
                }
            }
            Err(error) => {
                return Err(Error::io("resolve filesystem identity for", target, error));
            }
        }
    }
}

fn selected_paths(paths: &PerOsPaths, platform: TargetPlatform) -> &'static [&'static str] {
    match platform {
        TargetPlatform::Darwin => paths.darwin,
        TargetPlatform::Linux => paths.linux,
        TargetPlatform::Windows => paths.windows,
    }
}

fn expand_root(candidate: &str, environment: &SkillEnvironment) -> Option<PathBuf> {
    let variable = candidate.strip_prefix('$')?;
    let name_end = variable.find(['/', '\\']).unwrap_or(variable.len());
    let name = &variable[..name_end];
    let mut root = environment.variable(name)?;
    for component in variable[name_end..]
        .trim_start_matches(['/', '\\'])
        .split(['/', '\\'])
        .filter(|component| !component.is_empty())
    {
        root.push(component);
    }
    Some(root)
}

fn replace_managed_directory(
    target: &Path,
    spec: &SkillSpec,
    desired_hash: &str,
) -> std::io::Result<()> {
    replace_managed_directory_with(target, spec, desired_hash, |from, to| fs::rename(from, to))
}

fn replace_managed_directory_with(
    target: &Path,
    spec: &SkillSpec,
    desired_hash: &str,
    rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| std::io::Error::new(ErrorKind::InvalidInput, "target has no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary = Builder::new()
        .prefix(".browserclaw-skill-")
        .tempdir_in(parent)?;
    fs::write(temporary.path().join("SKILL.md"), &spec.content)?;
    let marker = OwnershipMarker::new(&spec.name, desired_hash);
    let marker = marker_content(&marker).map_err(std::io::Error::other)?;
    fs::write(temporary.path().join(MARKER_FILE), marker)?;
    let prepared = temporary.keep();
    let backup = sibling_backup_path(target);
    let result = swap_directories_with(&prepared, target, &backup, rename, remove_path);
    if result.is_err() && fs::symlink_metadata(&prepared).is_ok() {
        let _ = remove_path(&prepared);
    }
    result
}

fn swap_directories_with(
    prepared: &Path,
    target: &Path,
    backup: &Path,
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
    mut remove: impl FnMut(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let target_exists = fs::symlink_metadata(target).is_ok();
    if target_exists {
        rename(target, backup)?;
    }
    if let Err(replace_error) = rename(prepared, target) {
        if !target_exists {
            return Err(replace_error);
        }
        return match rename(backup, target) {
            Ok(()) => Err(replace_error),
            Err(restore_error) => Err(std::io::Error::other(format!(
                "replace failed: {replace_error}; restore failed: {restore_error}"
            ))),
        };
    }
    if target_exists && let Err(cleanup_error) = remove(backup) {
        // The prior target stays authoritative until its backup is gone so
        // a reported failure cannot leave the filesystem ahead of the ledger.
        if let Err(move_new_error) = rename(target, prepared) {
            return Err(std::io::Error::other(format!(
                "backup cleanup failed: {cleanup_error}; could not prepare rollback: {move_new_error}"
            )));
        }
        if let Err(restore_error) = rename(backup, target) {
            let reapply = rename(prepared, target);
            return Err(std::io::Error::other(match reapply {
                Ok(()) => format!(
                    "backup cleanup failed: {cleanup_error}; restore failed: {restore_error}; replacement was reapplied"
                ),
                Err(reapply_error) => format!(
                    "backup cleanup failed: {cleanup_error}; restore failed: {restore_error}; reapplying replacement failed: {reapply_error}"
                ),
            }));
        }
        return Err(cleanup_error);
    }
    Ok(())
}

fn sibling_backup_path(target: &Path) -> PathBuf {
    let file_name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("skill");
    target.with_file_name(format!(
        ".{file_name}.browserclaw-backup-{}-{}",
        std::process::id(),
        monotonic_nonce()
    ))
}

fn remove_path(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn read_content_hash(path: &Path) -> Result<Option<String>, Error> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => return Ok(None),
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::io("inspect", path, error)),
    }
    match fs::read(path) {
        Ok(content) => Ok(Some(content_hash(&content))),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::io("read", path, error)),
    }
}

fn content_hash(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

fn monotonic_nonce() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, fs, path::Path};

    use tempfile::tempdir;

    use crate::{AgentId, SkillEnvironment, SkillSpec, TargetPlatform};

    use super::{
        SkillReconciler, physical_skill_target, replace_managed_directory,
        replace_managed_directory_with, resolve_agent_skill_target, swap_directories_with,
    };

    #[test]
    fn identity_planning_groups_aliases_without_platform_symlinks()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let home = root.path().join("home");
        let state = root.path().join("state");
        let environment = SkillEnvironment::new(&home, TargetPlatform::Linux);
        let reconciler = SkillReconciler::new(&state);
        let claude = resolve_agent_skill_target(AgentId::ClaudeCode, "browserclaw", &environment)?;
        let agents = resolve_agent_skill_target(AgentId::Codex, "browserclaw", &environment)?;
        let physical = home.join(".skills/browserclaw");
        let identity = |target: &Path| {
            Ok(if target == claude || target == agents {
                physical.clone()
            } else {
                target.to_path_buf()
            })
        };
        let spec = SkillSpec::new("browserclaw", "managed\n")?;

        let installed = reconciler.reconcile_with_identity(
            &spec,
            &BTreeSet::from([AgentId::ClaudeCode]),
            &environment,
            identity,
            replace_managed_directory,
        )?;
        assert_eq!(installed.installed, 1);
        assert_eq!(installed.removed, 0);
        let modified = fs::metadata(physical.join("SKILL.md"))?.modified()?;

        let shared = reconciler.reconcile_with_identity(
            &spec,
            &BTreeSet::from([AgentId::ClaudeCode, AgentId::Codex]),
            &environment,
            |target| {
                Ok(if target == claude || target == agents {
                    physical.clone()
                } else {
                    target.to_path_buf()
                })
            },
            replace_managed_directory,
        )?;
        assert_eq!(shared.unchanged, 1);
        assert_eq!(
            fs::metadata(physical.join("SKILL.md"))?.modified()?,
            modified
        );

        let removed = reconciler.reconcile_with_identity(
            &spec,
            &BTreeSet::new(),
            &environment,
            |target| {
                Ok(if target == claude || target == agents {
                    physical.clone()
                } else {
                    target.to_path_buf()
                })
            },
            replace_managed_directory,
        )?;
        assert_eq!(removed.removed, 1);
        assert!(!physical.exists());
        Ok(())
    }

    #[test]
    fn identity_failure_precedes_all_directory_mutation() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = tempdir()?;
        let environment = SkillEnvironment::new(root.path().join("home"), TargetPlatform::Linux);
        let reconciler = SkillReconciler::new(root.path().join("state"));
        let spec = SkillSpec::new("browserclaw", "managed\n")?;
        let mut identity_calls = 0;
        let mut replace_calls = 0;

        let error = reconciler
            .reconcile_with_identity(
                &spec,
                &BTreeSet::from([AgentId::Cursor]),
                &environment,
                |target| {
                    identity_calls += 1;
                    if identity_calls == 3 {
                        return Err(crate::Error::io(
                            "resolve filesystem identity for",
                            target,
                            std::io::Error::other("injected identity failure"),
                        ));
                    }
                    Ok(target.to_path_buf())
                },
                |_, _, _| {
                    replace_calls += 1;
                    Ok(())
                },
            )
            .err()
            .ok_or("identity failure unexpectedly reconciled")?;

        assert!(error.to_string().contains("injected identity failure"));
        assert_eq!(replace_calls, 0);
        assert!(!root.path().join("state/skills.json").exists());
        Ok(())
    }

    #[test]
    fn missing_skill_roots_are_planned_without_creating_directories()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let missing_root = root.path().join("missing/nested/skills");
        let target = missing_root.join("browserclaw");

        let physical = physical_skill_target(&target)?;

        assert_eq!(
            physical,
            fs::canonicalize(root.path())?.join("missing/nested/skills/browserclaw")
        );
        assert!(!missing_root.exists());
        Ok(())
    }

    #[test]
    fn relative_skill_roots_resolve_from_the_process_directory()
    -> Result<(), Box<dyn std::error::Error>> {
        let target = Path::new("relative-home/.agents/skills/browserclaw");

        let physical = physical_skill_target(target)?;

        assert_eq!(
            physical,
            fs::canonicalize(std::env::current_dir()?)?
                .join("relative-home/.agents/skills/browserclaw")
        );
        assert!(!Path::new("relative-home").exists());
        Ok(())
    }

    #[test]
    fn failed_directory_swap_restores_the_previous_target() -> std::io::Result<()> {
        let root = tempdir()?;
        let target = root.path().join("browserclaw");
        let prepared = root.path().join("prepared");
        let backup = root.path().join("backup");
        fs::create_dir_all(&target)?;
        fs::create_dir_all(&prepared)?;
        fs::write(target.join("SKILL.md"), "old")?;
        fs::write(prepared.join("SKILL.md"), "new")?;
        let mut calls = 0;
        let error = swap_directories_with(
            &prepared,
            &target,
            &backup,
            |from, to| {
                calls += 1;
                if calls == 2 {
                    Err(std::io::Error::other("injected replace failure"))
                } else {
                    fs::rename(from, to)
                }
            },
            super::remove_path,
        )
        .err()
        .ok_or_else(|| std::io::Error::other("swap unexpectedly succeeded"))?;
        assert!(error.to_string().contains("injected replace failure"));
        assert_eq!(fs::read_to_string(target.join("SKILL.md"))?, "old");
        assert!(!backup.exists());
        Ok(())
    }

    #[test]
    fn failed_backup_cleanup_restores_the_previous_target() -> std::io::Result<()> {
        let root = tempdir()?;
        let target = root.path().join("browserclaw");
        let prepared = root.path().join("prepared");
        let backup = root.path().join("backup");
        fs::create_dir_all(&target)?;
        fs::create_dir_all(&prepared)?;
        fs::write(target.join("SKILL.md"), "old")?;
        fs::write(prepared.join("SKILL.md"), "new")?;

        let error = swap_directories_with(
            &prepared,
            &target,
            &backup,
            |from, to| fs::rename(from, to),
            |path| {
                if path == backup {
                    Err(std::io::Error::other("injected backup cleanup failure"))
                } else {
                    super::remove_path(path)
                }
            },
        )
        .err()
        .ok_or_else(|| std::io::Error::other("swap unexpectedly succeeded"))?;

        assert!(
            error
                .to_string()
                .contains("injected backup cleanup failure")
        );
        assert_eq!(fs::read_to_string(target.join("SKILL.md"))?, "old");
        assert_eq!(fs::read_to_string(prepared.join("SKILL.md"))?, "new");
        assert!(!backup.exists());
        Ok(())
    }

    #[test]
    fn failed_reconciliation_keeps_the_previous_manifest_entry()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let environment = SkillEnvironment::new(root.path().join("home"), TargetPlatform::Linux);
        let reconciler = SkillReconciler::new(root.path().join("state"));
        let consumers = BTreeSet::from([AgentId::Cursor]);
        let original = SkillSpec::new("browserclaw", "old")?;
        reconciler.reconcile(&original, &consumers, &environment)?;
        let target = resolve_agent_skill_target(AgentId::Cursor, "browserclaw", &environment)?;
        let manifest_path = root.path().join("state/skills.json");
        let manifest_before = fs::read(&manifest_path)?;

        let replacement = SkillSpec::new("browserclaw", "new")?;
        let outcome = reconciler.reconcile_with(
            &replacement,
            &consumers,
            &environment,
            |target, spec, hash| {
                let mut calls = 0;
                replace_managed_directory_with(target, spec, hash, |from, to| {
                    calls += 1;
                    if calls == 2 {
                        Err(std::io::Error::other("injected replace failure"))
                    } else {
                        fs::rename(from, to)
                    }
                })
            },
        )?;

        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(fs::read_to_string(target.join("SKILL.md"))?, "old");
        assert_eq!(fs::read(manifest_path)?, manifest_before);
        Ok(())
    }
}
