use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bpatch::process::Git;
use bpatch::store::{self, FeatureMatch, FeatureSuggestion, Store, StoreMetadata};
use tempfile::TempDir;

fn write_file(path: impl AsRef<Path>, bytes: &[u8]) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

fn make_executable(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(permissions.mode() | 0o111);
    fs::set_permissions(path, permissions).with_context(|| format!("chmod {}", path.display()))
}

fn init_repo() -> Result<(TempDir, Git)> {
    let dir = tempfile::tempdir()?;
    let git = Git::new(dir.path());
    git.run(&["init"])?;
    git.run(&["config", "user.email", "bpatch@example.test"])?;
    git.run(&["config", "user.name", "bpatch test"])?;
    git.run(&["config", "core.filemode", "true"])?;
    Ok((dir, git))
}

fn commit_paths(git: &Git, paths: &[&str], message: &str) -> Result<String> {
    let mut add_args = vec!["add", "--"];
    add_args.extend_from_slice(paths);
    git.run(&add_args)?;
    git.run(&["commit", "-m", message])?;
    git.run_str(&["rev-parse", "HEAD"])
}

fn write_minimal_store(dir: &Path) -> Result<()> {
    write_file(
        dir.join("base/version_info/BUILD.gn"),
        b"diff --git a/base/version_info/BUILD.gn b/base/version_info/BUILD.gn\nindex 1111111..2222222 100644\n--- a/base/version_info/BUILD.gn\n+++ b/base/version_info/BUILD.gn\n@@ -1 +1 @@\n-old\n+new\n",
    )?;
    write_file(
        dir.join(".features.yaml"),
        br#"version: "1.0"
features:
  # preserved comment
  llmchat:
    description: "feat: llm chat"
    files:
      - chrome/browser/ui/llmchat/panel.cc
      - chrome/browser/ui/llmchat/panel.h
  settings:
    description: "feat: settings"
    files:
      - chrome/browser/resources/settings/
  bootstrap:
    description: "chore: bootstrap"
    files:
      - chrome/browser/browseros/BUILD.gn
"#,
    )?;
    write_file(
        dir.join(".store.yaml"),
        br#"base_commit: 6b3fa66a923a9442c8ab0bc71b4b41ff24528d3b
base_version: "148.0.7778.97"
"#,
    )
}

fn count_store_patch_files(root: &Path) -> Result<usize> {
    fn visit(root: &Path, dir: &Path) -> Result<usize> {
        let mut count = 0;
        for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry.with_context(|| format!("reading {}", dir.display()))?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                count += visit(root, &path)?;
                continue;
            }
            if !file_type.is_file() {
                continue;
            }

            let rel = path.strip_prefix(root)?;
            if rel == Path::new(".features.yaml") || rel == Path::new(".store.yaml") {
                continue;
            }
            if fs::read(&path)
                .with_context(|| format!("reading {}", path.display()))?
                .starts_with(b"diff --git ")
            {
                count += 1;
            }
        }
        Ok(count)
    }

    visit(root, root)
}

#[test]
fn round_trip_store_preserves_patch_and_metadata_bytes() -> Result<()> {
    let src = tempfile::tempdir()?;
    write_minimal_store(src.path())?;

    let store = Store::load(src.path())?;
    assert_eq!(
        store.metadata(),
        &StoreMetadata {
            base_commit: "6b3fa66a923a9442c8ab0bc71b4b41ff24528d3b".to_string(),
            base_version: "148.0.7778.97".to_string(),
        }
    );
    assert_eq!(store.patches().len(), 1);
    assert_eq!(
        store
            .patches()
            .get("base/version_info/BUILD.gn")
            .expect("patch")
            .contents,
        fs::read(src.path().join("base/version_info/BUILD.gn"))?
    );

    let dst = tempfile::tempdir()?;
    store.save_to(dst.path())?;

    assert_eq!(
        fs::read(src.path().join("base/version_info/BUILD.gn"))?,
        fs::read(dst.path().join("base/version_info/BUILD.gn"))?
    );
    assert_eq!(
        fs::read(src.path().join(".store.yaml"))?,
        fs::read(dst.path().join(".store.yaml"))?
    );
    assert_eq!(
        fs::read(src.path().join(".features.yaml"))?,
        fs::read(dst.path().join(".features.yaml"))?
    );
    Ok(())
}

