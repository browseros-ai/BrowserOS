use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::process::Git;

pub const FEATURES_FILE: &str = ".features.yaml";
pub const STORE_FILE: &str = ".store.yaml";

const LEGACY_FEATURES_FILE: &str = "features.yaml";
const LEGACY_STORE_FILE: &str = "store.yaml";

/// In-memory view of a chromium_patches store directory.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
    metadata: StoreMetadata,
    features: Features,
    patches: BTreeMap<String, PatchFile>,
    ignored_files: BTreeSet<String>,
    store_yaml: Vec<u8>,
    features_yaml: Vec<u8>,
    metadata_dirty: bool,
}

impl Store {
    /// Loads patch files, feature ownership, and base metadata from a store dir.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        let store_path = metadata_file_path(&dir, STORE_FILE, LEGACY_STORE_FILE)?;
        let features_path = metadata_file_path(&dir, FEATURES_FILE, LEGACY_FEATURES_FILE)?;
        let store_yaml =
            fs::read(&store_path).with_context(|| format!("reading {}", store_path.display()))?;
        let metadata: StoreMetadata = serde_yaml::from_slice(&store_yaml)
            .with_context(|| format!("parsing {}", store_path.display()))?;
        let features_yaml = fs::read(&features_path)
            .with_context(|| format!("reading {}", features_path.display()))?;
        let features = parse_features_yaml(&features_yaml)
            .with_context(|| format!("parsing {}", features_path.display()))?;
        let loaded = load_patch_files(&dir)?;

