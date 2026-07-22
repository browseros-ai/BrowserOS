use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tempfile::NamedTempFile;

use crate::{AgentId, Error};

pub(crate) const MANIFEST_FILE: &str = "skills.json";
pub(crate) const MARKER_FILE: &str = ".browserclaw-managed.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SkillManifest {
    pub(crate) version: u8,
    pub(crate) targets: Vec<SkillManifestEntry>,
}

impl Default for SkillManifest {
    fn default() -> Self {
        Self {
            version: 1,
            targets: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkillManifestEntry {
    pub(crate) target_path: PathBuf,
    pub(crate) skill_name: String,
    pub(crate) content_hash: String,
    pub(crate) consumers: Vec<AgentId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OwnershipMarker {
    pub(crate) version: u8,
    pub(crate) managed_by: String,
    pub(crate) skill_name: String,
    pub(crate) content_hash: String,
}

impl OwnershipMarker {
    pub(crate) fn new(skill_name: &str, content_hash: &str) -> Self {
        Self {
            version: 1,
            managed_by: "browserclaw".to_string(),
            skill_name: skill_name.to_string(),
            content_hash: content_hash.to_string(),
        }
    }

    pub(crate) fn controls(&self, skill_name: &str) -> bool {
        self.version == 1 && self.managed_by == "browserclaw" && self.skill_name == skill_name
    }
}

pub(crate) fn read_manifest(workspace_dir: &Path) -> Result<SkillManifest, Error> {
    let path = workspace_dir.join(MANIFEST_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(SkillManifest::default()),
        Err(error) => return Err(Error::io("read", path, error)),
    };
    let value: Value = serde_json::from_str(&raw).map_err(|_| Error::Manifest {
        message: format!(
            "Skill manifest at {} is not valid JSON. Inspect and repair or delete it to start fresh.",
            path.display()
        ),
    })?;
    let version = value.get("version").and_then(Value::as_u64);
    if version != Some(1) {
        return Err(Error::Manifest {
            message: format!(
                "Skill manifest at {} has unsupported version {}; expected 1.",
                path.display(),
                version.map_or_else(|| "undefined".to_string(), |value| value.to_string())
            ),
        });
    }
    if !value.get("targets").is_some_and(Value::is_array) {
        return Err(Error::Manifest {
            message: format!(
                "Skill manifest at {} is missing a valid `targets` array.",
                path.display()
            ),
        });
    }
    serde_json::from_value(value).map_err(|error| Error::Manifest {
        message: format!(
            "Skill manifest at {} has invalid schema: {error}.",
            path.display()
        ),
    })
}

pub(crate) fn write_manifest(workspace_dir: &Path, manifest: &SkillManifest) -> Result<(), Error> {
    let path = workspace_dir.join(MANIFEST_FILE);
    let content = serialize_json(manifest, "skill manifest")?;
    atomic_write_file(&path, content.as_bytes())
}

pub(crate) fn read_marker(target: &Path) -> Result<Option<OwnershipMarker>, Error> {
    let path = target.join(MARKER_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::io("read", path, error)),
    };
    Ok(serde_json::from_str(&raw).ok())
}

pub(crate) fn marker_content(marker: &OwnershipMarker) -> Result<String, Error> {
    serialize_json(marker, "skill ownership marker")
}

fn serialize_json(value: &impl Serialize, description: &str) -> Result<String, Error> {
    serde_json::to_string_pretty(value)
        .map(|serialized| format!("{serialized}\n"))
        .map_err(|error| Error::Manifest {
            message: format!("Could not serialize {description}: {error}"),
        })
}

fn atomic_write_file(path: &Path, content: &[u8]) -> Result<(), Error> {
    let parent = path.parent().ok_or_else(|| {
        Error::io(
            "create parent directory for",
            path,
            std::io::Error::new(ErrorKind::InvalidInput, "path has no parent directory"),
        )
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| Error::io("create parent directory for", path, error))?;
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|error| Error::io("create temporary file for", path, error))?;
    temporary
        .write_all(content)
        .map_err(|error| Error::io("write temporary file for", path, error))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| Error::io("sync temporary file for", path, error))?;
    let temporary_path = temporary.into_temp_path();
    replace_file(&temporary_path, path).map_err(|error| Error::io("replace", path, error))?;
    Ok(())
}

fn replace_file(temporary: &Path, target: &Path) -> std::io::Result<()> {
    if !target.exists() {
        return fs::rename(temporary, target);
    }
    let backup = sibling_backup_path(target);
    fs::rename(target, &backup)?;
    match fs::rename(temporary, target) {
        Ok(()) => fs::remove_file(backup),
        Err(replace_error) => match fs::rename(&backup, target) {
            Ok(()) => Err(replace_error),
            Err(restore_error) => Err(std::io::Error::other(format!(
                "replace failed: {replace_error}; restore failed: {restore_error}"
            ))),
        },
    }
}

fn sibling_backup_path(path: &Path) -> PathBuf {
    let suffix = format!("backup-{}-{}", std::process::id(), monotonic_nonce());
    path.with_extension(suffix)
}

fn monotonic_nonce() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
