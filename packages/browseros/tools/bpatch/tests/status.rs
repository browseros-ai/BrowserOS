mod fixtures;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use bpatch::cli::{diff, status};
use bpatch::engine::conflict::{self, ConflictFile, ConflictSession};
use bpatch::engine::state::{
    self, StateContext, TRAILER_ANNOTATED, TRAILER_BASE, TRAILER_STORE_REV, TRAILER_TREE,
};
use fixtures::FixtureRepo;
use serde_json::Value;

struct AppliedScenario {
    checkout: FixtureRepo,
    store: FixtureRepo,
    store_dir: PathBuf,
    base: String,
    store_rev: String,
    applied_tree: String,
    apply_commit: String,
}

#[test]
fn fresh_and_apply_trailer_state_resolve() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_base_checkout(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_store(&store, &base)?;
    store.commit("seed store")?;

    let fresh = state::resolve(&StateContext::new(checkout.path(), &store_dir))?;
    assert!(fresh.applied.is_none());
    assert_eq!(fresh.base.display, "148.0.7204.1");
    assert!(fresh.drift.is_clean());

    let scenario = applied_scenario(true)?;
    let ctx = StateContext::new(scenario.checkout.path(), &scenario.store_dir);
    let resolved = state::resolve(&ctx)?;
    let applied = resolved.applied.expect("applied state");
    assert_eq!(applied.store_rev, scenario.store_rev);
    assert_eq!(applied.tree, scenario.applied_tree);
    assert_eq!(applied.feature_commit_count, 1);

    let report = status::run(&ctx)?;
    let human = status::render_human(&report);
    assert!(human.contains("base     148.0.7204.1"));
    assert!(human.contains("applied  store @"));
    assert!(human.contains("  ·  1 feature commit  ·  last: feat: llmchat"));
    assert!(human.contains("1 feature commit"));
    assert!(human.contains("tree     clean — no drift"));

    let json = serde_json::to_value(&report)?;
    assert_eq!(json["result"], "clean");
    assert_eq!(json["base"], "148.0.7204.1");
    assert_eq!(json["store_rev"], scenario.store_rev);
    assert_eq!(json["feature_commits"], 1);
    assert_eq!(json["drift"], Value::Array(Vec::new()));
    Ok(())
}

#[test]
fn missing_tree_trailer_recovers_by_recomputing_current_store() -> Result<()> {
    let scenario = applied_scenario(false)?;
    let resolved = state::resolve(&StateContext::new(
        scenario.checkout.path(),
        &scenario.store_dir,
    ))?;
    let applied = resolved.applied.expect("applied state");
    assert_eq!(applied.store_rev, scenario.store_rev);
    assert_eq!(applied.tree, scenario.applied_tree);
    Ok(())
}

