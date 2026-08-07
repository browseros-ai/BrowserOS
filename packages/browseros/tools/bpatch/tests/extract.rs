mod fixtures;

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use bpatch::cli::extract::{self, ExtractMode, ExtractOptions, ExtractReportResult};
use bpatch::engine::extract::{ExtractContext, ExtractSpec, FeatureDecisionPolicy};
use bpatch::engine::lock::CheckoutLock;
use bpatch::engine::progress;
use bpatch::engine::state::{TRAILER_BASE, TRAILER_STORE_REV, TRAILER_TREE};
use bpatch::store::Store;
use fixtures::FixtureRepo;
use serde_json::Value;

#[test]
fn extract_range_writes_net_diffs_and_removes_folded_store_patch() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_base_checkout(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_store(&store, &base)?;
    let original_features = fs::read(store_dir.join(".features.yaml"))?;
    store.write_file(
        "chromium_patches/chrome/browser/ui/llmchat/tmp_probe.cc",
        b"diff --git a/chrome/browser/ui/llmchat/tmp_probe.cc b/chrome/browser/ui/llmchat/tmp_probe.cc\nnew file mode 100644\nindex abc..def 100644\n--- /dev/null\n+++ b/chrome/browser/ui/llmchat/tmp_probe.cc\n@@ -0,0 +1 @@\n+stale\n",
    )?;
    store.commit("seed store")?;

    checkout.write_file("chrome/browser/ui/llmchat/panel.cc", "panel changed\n")?;
    checkout.write_file("chrome/browser/ui/llmchat/tmp_probe.cc", "tmp\n")?;
    checkout.commit("intermediate")?;
    checkout.remove_file("chrome/browser/ui/llmchat/tmp_probe.cc")?;
    checkout.write_file("chrome/browser/ui/llmchat/resize_util.cc", "resize\n")?;
    let target = checkout.commit("final")?;

    let report = extract::run(
        &ExtractContext::new(checkout.path(), &store_dir),
        &ExtractOptions {
            mode: ExtractMode::Revs {
                spec: ExtractSpec::Range {
                    from: base,
                    to: target,
                },
                policy: FeatureDecisionPolicy::AcceptSuggestions,
            },
            commit: false,
        },
        &mut progress::noop(),
    )?;

    assert_eq!(report.result, ExtractReportResult::Extracted);
    assert_eq!(report.patches, Some(3));
    assert_eq!(report.net_folds.len(), 1);
    assert_eq!(
        report.net_folds[0].path,
        "chrome/browser/ui/llmchat/tmp_probe.cc"
    );
    assert!(
        !store_dir
            .join("chrome/browser/ui/llmchat/tmp_probe.cc")
            .exists()
    );
    assert!(
        store_dir
            .join("chrome/browser/ui/llmchat/panel.cc")
            .exists()
    );
    assert!(
        store_dir
            .join("chrome/browser/ui/llmchat/resize_util.cc")
            .exists()
    );
    assert_eq!(
        fs::read(store_dir.join(".features.yaml"))?,
        original_features
    );
    let human = extract::render_human(&report);
    assert!(human.contains("extract: 2 files changed vs base 148.0.7204.1"));
    assert!(human.contains("→ feature: llmchat (matched)"));
    assert!(human.contains("→ feature: llmchat (nearest path)"));
    assert!(human.contains("net-fold: chrome/browser/ui/llmchat/tmp_probe.cc"));
    assert!(human.contains("→ no patch"));
    assert!(human.contains(".features.yaml unchanged"));
    assert!(human.contains("next: bpatch extract --commit"));
    assert!(!store.status_porcelain()?.is_empty());
    Ok(())
}