        Ok(Self {
            dir,
            metadata,
            features,
            patches: loaded.patches,
            ignored_files: loaded.ignored_files,
            store_yaml,
            features_yaml,
            metadata_dirty: false,
        })
    }

    /// Saves the current model back to the directory it was loaded from.
    pub fn save(&self) -> Result<()> {
        self.save_to(&self.dir)
    }

    /// Saves the current model to another store dir, preserving unchanged YAML bytes.
    pub fn save_to(&self, dir: impl AsRef<Path>) -> Result<()> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        migrate_legacy_metadata_files(dir)?;
        remove_stale_patch_files(dir, self.patches.keys())?;

        for patch in self.patches.values() {
            let path = dir.join(&patch.path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            fs::write(&path, &patch.contents)
                .with_context(|| format!("writing patch {}", path.display()))?;
        }

        fs::write(dir.join(STORE_FILE), self.store_yaml_bytes()?)
            .with_context(|| format!("writing {}", dir.join(STORE_FILE).display()))?;
        fs::write(dir.join(FEATURES_FILE), self.features_yaml_bytes())
            .with_context(|| format!("writing {}", dir.join(FEATURES_FILE).display()))?;
        Ok(())
    }

    /// Returns the base pin loaded from .store.yaml.
    pub fn metadata(&self) -> &StoreMetadata {
        &self.metadata
    }

    /// Replaces the base pin that will be written to .store.yaml.
    pub fn set_metadata(&mut self, metadata: StoreMetadata) {
        self.metadata = metadata;
        self.metadata_dirty = true;
    }

    /// Returns the parsed .features.yaml model.
    pub fn features(&self) -> &Features {
        &self.features
    }

    /// Returns the patch files keyed by chromium-relative path.
    pub fn patches(&self) -> &BTreeMap<String, PatchFile> {
        &self.patches
    }

    /// Returns store files skipped because they are not patch files.
    pub fn ignored_files(&self) -> &BTreeSet<String> {
        &self.ignored_files
    }

    /// Replaces all patch files in the model with net-diff patch entries.
    pub fn set_patches(&mut self, patches: Vec<PatchFile>) -> Result<()> {
        let mut next = BTreeMap::new();
        for patch in patches {
            validate_store_path(&patch.path)?;
            if next.insert(patch.path.clone(), patch).is_some() {
                bail!("duplicate patch path");
            }
        }
        self.patches = next;
        Ok(())
    }

    /// Appends a new stored feature block while leaving existing .features.yaml bytes intact.
    pub fn add_feature(&mut self, name: &str, description: &str, paths: Vec<String>) -> Result<()> {
        self.add_feature_with_store(name, description, paths, true)
    }

    /// Appends a new feature block while preserving the surrounding YAML bytes.
    pub fn add_feature_with_store(
        &mut self,
        name: &str,
        description: &str,
        paths: Vec<String>,
        store: bool,
    ) -> Result<()> {
        validate_feature_name(name)?;
        if self.features.features.contains_key(name) {
            bail!("feature {name} already exists");
        }
        if paths.is_empty() {
            bail!("feature {name} must own at least one path");
        }
        for path in &paths {
            validate_feature_path(path)?;
        }

        append_feature_block(&mut self.features_yaml, name, description, store, &paths)?;
        self.features = parse_features_yaml(&self.features_yaml)?;
        Ok(())
    }

    /// Appends owned paths to an existing feature without reserializing comments.
    pub fn append_feature_paths(&mut self, name: &str, paths: Vec<String>) -> Result<usize> {
        validate_feature_name(name)?;
        if paths.is_empty() {
            return Ok(0);
        }
        let Some(feature) = self.features.features.get(name) else {
            bail!("feature {name} does not exist");
        };
        let mut existing = feature.paths.iter().cloned().collect::<BTreeSet<_>>();
        let mut appended = Vec::new();
        for path in paths {
            validate_feature_path(&path)?;
            if existing.insert(path.clone()) {
                appended.push(path);
            }
        }
        if appended.is_empty() {
            return Ok(0);
        }

        append_paths_to_feature_block(&mut self.features_yaml, name, &appended)?;
        self.features = parse_features_yaml(&self.features_yaml)?;
        Ok(appended.len())
    }

    /// Resolves a chromium path to a feature match or a nearest-path suggestion.
    pub fn match_path(&self, path: &str) -> FeatureMatch {
        if let Some((feature, matched_path)) = self.exact_match(path) {
            return FeatureMatch::Matched {
                feature,
                matched_path,
            };
        }
        if let Some((feature, matched_path)) = self.prefix_match(path) {
            return FeatureMatch::Matched {
                feature,
                matched_path,
            };
        }
        FeatureMatch::Unmatched {
            suggestion: self.nearest_suggestion(path),
        }
    }

    /// Returns whether path content belongs in the patch store.
    pub fn stores_path(&self, path: &str) -> bool {
        match self.match_path(path) {
            FeatureMatch::Matched { feature, .. } => self
                .features
                .features
                .get(&feature)
                .is_none_or(|feature| feature.store),
            FeatureMatch::Unmatched { .. } => true,
        }
    }

    fn exact_match(&self, path: &str) -> Option<(String, String)> {
        self.features.features.iter().find_map(|(name, feature)| {
            feature
                .paths
                .iter()
                .find(|candidate| !candidate.ends_with('/') && candidate.as_str() == path)
                .map(|candidate| (name.clone(), candidate.clone()))
        })
    }

    fn prefix_match(&self, path: &str) -> Option<(String, String)> {
        let mut best: Option<(usize, String, String)> = None;
        for (name, feature) in &self.features.features {
            for candidate in &feature.paths {
                if candidate.ends_with('/') && path.starts_with(candidate) {
                    let score = candidate.len();
                    if best
                        .as_ref()
                        .is_none_or(|(best_score, _, _)| score > *best_score)
                    {
                        best = Some((score, name.clone(), candidate.clone()));
                    }
                }
            }
        }
        best.map(|(_, feature, matched_path)| (feature, matched_path))
    }

    fn nearest_suggestion(&self, path: &str) -> FeatureSuggestion {
        if let Some(parent) = parent_dir(path) {
            for (name, feature) in &self.features.features {
                if feature.paths.iter().any(|candidate| {
                    let trimmed = candidate.trim_end_matches('/');
                    parent_dir(trimmed).as_deref() == Some(parent.as_str())
                }) {
                    return FeatureSuggestion::ExistingFeature(name.clone());
                }
            }
            if let Some(segment) = parent
                .rsplit('/')
                .next()
                .filter(|segment| !segment.is_empty())
            {
                return FeatureSuggestion::NewFeature(segment.to_string());
            }
        }

        let fallback = path
            .rsplit('/')
            .next()
            .and_then(|name| name.split('.').next())
            .filter(|name| !name.is_empty())
            .unwrap_or("feature");
        FeatureSuggestion::NewFeature(fallback.to_string())
    }

    fn store_yaml_bytes(&self) -> Result<Vec<u8>> {
        if self.metadata_dirty || self.store_yaml.is_empty() {
            return Ok(serde_yaml::to_string(&self.metadata)?.into_bytes());
        }
        Ok(self.store_yaml.clone())
    }

    fn features_yaml_bytes(&self) -> Vec<u8> {
        self.features_yaml.clone()
    }
}