#[test]
fn adding_feature_appends_without_rewriting_existing_comments() -> Result<()> {
    let dir = tempfile::tempdir()?;
    write_minimal_store(dir.path())?;
    let original = fs::read(dir.path().join(".features.yaml"))?;

    let mut store = Store::load(dir.path())?;
    store.add_feature(
        "wallet",
        "feat: browseros wallet",
        vec!["chrome/browser/browseros/wallet/".to_string()],
    )?;
    store.save()?;

    let updated = fs::read(dir.path().join(".features.yaml"))?;
    assert!(updated.starts_with(&original));

    let reparsed = Store::load(dir.path())?;
    let wallet = reparsed
        .features()
        .features
        .get("wallet")
        .expect("wallet feature");
    assert_eq!(wallet.description, "feat: browseros wallet");
    assert_eq!(wallet.paths, vec!["chrome/browser/browseros/wallet/"]);
    Ok(())
}

#[test]
fn feature_store_false_defaults_and_appends_preserve_yaml() -> Result<()> {
    let dir = tempfile::tempdir()?;
    write_minimal_store(dir.path())?;
    let original = fs::read(dir.path().join(".features.yaml"))?;

    let mut store = Store::load(dir.path())?;
    assert!(
        store
            .features()
            .features
            .get("llmchat")
            .expect("llmchat")
            .store
    );
    store.add_feature_with_store(
        "build-resources",
        "resource: bos_build outputs",
        vec!["chrome/app/theme/chromium/".to_string()],
        false,
    )?;
    store.append_feature_paths(
        "build-resources",
        vec!["chrome/BROWSEROS_VERSION".to_string()],
    )?;
    store.save()?;

    let updated = fs::read(dir.path().join(".features.yaml"))?;
    assert!(updated.starts_with(&original));
    let text = String::from_utf8(updated)?;
    assert!(text.contains("  build-resources:"));
    assert!(text.contains("    store: false"));
    assert!(text.contains("      - \"chrome/app/theme/chromium/\""));
    assert!(text.contains("      - \"chrome/BROWSEROS_VERSION\""));

    let reparsed = Store::load(dir.path())?;
    let feature = reparsed
        .features()
        .features
        .get("build-resources")
        .expect("build-resources");
    assert!(!feature.store);
    assert!(!reparsed.stores_path("chrome/app/theme/chromium/logo.png"));
    assert!(reparsed.stores_path("chrome/browser/ui/llmchat/panel.cc"));
    Ok(())
}

#[test]
fn canonical_store_loads_build_resources_as_unstored() -> Result<()> {
    let store_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("chromium_patches");
    let store = Store::load(&store_dir)?;
    let feature = store
        .features()
        .features
        .get("build-resources")
        .expect("build-resources");
    let expected = vec![
        "chrome/BROWSEROS_VERSION",
        "chrome/VERSION",
        "chrome/app/chromium_strings.grd",
        "chrome/app/settings_chromium_strings.grdp",
        "chrome/app/theme/chromium/",
        "chrome/app/theme/default_100_percent/chromium/",
        "chrome/app/theme/default_200_percent/chromium/",
        "chrome/browser/browseros/claw_server/resources/",
        "chrome/browser/browseros/server/resources/",
        "chrome/browser/browseros/onboarding/resources/",
        "chrome/enterprise_companion/branding.gni",
        "chrome/updater/branding.gni",
    ];

    assert_eq!(feature.description, "resource: bos_build outputs");
    assert!(!feature.store);
    assert_eq!(feature.paths, expected);
    assert!(!store.stores_path("chrome/app/theme/chromium/product_logo.png"));
    assert!(!store.stores_path("chrome/browser/browseros/server/resources/bin/browseros_server"));
    assert!(!store.stores_path("chrome/browser/browseros/claw_server/resources/bin/server"));
    assert!(store.stores_path("chrome/browser/browseros/claw_server/BUILD.gn"));
    Ok(())
}

#[test]
fn non_patch_files_are_ignored_and_preserved_on_save() -> Result<()> {
    let dir = tempfile::tempdir()?;
    write_minimal_store(dir.path())?;
    write_file(dir.path().join(".DS_Store"), b"finder metadata")?;
    write_file(dir.path().join("README.md"), b"# patch store\n")?;

    let store = Store::load(dir.path())?;

    assert_eq!(store.patches().len(), 1);
    assert!(!store.patches().contains_key(".DS_Store"));
    assert!(!store.patches().contains_key("README.md"));
    assert_eq!(
        store.ignored_files(),
        &BTreeSet::from([".DS_Store".to_string(), "README.md".to_string()])
    );
    store.save()?;
    assert_eq!(fs::read(dir.path().join(".DS_Store"))?, b"finder metadata");
    assert_eq!(fs::read(dir.path().join("README.md"))?, b"# patch store\n");
    Ok(())
}