#[test]
fn feature_routing_requires_explicit_policy_then_named_feature_appends_yaml() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_base_checkout(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_store(&store, &base)?;
    let original_features = fs::read(store_dir.join(".features.yaml"))?;
    store.commit("seed store")?;

    checkout.write_file("chrome/browser/browseros/wallet/service.cc", "wallet cc\n")?;
    checkout.write_file("chrome/browser/browseros/wallet/service.h", "wallet h\n")?;
    checkout.write_file("chrome/browser/browseros/wallet/BUILD.gn", "wallet build\n")?;
    checkout.write_file(
        "chrome/browser/browseros/BUILD.gn",
        "browseros build changed\n",
    )?;
    let rev = checkout.commit("wallet")?;

    let needs = extract::run(
        &ExtractContext::new(checkout.path(), &store_dir),
        &ExtractOptions {
            mode: ExtractMode::Revs {
                spec: ExtractSpec::Rev(rev.clone()),
                policy: FeatureDecisionPolicy::RequireExplicit,
            },
            commit: false,
        },
        &mut progress::noop(),
    )?;
    assert_eq!(needs.result, ExtractReportResult::NeedsFeature);
    assert_eq!(needs.unmatched.len(), 3);
    assert_eq!(needs.suggestion.as_deref(), Some("wallet"));
    let json = serde_json::from_str::<Value>(&extract::render_json(&needs)?)?;
    assert_eq!(json["result"], "needs-feature");
    assert_eq!(json["suggestion"], "wallet");
    assert_eq!(json["exit"], 3);
    assert_eq!(json["unmatched"].as_array().expect("unmatched").len(), 3);
    assert_eq!(store.status_porcelain()?, "");

    let routed = extract::run(
        &ExtractContext::new(checkout.path(), &store_dir),
        &ExtractOptions {
            mode: ExtractMode::Revs {
                spec: ExtractSpec::Rev(rev),
                policy: FeatureDecisionPolicy::Named("wallet".to_string()),
            },
            commit: false,
        },
        &mut progress::noop(),
    )?;
    assert_eq!(routed.result, ExtractReportResult::Extracted);
    assert_eq!(routed.patches, Some(4));
    assert_eq!(routed.new_features, vec!["wallet"]);
    let updated_features = fs::read(store_dir.join(".features.yaml"))?;
    assert!(updated_features.starts_with(&original_features));
    let store_model = Store::load(&store_dir)?;
    assert_eq!(
        store_model.features().features["wallet"].paths,
        vec!["chrome/browser/browseros/wallet/"]
    );
    let human = extract::render_human(&routed);
    assert!(human.contains("created feature \"wallet\""));
    assert!(human.contains("store: 4 patches written · .features.yaml +1 feature"));
    Ok(())
}

#[test]
fn commit_option_commits_store_repo_and_hand_commit_extraction_needs_no_trailers() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_base_checkout(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_store(&store, &base)?;
    store.commit("seed store")?;

    checkout.write_file("chrome/browser/ui/llmchat/panel.cc", "hand edit\n")?;
    let rev = checkout.commit("fix llm chat panel resize jitter")?;

    let report = extract::run(
        &ExtractContext::new(checkout.path(), &store_dir),
        &ExtractOptions {
            mode: ExtractMode::Revs {
                spec: ExtractSpec::Rev(rev),
                policy: FeatureDecisionPolicy::RequireExplicit,
            },
            commit: true,
        },
        &mut progress::noop(),
    )?;

    assert_eq!(report.result, ExtractReportResult::Extracted);
    assert_eq!(report.patches, Some(1));
    assert!(report.store_commit.is_some());
    assert_eq!(store.status_porcelain()?, "");
    let subject = store.git().run_str(&["log", "-1", "--format=%s"])?;
    assert!(subject.starts_with("feat(chromium_patches): extract "));
    Ok(())
}