/// Validates that a store directory has one readable spelling for each metadata file.
pub fn validate_metadata_layout(dir: impl AsRef<Path>) -> Result<()> {
    let dir = dir.as_ref();
    metadata_file_path(dir, STORE_FILE, LEGACY_STORE_FILE)?;
    metadata_file_path(dir, FEATURES_FILE, LEGACY_FEATURES_FILE)?;
    Ok(())
}

/// .store.yaml schema: base Chromium commit plus human Chromium version string.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoreMetadata {
    pub base_commit: String,
    pub base_version: String,
}

/// Parsed .features.yaml content keyed by feature name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Features {
    pub version: Option<String>,
    pub features: BTreeMap<String, Feature>,
}

/// A feature's commit description and owned chromium paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Feature {
    pub name: String,
    pub description: String,
    pub paths: Vec<String>,
    pub store: bool,
}

/// One per-file unified diff stored under its chromium-relative path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatchFile {
    pub path: String,
    pub contents: Vec<u8>,
}

/// Result of routing one changed chromium path through .features.yaml.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FeatureMatch {
    Matched {
        feature: String,
        matched_path: String,
    },
    Unmatched {
        suggestion: FeatureSuggestion,
    },
}

/// Suggested routing for an unmatched chromium path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FeatureSuggestion {
    ExistingFeature(String),
    NewFeature(String),
}

/// Generates per-file net patches by diffing two trees, never walking history.
pub fn generate_net_patches(
    repo: impl AsRef<Path>,
    base_treeish: &str,
    target_treeish: &str,
) -> Result<Vec<PatchFile>> {
    let git = Git::new(repo.as_ref());
    let diff = git.run(&[
        "diff",
        "--binary",
        "--full-index",
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        base_treeish,
        target_treeish,
        "--",
    ])?;
    if diff.is_empty() {
        return Ok(Vec::new());
    }

    let names = git.run(&[
        "diff",
        "--name-only",
        "-z",
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        base_treeish,
        target_treeish,
        "--",
    ])?;
    let paths = parse_nul_paths(&names)?;
    let chunks = split_diff(&diff)?;
    if paths.len() != chunks.len() {
        bail!(
            "git diff produced {} paths but {} patch hunks",
            paths.len(),
            chunks.len()
        );
    }

    paths
        .into_iter()
        .zip(chunks)
        .map(|(path, contents)| {
            validate_store_path(&path)?;
            Ok(PatchFile { path, contents })
        })
        .collect()
}

#[derive(Debug, Deserialize)]
struct FeaturesYaml {
    version: Option<String>,
    features: Option<BTreeMap<String, FeatureYaml>>,
}

#[derive(Debug, Deserialize)]
struct FeatureYaml {
    description: Option<String>,
    store: Option<bool>,
    files: Option<Vec<String>>,
}

fn parse_features_yaml(bytes: &[u8]) -> Result<Features> {
    let parsed: FeaturesYaml = serde_yaml::from_slice(bytes)?;
    let mut features = BTreeMap::new();
    for (name, feature) in parsed.features.unwrap_or_default() {
        let paths = feature.files.unwrap_or_default();
        for path in &paths {
            validate_feature_path(path)?;
        }
        features.insert(
            name.clone(),
            Feature {
                name,
                description: feature.description.unwrap_or_default(),
                store: feature.store.unwrap_or(true),
                paths,
            },
        );
    }
    Ok(Features {
        version: parsed.version,
        features,
    })
}

struct LoadedPatchFiles {
    patches: BTreeMap<String, PatchFile>,
    ignored_files: BTreeSet<String>,
}

fn load_patch_files(dir: &Path) -> Result<LoadedPatchFiles> {
    let mut loaded = LoadedPatchFiles {
        patches: BTreeMap::new(),
        ignored_files: BTreeSet::new(),
    };
    collect_patch_files(dir, dir, &mut loaded)?;
    Ok(loaded)
}