#[test]
fn net_diff_generation_covers_forms_and_tree_foldings() -> Result<()> {
    let (repo, git) = init_repo()?;
    let root = repo.path();

    write_file(root.join("modify.txt"), b"old\n")?;
    write_file(root.join("delete.txt"), b"gone\n")?;
    write_file(root.join("mode.sh"), b"#!/bin/sh\necho mode\n")?;
    write_file(root.join("rename.txt"), b"rename me\n")?;
    write_file(root.join("binary.bin"), &[0, 1, 2, 3, 0, 255])?;
    write_file(root.join("reverted.txt"), b"same\n")?;
    let base = commit_paths(
        &git,
        &[
            "modify.txt",
            "delete.txt",
            "mode.sh",
            "rename.txt",
            "binary.bin",
            "reverted.txt",
        ],
        "base",
    )?;

    write_file(root.join("modify.txt"), b"new\n")?;
    write_file(root.join("new.txt"), b"created\n")?;
    fs::remove_file(root.join("delete.txt"))?;
    make_executable(root.join("mode.sh"))?;
    fs::rename(root.join("rename.txt"), root.join("renamed.txt"))?;
    write_file(root.join("binary.bin"), &[0, 9, 8, 7, 0, 255, 1])?;
    write_file(root.join("transient.txt"), b"created then deleted\n")?;
    fs::remove_file(root.join("transient.txt"))?;
    write_file(root.join("reverted.txt"), b"changed\n")?;
    write_file(root.join("reverted.txt"), b"same\n")?;
    let target = commit_paths(
        &git,
        &[
            "modify.txt",
            "new.txt",
            "delete.txt",
            "mode.sh",
            "rename.txt",
            "renamed.txt",
            "binary.bin",
        ],
        "target",
    )?;

    let patches = store::generate_net_patches(root, &base, &target)?;
    let paths = patches
        .iter()
        .map(|patch| patch.path.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        paths,
        BTreeSet::from([
            "binary.bin",
            "delete.txt",
            "mode.sh",
            "modify.txt",
            "new.txt",
            "rename.txt",
            "renamed.txt",
        ])
    );
    assert!(!paths.contains("transient.txt"));
    assert!(!paths.contains("reverted.txt"));

    let patch_text = |path: &str| -> String {
        String::from_utf8_lossy(
            &patches
                .iter()
                .find(|patch| patch.path == path)
                .expect("patch")
                .contents,
        )
        .into_owned()
    };
    assert!(patch_text("modify.txt").contains("-old\n+new\n"));
    assert!(patch_text("new.txt").contains("new file mode"));
    assert!(patch_text("delete.txt").contains("deleted file mode"));
    assert!(patch_text("mode.sh").contains("old mode 100644"));
    assert!(patch_text("mode.sh").contains("new mode 100755"));
    assert!(patch_text("rename.txt").contains("deleted file mode"));
    assert!(patch_text("renamed.txt").contains("new file mode"));
    assert!(patch_text("binary.bin").contains("GIT binary patch"));
    Ok(())
}

#[test]
fn feature_matching_exact_prefix_and_nearest_suggestions() -> Result<()> {
    let dir = tempfile::tempdir()?;
    write_minimal_store(dir.path())?;
    let store = Store::load(dir.path())?;

    assert_eq!(
        store.match_path("chrome/browser/ui/llmchat/panel.cc"),
        FeatureMatch::Matched {
            feature: "llmchat".to_string(),
            matched_path: "chrome/browser/ui/llmchat/panel.cc".to_string(),
        }
    );
    assert_eq!(
        store.match_path("chrome/browser/resources/settings/page.ts"),
        FeatureMatch::Matched {
            feature: "settings".to_string(),
            matched_path: "chrome/browser/resources/settings/".to_string(),
        }
    );
    assert_eq!(
        store.match_path("chrome/browser/ui/llmchat/resize_util.cc"),
        FeatureMatch::Unmatched {
            suggestion: FeatureSuggestion::ExistingFeature("llmchat".to_string()),
        }
    );
    assert_eq!(
        store.match_path("chrome/browser/browseros/wallet/service.cc"),
        FeatureMatch::Unmatched {
            suggestion: FeatureSuggestion::NewFeature("wallet".to_string()),
        }
    );
    Ok(())
}

#[test]
fn real_store_loads_when_seeded() -> Result<()> {
    let store_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../chromium_patches");
    if !store_dir.join(".store.yaml").exists() || !store_dir.join(".features.yaml").exists() {
        return Ok(());
    }

    let store = Store::load(&store_dir)?;
    assert_eq!(store.patches().len(), count_store_patch_files(&store_dir)?);
    assert!(!store.metadata().base_commit.is_empty());
    assert!(!store.metadata().base_version.is_empty());
    assert!(store.features().features.contains_key("browseros-core"));
    Ok(())
}