#[test]
fn extract_fails_fast_when_store_lock_is_held() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_base_checkout(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_store(&store, &base)?;
    store.commit("seed store")?;
    checkout.write_file("chrome/browser/ui/llmchat/panel.cc", "locked edit\n")?;
    let rev = checkout.commit("locked edit")?;
    let _held = CheckoutLock::acquire_store_repo(&store_dir)?;

    let err = extract::run(
        &ExtractContext::new(checkout.path(), &store_dir),
        &ExtractOptions {
            mode: ExtractMode::Revs {
                spec: ExtractSpec::Rev(rev),
                policy: FeatureDecisionPolicy::RequireExplicit,
            },
            commit: false,
        },
        &mut progress::noop(),
    )
    .unwrap_err();

    let message = err.to_string();
    assert!(message.contains("lock held by pid"));
    assert!(message.contains("(started "));
    Ok(())
}

#[test]
fn extract_patch_generation_ignores_user_diff_drivers() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_base_checkout(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_store(&store, &base)?;
    store.commit("seed store")?;
    checkout
        .git()
        .run(&["config", "diff.external", "/bin/false"])?;
    checkout.write_file("chrome/browser/ui/llmchat/panel.cc", "external diff safe\n")?;
    let rev = checkout.commit("panel edit")?;

    let report = extract::run(
        &ExtractContext::new(checkout.path(), &store_dir),
        &ExtractOptions {
            mode: ExtractMode::Revs {
                spec: ExtractSpec::Rev(rev),
                policy: FeatureDecisionPolicy::RequireExplicit,
            },
            commit: false,
        },
        &mut progress::noop(),
    )?;

    assert_eq!(report.result, ExtractReportResult::Extracted);
    let patch_path = store_dir.join("chrome/browser/ui/llmchat/panel.cc");
    let patch = fs::read(&patch_path)?;
    assert!(patch.starts_with(b"diff --git "));
    checkout.git().run(&["reset", "--hard", &base])?;
    checkout
        .git()
        .run(&["apply", "--check", patch_path.to_str().expect("utf-8 path")])?;
    Ok(())
}

#[test]
fn repin_updates_base_and_preserves_semantically_unchanged_patch_bytes() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base_148 = write_repin_base(&checkout, "148.0.7204.1", "foo upstream 148\n")?;
    checkout.write_file("chrome/browser/ui/llmchat/foo.cc", "foo feature old\n")?;
    checkout.write_file("chrome/browser/ui/llmchat/keep.cc", "keep feature\n")?;
    let old_target = checkout.commit("old applied")?;

    let store = FixtureRepo::new()?;
    let store_dir = seed_store(&store, &base_148)?;
    let foo_patch = checkout.git().run(&[
        "diff",
        "--binary",
        "--full-index",
        &base_148,
        &old_target,
        "--",
        "chrome/browser/ui/llmchat/foo.cc",
    ])?;
    let mut keep_patch = checkout.git().run(&[
        "diff",
        "--binary",
        "--full-index",
        &base_148,
        &old_target,
        "--",
        "chrome/browser/ui/llmchat/keep.cc",
    ])?;
    rewrite_index_line(&mut keep_patch, b"index abc..def 100644\n")?;
    store.write_file(
        "chromium_patches/chrome/browser/ui/llmchat/foo.cc",
        foo_patch,
    )?;
    store.write_file(
        "chromium_patches/chrome/browser/ui/llmchat/keep.cc",
        keep_patch.clone(),
    )?;
    let store_rev = store.commit("seed store")?;

    checkout
        .git()
        .run(&["checkout", "-B", "base-149", &base_148])?;
    checkout.write_file(
        "chrome/VERSION",
        "MAJOR=149\nMINOR=0\nBUILD=7250\nPATCH=0\n",
    )?;
    checkout.write_file("chrome/browser/ui/llmchat/foo.cc", "foo upstream 149\n")?;
    let base_149 = checkout.commit("Chromium 149.0.7250.0")?;
    checkout.write_file("chrome/browser/ui/llmchat/foo.cc", "foo feature resolved\n")?;
    checkout.write_file("chrome/browser/ui/llmchat/keep.cc", "keep feature\n")?;
    checkout.git().run(&["add", "-A"])?;
    let applied_tree = checkout.git().run_str(&["write-tree"])?;
    checkout.commit_with_trailers(
        "feat: llmchat",
        &[
            (TRAILER_STORE_REV, store_rev.as_str()),
            (TRAILER_BASE, base_149.as_str()),
            (TRAILER_TREE, applied_tree.as_str()),
        ],
    )?;

    let report = extract::run(
        &ExtractContext::new(checkout.path(), &store_dir),
        &ExtractOptions {
            mode: ExtractMode::Repin,
            commit: false,
        },
        &mut progress::noop(),
    )?;

    assert_eq!(report.result, ExtractReportResult::Repinned);
    assert_eq!(report.rediffed, Some(2));
    assert_eq!(report.content_changed, Some(1));
    let model = Store::load(&store_dir)?;
    assert_eq!(model.metadata().base_commit, base_149);
    assert_eq!(model.metadata().base_version, "149.0.7250.0");
    assert_eq!(
        fs::read(store_dir.join("chrome/browser/ui/llmchat/keep.cc"))?,
        keep_patch
    );
    assert_ne!(
        fs::read(store_dir.join("chrome/browser/ui/llmchat/foo.cc"))?,
        model.patches()["chrome/browser/ui/llmchat/keep.cc"].contents
    );
    let human = extract::render_human(&report);
    assert!(human.contains("re-diffed 2 patches against base 149.0.7250.0 (1 content change"));
    assert!(human.contains("store base pin: 148.0.7204.1 → 149.0.7250.0"));
    assert!(human.contains("chore: repin to 149.0.7250.0"));
    Ok(())
}