fn collect_patch_files(root: &Path, dir: &Path, loaded: &mut LoadedPatchFiles) -> Result<()> {
    let mut entries = fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("reading {}", dir.display()))?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_patch_files(root, &path, loaded)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        if is_root_metadata_file(root, &path) {
            continue;
        }
        let rel = relative_store_path(root, &path)?;
        let contents = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        if !contents.starts_with(b"diff --git ") {
            loaded.ignored_files.insert(rel);
            continue;
        }
        loaded.patches.insert(
            rel.clone(),
            PatchFile {
                path: rel,
                contents,
            },
        );
    }
    Ok(())
}

fn remove_stale_patch_files<'a>(
    dir: &Path,
    desired_paths: impl IntoIterator<Item = &'a String>,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    let desired = desired_paths.into_iter().cloned().collect::<BTreeSet<_>>();
    let existing = load_patch_files(dir)?;
    for path in existing
        .patches
        .keys()
        .filter(|path| !desired.contains(*path))
    {
        let full = dir.join(path);
        fs::remove_file(&full).with_context(|| format!("removing {}", full.display()))?;
        remove_empty_parents(dir, full.parent())?;
    }
    Ok(())
}

fn remove_empty_parents(root: &Path, mut dir: Option<&Path>) -> Result<()> {
    while let Some(current) = dir {
        if current == root {
            break;
        }
        if fs::read_dir(current)?.next().is_some() {
            break;
        }
        fs::remove_dir(current).with_context(|| format!("removing {}", current.display()))?;
        dir = current.parent();
    }
    Ok(())
}

fn relative_store_path(root: &Path, path: &Path) -> Result<String> {
    let rel = path
        .strip_prefix(root)
        .with_context(|| format!("{} is not under {}", path.display(), root.display()))?;
    let mut out = Vec::new();
    for component in rel.components() {
        match component {
            Component::Normal(part) => out.push(
                part.to_str()
                    .ok_or_else(|| anyhow!("store path is not UTF-8: {}", path.display()))?,
            ),
            _ => bail!("invalid store path: {}", path.display()),
        }
    }
    let joined = out.join("/");
    validate_store_path(&joined)?;
    Ok(joined)
}

fn validate_store_path(path: &str) -> Result<()> {
    if path == STORE_FILE || path == FEATURES_FILE {
        bail!(
            "store path {path} is reserved for bpatch metadata; choose a Chromium-relative patch path"
        );
    }
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        bail!("invalid store path: {path}");
    }
    Ok(())
}

fn metadata_file_path(dir: &Path, current: &str, legacy: &str) -> Result<PathBuf> {
    let current_path = dir.join(current);
    let legacy_path = dir.join(legacy);
    match (current_path.exists(), legacy_path.exists()) {
        (true, true) => bail!(
            "patch store {} has both {current} and legacy {legacy}; remove one spelling before running bpatch",
            dir.display()
        ),
        (true, false) => Ok(current_path),
        (false, true) => Ok(legacy_path),
        (false, false) => bail!(
            "patch store {} is missing {current} (legacy {legacy} was not found either)",
            dir.display()
        ),
    }
}

fn migrate_legacy_metadata_files(dir: &Path) -> Result<()> {
    migrate_legacy_metadata_file(dir, STORE_FILE, LEGACY_STORE_FILE)?;
    migrate_legacy_metadata_file(dir, FEATURES_FILE, LEGACY_FEATURES_FILE)
}

fn migrate_legacy_metadata_file(dir: &Path, current: &str, legacy: &str) -> Result<()> {
    let current_path = dir.join(current);
    let legacy_path = dir.join(legacy);
    match (current_path.exists(), legacy_path.exists()) {
        (true, true) => bail!(
            "patch store {} has both {current} and legacy {legacy}; remove one spelling before running bpatch",
            dir.display()
        ),
        (false, true) => fs::rename(&legacy_path, &current_path).with_context(|| {
            format!(
                "migrating {} to {}",
                legacy_path.display(),
                current_path.display()
            )
        }),
        _ => Ok(()),
    }
}

fn is_root_metadata_file(root: &Path, path: &Path) -> bool {
    path.parent() == Some(root)
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                matches!(
                    name,
                    FEATURES_FILE | STORE_FILE | LEGACY_FEATURES_FILE | LEGACY_STORE_FILE
                )
            })
}