#[test]
fn drift_reports_clean_committed_and_uncommitted_paths() -> Result<()> {
    let scenario = applied_scenario(true)?;
    let ctx = StateContext::new(scenario.checkout.path(), &scenario.store_dir);
    assert!(status::run(&ctx)?.drift.is_empty());

    scenario
        .checkout
        .write_file("chrome/browser/ui/llmchat/panel.cc", "manual commit\n")?;
    scenario.checkout.commit("manual edit")?;
    scenario
        .checkout
        .write_file("chrome/BUILD.gn", "uncommitted build edit\n")?;
    scenario
        .checkout
        .plant_untracked("out/Default_arm64/local.marker", "ignored\n")?;

    let report = status::run(&ctx)?;
    assert_eq!(report.result, status::StatusResult::Drift);
    assert_eq!(report.drift.len(), 2);
    let drift = report
        .drift
        .iter()
        .map(|file| {
            (
                file.path.to_string_lossy().into_owned(),
                file.annotation.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        drift.get("chrome/browser/ui/llmchat/panel.cc"),
        Some(&"modified since feat: llmchat".to_string())
    );
    assert_eq!(
        drift.get("chrome/BUILD.gn"),
        Some(&"modified, uncommitted".to_string())
    );

    let human = status::render_human(&report);
    assert!(human.contains("chrome/browser/ui/llmchat/panel.cc"));
    assert!(human.contains("(modified since feat: llmchat)"));
    assert!(human.contains("chrome/BUILD.gn"));
    assert!(human.contains("(modified, uncommitted)"));
    Ok(())
}

#[test]
fn annotate_commit_is_clean_and_anchors_later_committed_drift() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_base_checkout(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_store(&store, &base)?;
    store.commit("seed store")?;

    checkout.write_file("chrome/browser/ui/llmchat/panel.cc", "annotated\n")?;
    checkout.commit_with_trailers(
        "feat: llmchat from bos_build",
        &[(TRAILER_BASE, base.as_str()), (TRAILER_ANNOTATED, "true")],
    )?;

    let ctx = StateContext::new(checkout.path(), &store_dir);
    let annotated = status::run(&ctx)?;
    assert!(annotated.applied_store_rev.is_none());
    assert!(annotated.drift.is_empty());

    checkout.write_file("chrome/browser/ui/llmchat/panel.cc", "manual commit\n")?;
    checkout.commit("manual edit")?;

    let drifted = status::run(&ctx)?;
    assert_eq!(drifted.drift.len(), 1);
    assert_eq!(
        drifted.drift[0].annotation,
        "modified since feat: llmchat from bos_build"
    );
    Ok(())
}

#[test]
fn annotate_after_apply_replaces_only_the_drift_anchor() -> Result<()> {
    let scenario = applied_scenario(true)?;
    scenario.checkout.write_file(
        "chrome/browser/ui/llmchat/panel.cc",
        "annotated after apply\n",
    )?;
    scenario.checkout.commit_with_trailers(
        "feat: llmchat from newer bos_build",
        &[
            (TRAILER_BASE, scenario.base.as_str()),
            (TRAILER_ANNOTATED, "true"),
        ],
    )?;

    let resolved = state::resolve(&StateContext::new(
        scenario.checkout.path(),
        &scenario.store_dir,
    ))?;
    let applied = resolved.applied.expect("older applied state");
    assert_eq!(applied.store_rev, scenario.store_rev);
    assert_eq!(applied.commit, scenario.apply_commit);
    assert!(resolved.drift.is_clean());
    Ok(())
}

#[test]
fn missing_tree_recompute_ignores_store_false_patches() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    checkout.write_file(
        "chrome/VERSION",
        "MAJOR=148\nMINOR=0\nBUILD=7204\nPATCH=1\n",
    )?;
    checkout.write_file("chrome/browser/ui/llmchat/panel.cc", "base\n")?;
    checkout.write_file(
        "chrome/browser/browseros/server/resources/bundle.js",
        "base resource\n",
    )?;
    let base = checkout.commit("Chromium 148.0.7204.1")?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_store(&store, &base)?;
    store.write_file(
        "chromium_patches/.features.yaml",
        r#"version: "1.0"
features:
  llmchat:
    description: "feat: llmchat"
    files:
      - chrome/browser/ui/llmchat/
  build-resources:
    description: "resource: bos_build outputs"
    store: false
    files:
      - chrome/browser/browseros/server/resources/
"#,
    )?;

    checkout.write_file("chrome/browser/ui/llmchat/panel.cc", "applied\n")?;
    checkout.write_file(
        "chrome/browser/browseros/server/resources/bundle.js",
        "store-only resource\n",
    )?;
    checkout.git().run(&["add", "-A"])?;
    let store_rev = commit_store_from_index(
        &store,
        &checkout,
        &base,
        &[
            "chrome/browser/ui/llmchat/panel.cc",
            "chrome/browser/browseros/server/resources/bundle.js",
        ],
        "store with ignored resource patch",
    )?;
    checkout.git().run(&[
        "checkout",
        &base,
        "--",
        "chrome/browser/browseros/server/resources/bundle.js",
    ])?;
    checkout.git().run(&["add", "-A"])?;
    let expected_tree = checkout.git().run_str(&["write-tree"])?;
    checkout.commit_with_trailers(
        "feat: llmchat",
        &[
            (TRAILER_STORE_REV, store_rev.as_str()),
            (TRAILER_BASE, base.as_str()),
        ],
    )?;

    let resolved = state::resolve(&StateContext::new(checkout.path(), &store_dir))?;
    assert_eq!(resolved.applied.expect("applied state").tree, expected_tree);
    assert!(resolved.drift.is_clean());
    Ok(())
}

#[test]
fn diff_groups_by_feature_and_reports_rebuild_scope() -> Result<()> {
    let no_build = applied_scenario(true)?;
    no_build
        .checkout
        .write_file("chrome/browser/ui/llmchat/panel.cc", "store current\n")?;
    no_build
        .checkout
        .write_file("chrome/browser/ui/llmchat/resize_util.cc", "resize\n")?;
    no_build.checkout.git().run(&["add", "-A"])?;
    let no_build_store_rev = commit_store_from_index(
        &no_build.store,
        &no_build.checkout,
        &no_build.base,
        &[
            "chrome/browser/ui/llmchat/panel.cc",
            "chrome/browser/ui/llmchat/resize_util.cc",
        ],
        "store current",
    )?;
    no_build
        .checkout
        .git()
        .run(&["reset", "--hard", &no_build.apply_commit])?;

    let no_build_report = diff::run(&StateContext::new(
        no_build.checkout.path(),
        &no_build.store_dir,
    ))?;
    assert_eq!(no_build_report.files_changed, 2);
    assert_eq!(no_build_report.features_changed, 1);
    assert_eq!(no_build_report.groups[0].feature, "llmchat");
    assert!(!no_build_report.rebuild_scope.touches_build_files);
    let no_build_human = diff::render_human(&no_build_report);
    assert!(no_build_human.contains("apply would touch 2 files · 1 feature:"));
    assert!(
        no_build_human
            .contains("no BUILD.gn / *.gni / include-fanout files touched → small incremental")
    );
    let json = serde_json::to_value(&no_build_report)?;
    assert_eq!(json["result"], "changes");
    assert_eq!(json["store_rev"], no_build_store_rev);
    assert_eq!(json["files_changed"], 2);

    let with_build = applied_scenario(true)?;
    with_build
        .checkout
        .write_file("chrome/BUILD.gn", "build change\n")?;
    with_build
        .checkout
        .write_file("chrome/browser/ui/llmchat/features.gni", "gni change\n")?;
    with_build.checkout.git().run(&["add", "-A"])?;
    commit_store_from_index(
        &with_build.store,
        &with_build.checkout,
        &with_build.base,
        &["chrome/BUILD.gn", "chrome/browser/ui/llmchat/features.gni"],
        "store build current",
    )?;
    with_build
        .checkout
        .git()
        .run(&["reset", "--hard", &with_build.apply_commit])?;

    let with_build_report = diff::run(&StateContext::new(
        with_build.checkout.path(),
        &with_build.store_dir,
    ))?;
    assert_eq!(with_build_report.files_changed, 2);
    assert_eq!(with_build_report.groups[0].feature, "bootstrap");
    assert!(with_build_report.rebuild_scope.touches_build_files);
    assert_eq!(with_build_report.rebuild_scope.build_files_changed, 2);
    assert!(
        diff::render_human(&with_build_report)
            .contains("touches 2 BUILD.gn / *.gni files → large rebuild likely")
    );
    Ok(())
}

#[test]
fn diff_after_annotate_reports_only_head_to_apply_target() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_base_checkout(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_store(&store, &base)?;

    checkout.write_file("chrome/browser/ui/llmchat/panel.cc", "annotated panel\n")?;
    checkout.write_file("chrome/browser/ui/llmchat/features.gni", "annotated gni\n")?;
    checkout.git().run(&["add", "-A"])?;
    commit_store_from_index(
        &store,
        &checkout,
        &base,
        &[
            "chrome/browser/ui/llmchat/panel.cc",
            "chrome/browser/ui/llmchat/features.gni",
        ],
        "store annotated state",
    )?;
    let annotate_commit = checkout.commit_with_trailers(
        "feat: llmchat from bos_build",
        &[(TRAILER_BASE, base.as_str()), (TRAILER_ANNOTATED, "true")],
    )?;

    checkout.write_file("chrome/browser/ui/llmchat/panel.cc", "store current\n")?;
    checkout.git().run(&["add", "-A"])?;
    commit_store_from_index(
        &store,
        &checkout,
        &base,
        &["chrome/browser/ui/llmchat/panel.cc"],
        "store current",
    )?;
    checkout.git().run(&["reset", "--hard", &annotate_commit])?;

    let report = diff::run(&StateContext::new(checkout.path(), &store_dir))?;
    assert_eq!(report.files_changed, 1);
    assert_eq!(report.groups.len(), 1);
    assert_eq!(
        report.groups[0].files[0].path,
        PathBuf::from("chrome/browser/ui/llmchat/panel.cc")
    );
    Ok(())
}

#[test]
fn amending_latest_feature_commit_to_drop_trailers_does_not_wedge_state() -> Result<()> {
    let scenario = applied_scenario(true)?;
    scenario
        .checkout
        .write_file("chrome/browser/ui/llmchat/panel.cc", "second apply\n")?;
    scenario.checkout.git().run(&["add", "-A"])?;
    let second_tree = scenario.checkout.git().run_str(&["write-tree"])?;
    let second_store_rev = commit_store_from_index(
        &scenario.store,
        &scenario.checkout,
        &scenario.base,
        &["chrome/browser/ui/llmchat/panel.cc"],
        "store second",
    )?;
    scenario.checkout.commit_with_trailers(
        "feat: llmchat #2",
        &[
            (TRAILER_STORE_REV, second_store_rev.as_str()),
            (TRAILER_BASE, scenario.base.as_str()),
            (TRAILER_TREE, second_tree.as_str()),
        ],
    )?;

    scenario
        .checkout
        .git()
        .run(&["commit", "--amend", "-m", "feat: llmchat rewritten"])?;

    let resolved = state::resolve(&StateContext::new(
        scenario.checkout.path(),
        &scenario.store_dir,
    ))?;
    let applied = resolved.applied.expect("previous trailer commit");
    assert_eq!(applied.store_rev, scenario.store_rev);
    assert_eq!(applied.tree, scenario.applied_tree);
    Ok(())
}

#[test]
fn status_reports_in_progress_conflict_session() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_base_checkout(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_store(&store, &base)?;
    let store_rev = store.commit("seed store")?;
    let path = conflict::session_path(checkout.path())?;
    std::fs::create_dir_all(path.parent().expect("session parent"))?;
    std::fs::write(
        &path,
        serde_json::to_vec(&ConflictSession {
            new_base: base.clone(),
            new_base_display: "148.0.7204.1".to_string(),
            pin_base: base.clone(),
            store_rev,
            merged_tree: checkout.git_adapter().tree_id("HEAD")?,
            target_tree: checkout.git_adapter().tree_id("HEAD")?,
            conflicts: vec![ConflictFile {
                file: PathBuf::from("chrome/app/chrome_main_delegate.cc"),
                feature: "bootstrap".to_string(),
                kind: "content".to_string(),
            }],
            parent_head: base,
            created_at: 123,
            materialized: false,
        })?,
    )?;

    let report = status::run(&StateContext::new(checkout.path(), &store_dir))?;

    let session = report.conflict_session.as_ref().expect("conflict session");
    assert_eq!(session.created_at, 123);
    assert_eq!(session.base, "148.0.7204.1");
    assert_eq!(session.conflicts, 1);
    let human = status::render_human(&report);
    assert!(human.contains(
        "session  conflict session in progress (1 conflict) — bpatch continue / bpatch abort"
    ));
    let json = serde_json::to_value(&report)?;
    assert_eq!(json["conflict_session"]["conflicts"], 1);
    assert_eq!(json["conflict_session"]["created_at"], 123);
    Ok(())
}

fn applied_scenario(include_tree: bool) -> Result<AppliedScenario> {
    let checkout = FixtureRepo::new()?;
    let base = write_base_checkout(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_store(&store, &base)?;

    checkout.write_file("chrome/browser/ui/llmchat/panel.cc", "applied\n")?;
    checkout.git().run(&["add", "-A"])?;
    let applied_tree = checkout.git().run_str(&["write-tree"])?;
    let store_rev = commit_store_from_index(
        &store,
        &checkout,
        &base,
        &["chrome/browser/ui/llmchat/panel.cc"],
        "store applied",
    )?;

    let mut trailers = vec![
        (TRAILER_STORE_REV, store_rev.as_str()),
        (TRAILER_BASE, base.as_str()),
    ];
    if include_tree {
        trailers.push((TRAILER_TREE, applied_tree.as_str()));
    }
    let apply_commit = checkout.commit_with_trailers("feat: llmchat", &trailers)?;

    Ok(AppliedScenario {
        checkout,
        store,
        store_dir,
        base,
        store_rev,
        applied_tree,
        apply_commit,
    })
}

fn write_base_checkout(repo: &FixtureRepo) -> Result<String> {
    repo.write_file(
        "chrome/VERSION",
        "MAJOR=148\nMINOR=0\nBUILD=7204\nPATCH=1\n",
    )?;
    repo.write_file("chrome/browser/ui/llmchat/panel.cc", "base\n")?;
    repo.write_file("chrome/browser/ui/llmchat/features.gni", "base gni\n")?;
    repo.write_file("chrome/BUILD.gn", "base build\n")?;
    repo.commit("Chromium 148.0.7204.1")
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
  llmchat:
    description: "feat: llmchat"
    files:
      - chrome/browser/ui/llmchat/
  bootstrap:
    description: "chore: bootstrap"
    files:
      - chrome/BUILD.gn
"#,
    )?;
    Ok(store.path().join("chromium_patches"))
}

fn commit_store_from_index(
    store: &FixtureRepo,
    checkout: &FixtureRepo,
    base: &str,
    paths: &[&str],
    message: &str,
) -> Result<String> {
    for path in paths {
        let diff = checkout
            .git()
            .run(&["diff", "--binary", "--cached", base, "--", path])?;
        store.write_file(Path::new("chromium_patches").join(path), diff)?;
    }
    store.commit(message)
}