fn write_base_checkout(repo: &FixtureRepo) -> Result<String> {
    repo.write_file(
        "chrome/VERSION",
        "MAJOR=148\nMINOR=0\nBUILD=7204\nPATCH=1\n",
    )?;
    repo.write_file("chrome/browser/ui/llmchat/panel.cc", "panel base\n")?;
    repo.write_file("chrome/browser/ui/llmchat/panel.h", "panel h base\n")?;
    repo.write_file(
        "chrome/browser/browseros/BUILD.gn",
        "browseros build base\n",
    )?;
    repo.commit("Chromium 148.0.7204.1")
}

fn write_repin_base(repo: &FixtureRepo, version: &str, upstream_contents: &str) -> Result<String> {
    let mut parts = version.split('.');
    repo.write_file(
        "chrome/VERSION",
        format!(
            "MAJOR={}\nMINOR={}\nBUILD={}\nPATCH={}\n",
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap()
        ),
    )?;
    repo.write_file("chrome/browser/ui/llmchat/foo.cc", upstream_contents)?;
    repo.write_file("chrome/browser/ui/llmchat/keep.cc", "keep base\n")?;
    repo.commit(&format!("Chromium {version}"))
}

fn seed_store(store: &FixtureRepo, base: &str) -> Result<PathBuf> {
    store.write_file(
        "chromium_patches/.store.yaml",
        format!("base_commit: {base}\nbase_version: \"148.0.7204.1\"\n"),
    )?;
    store.write_file(
        "chromium_patches/.features.yaml",
        r#"version: "1.0"
features:
  # comment that must survive feature appends
  llmchat:
    description: "feat: llmchat"
    files:
      - chrome/browser/ui/llmchat/panel.cc
      - chrome/browser/ui/llmchat/panel.h
  bootstrap:
    description: "chore: bootstrap"
    files:
      - chrome/browser/browseros/BUILD.gn
"#,
    )?;
    Ok(store.path().join("chromium_patches"))
}

fn rewrite_index_line(bytes: &mut Vec<u8>, replacement: &[u8]) -> Result<()> {
    let text = String::from_utf8(bytes.clone())?;
    let start = text.find("index ").context("index line")?;
    let end = text[start..].find('\n').context("index line end")? + start + 1;
    bytes.splice(start..end, replacement.iter().copied());
    Ok(())
}