fn validate_feature_path(path: &str) -> Result<()> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        bail!("invalid feature path: {path}");
    }
    if path.split('/').any(|part| part == "." || part == "..") {
        bail!("invalid feature path: {path}");
    }
    Ok(())
}

fn validate_feature_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("invalid feature name: {name}");
    }
    Ok(())
}

fn append_feature_block(
    yaml: &mut Vec<u8>,
    name: &str,
    description: &str,
    store: bool,
    paths: &[String],
) -> Result<()> {
    if yaml.is_empty() {
        yaml.extend_from_slice(b"version: \"1.0\"\nfeatures:\n");
    }
    if !yaml.ends_with(b"\n") {
        yaml.push(b'\n');
    }

    let mut block = String::new();
    block.push('\n');
    block.push_str("  ");
    block.push_str(name);
    block.push_str(":\n");
    block.push_str("    description: ");
    block.push_str(&yaml_string(description));
    block.push('\n');
    if !store {
        block.push_str("    store: false\n");
    }
    block.push_str("    files:\n");
    for path in paths {
        block.push_str("      - ");
        block.push_str(&yaml_string(path));
        block.push('\n');
    }
    yaml.extend_from_slice(block.as_bytes());
    Ok(())
}

fn append_paths_to_feature_block(yaml: &mut Vec<u8>, name: &str, paths: &[String]) -> Result<()> {
    let text = std::str::from_utf8(yaml).context(".features.yaml is not UTF-8")?;
    let lines = text
        .split_inclusive('\n')
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let header = format!("  {name}:");
    let feature_start = lines
        .iter()
        .position(|line| line.trim_end() == header)
        .ok_or_else(|| anyhow!("feature {name} does not exist in .features.yaml bytes"))?;
    let feature_end = lines
        .iter()
        .enumerate()
        .skip(feature_start + 1)
        .find_map(|(index, line)| {
            let trimmed = line.trim_end();
            (trimmed.starts_with("  ") && !trimmed.starts_with("    ")).then_some(index)
        })
        .unwrap_or(lines.len());
    let files_line = lines[feature_start + 1..feature_end]
        .iter()
        .position(|line| line.trim() == "files:")
        .map(|offset| feature_start + 1 + offset)
        .ok_or_else(|| anyhow!("feature {name} has no files list in .features.yaml"))?;
    let insert_at = lines
        .iter()
        .enumerate()
        .take(feature_end)
        .skip(files_line + 1)
        .find_map(|(index, line)| {
            let trimmed = line.trim_start();
            (!(trimmed.starts_with("- ") || trimmed.starts_with('#') || trimmed.trim().is_empty()))
                .then_some(index)
        })
        .unwrap_or(feature_end);

    let mut next = String::new();
    for line in &lines[..insert_at] {
        next.push_str(line);
    }
    for path in paths {
        next.push_str("      - ");
        next.push_str(&yaml_string(path));
        next.push('\n');
    }
    for line in &lines[insert_at..] {
        next.push_str(line);
    }
    *yaml = next.into_bytes();
    Ok(())
}

fn yaml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn split_diff(diff: &[u8]) -> Result<Vec<Vec<u8>>> {
    if !diff.starts_with(b"diff --git ") {
        bail!("git diff output did not start with a diff header");
    }

    let mut starts = vec![0];
    let needle = b"\ndiff --git ";
    let mut cursor = 0;
    while let Some(offset) = find_bytes(&diff[cursor..], needle) {
        let start = cursor + offset + 1;
        starts.push(start);
        cursor = start;
    }

    let mut chunks = Vec::with_capacity(starts.len());
    for (index, start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(diff.len());
        chunks.push(diff[*start..end].to_vec());
    }
    Ok(chunks)
}

fn parse_nul_paths(bytes: &[u8]) -> Result<Vec<String>> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8(part.to_vec()).context("git diff path output was not UTF-8"))
        .collect()
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn parent_dir(path: &str) -> Option<String> {
    path.rsplit_once('/')
        .map(|(parent, _)| parent)
        .filter(|parent| !parent.is_empty())
        .map(ToOwned::to_owned)
}
