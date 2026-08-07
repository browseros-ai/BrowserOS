mod fixtures;

use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bpatch::cli::abort::{self, AbortReport};
use bpatch::cli::apply::{self as cli_apply, ApplyReport};
use bpatch::cli::continue_cmd::{self, ContinueOptions, ContinueReport};
use bpatch::engine::apply::ApplyOptions;
use bpatch::engine::conflict;
use bpatch::engine::progress;
use bpatch::engine::state::{self, StateContext};
use fixtures::FixtureRepo;
use serde_json::Value;

struct ConflictScenario {
    checkout: FixtureRepo,
    store: FixtureRepo,
    store_dir: PathBuf,
    new_base: String,
    store_rev: String,
}

#[test]
fn apply_base_bump_begins_conflict_session_without_worktree_writes() -> Result<()> {
    let scenario = conflict_scenario()?;
    let before = worktree_snapshot(scenario.checkout.path())?;

    let report = run_apply(&scenario);

    match &report {
        ApplyReport::Conflicts {
            base,
            merged,
            conflicts,
            worktree_touched,
            exit,
        } => {
            assert_eq!(base, "149.0.7250.0");
            assert_eq!(*merged, 1);
            assert!(!worktree_touched);
            assert_eq!(*exit, 2);
            assert_eq!(conflicts.len(), 1);
            assert_eq!(
                conflicts[0].file,
                PathBuf::from("chrome/app/chrome_main_delegate.cc")
            );
            assert_eq!(conflicts[0].feature, "bootstrap");
            assert_eq!(conflicts[0].kind, "content");
        }
        other => panic!("expected conflicts report, got {other:?}"),
    }

    let json: Value = serde_json::from_str(&cli_apply::render_json(&report)?)?;
    assert_eq!(json["result"], "conflicts");
    assert_eq!(json["base"], "149.0.7250.0");
    assert_eq!(json["merged"], 1);
    assert_eq!(json["worktree_touched"], false);
    assert_eq!(json["exit"], 2);
    assert_eq!(
        json["conflicts"][0]["file"],
        "chrome/app/chrome_main_delegate.cc"
    );
    assert_eq!(json["conflicts"][0]["feature"], "bootstrap");
    assert_eq!(json["conflicts"][0]["kind"], "content");

    assert!(conflict::session_path(scenario.checkout.path())?.exists());
    assert!(
        !conflict::load_session(scenario.checkout.path())?
            .expect("session")
            .materialized
    );
    assert_eq!(worktree_snapshot(scenario.checkout.path())?, before);
    Ok(())
}

#[test]
fn apply_refuses_to_overwrite_pending_conflict_session() -> Result<()> {
    let scenario = conflict_scenario()?;
    run_apply(&scenario);
    let session_path = conflict::session_path(scenario.checkout.path())?;
    let session_before = fs::read(&session_path)?;

    let report = run_apply(&scenario);

    match &report {
        ApplyReport::SessionPending {
            created_at,
            conflicts,
            exit,
        } => {
            assert!(*created_at > 0);
            assert_eq!(*conflicts, 1);
            assert_eq!(*exit, 2);
        }
        other => panic!("expected session-pending report, got {other:?}"),
    }
    let human = cli_apply::render_human(&report);
    assert!(human.contains("conflict session in progress"));
    assert!(human.contains("finish with `bpatch continue` or `bpatch abort`"));
    let json: Value = serde_json::from_str(&cli_apply::render_json(&report)?)?;
    assert_eq!(json["result"], "session-pending");
    assert_eq!(json["exit"], 2);
    assert_eq!(fs::read(&session_path)?, session_before);
    Ok(())
}

#[test]
fn final_continue_refuses_before_conflicts_are_materialized() -> Result<()> {
    let scenario = conflict_scenario()?;
    run_apply(&scenario);
    let session_path = conflict::session_path(scenario.checkout.path())?;
    let session_before = fs::read(&session_path)?;
    let worktree_before = worktree_snapshot(scenario.checkout.path())?;
    let store_before = scenario.store.status_porcelain()?;

    let report = continue_cmd::run(
        &ctx(&scenario),
        ContinueOptions { materialize: false },
        &mut progress::noop(),
    );

    match &report {
        ContinueReport::NotMaterialized { reason, exit } => {
            assert_eq!(*exit, 2);
            assert_eq!(
                reason,
                "conflicts were never materialized — run `bpatch continue --materialize`, resolve, then continue"
            );
        }
        other => panic!("expected not-materialized report, got {other:?}"),
    }
    assert_eq!(
        worktree_snapshot(scenario.checkout.path())?,
        worktree_before
    );
    assert_eq!(scenario.store.status_porcelain()?, store_before);
    assert_eq!(fs::read(&session_path)?, session_before);
    Ok(())
}

#[test]
fn base_bump_apply_refuses_uncommitted_drift_before_session_begin() -> Result<()> {
    let scenario = conflict_scenario()?;
    scenario
        .checkout
        .write_file("chrome/app/chrome_main_delegate.cc", "local edit\n")?;
    let before = worktree_snapshot(scenario.checkout.path())?;

    let report = run_apply(&scenario);

    match report {
        ApplyReport::Drift { files, exit } => {
            assert_eq!(exit, 3);
            assert_eq!(files.len(), 1);
            assert_eq!(
                files[0].path,
                PathBuf::from("chrome/app/chrome_main_delegate.cc")
            );
            assert_eq!(files[0].annotation, "modified, uncommitted");
        }
        other => panic!("expected drift report, got {other:?}"),
    }
    assert_eq!(worktree_snapshot(scenario.checkout.path())?, before);
    assert!(!conflict::session_path(scenario.checkout.path())?.exists());
    Ok(())
}

#[test]
fn abort_clears_session_without_touching_worktree_and_handles_stale_file() -> Result<()> {
    let scenario = conflict_scenario()?;
    run_apply(&scenario);
    let before = worktree_snapshot(scenario.checkout.path())?;

    let report = abort::run(&ctx(&scenario));

    assert_eq!(report, AbortReport::Aborted { exit: 0 });
    assert!(!conflict::session_path(scenario.checkout.path())?.exists());
    assert_eq!(worktree_snapshot(scenario.checkout.path())?, before);

    let stale_path = conflict::session_path(scenario.checkout.path())?;
    fs::create_dir_all(stale_path.parent().expect("session parent"))?;
    fs::write(&stale_path, br#"{"stale":true}"#)?;
    assert_eq!(
        abort::run(&ctx(&scenario)),
        AbortReport::Aborted { exit: 0 }
    );
    assert!(!stale_path.exists());
    Ok(())
}

#[test]
fn continue_materialize_writes_only_conflicted_files_with_markers() -> Result<()> {
    let scenario = conflict_scenario()?;
    run_apply(&scenario);

    let report = continue_cmd::run(
        &ctx(&scenario),
        ContinueOptions { materialize: true },
        &mut progress::noop(),
    );

    assert_eq!(
        report,
        ContinueReport::Materialized {
            files_written: 1,
            clean_files: 1,
            exit: 0,
        }
    );
    assert!(
        conflict::load_session(scenario.checkout.path())?
            .expect("session")
            .materialized
    );
    assert!(
        continue_cmd::render_human(&report)
            .contains("1 file written with conflict markers; 1 clean file staged for convergence")
    );
    assert_eq!(
        scenario.checkout.git().run_str(&["diff", "--name-only"])?,
        "chrome/app/chrome_main_delegate.cc"
    );
    let contents = scenario
        .checkout
        .read_file("chrome/app/chrome_main_delegate.cc")?;
    assert!(contents.contains("<<<<<<<"));
    assert!(contents.contains("======="));
    assert!(contents.contains(">>>>>>>"));
    assert_eq!(
        scenario
            .checkout
            .read_file("chrome/browser/ui/llmchat/clean.cc")?,
        "clean base\n"
    );
    Ok(())
}

#[test]
fn materialize_refuses_to_overwrite_dirty_conflict_file() -> Result<()> {
    let scenario = conflict_scenario()?;
    run_apply(&scenario);
    scenario
        .checkout
        .write_file("chrome/app/chrome_main_delegate.cc", "local edit\n")?;
    let before = worktree_snapshot(scenario.checkout.path())?;

    let report = continue_cmd::run(
        &ctx(&scenario),
        ContinueOptions { materialize: true },
        &mut progress::noop(),
    );

    match report {
        ContinueReport::Drift { files, exit } => {
            assert_eq!(exit, 3);
            assert_eq!(files.len(), 1);
            assert_eq!(
                files[0].path,
                PathBuf::from("chrome/app/chrome_main_delegate.cc")
            );
            assert_eq!(files[0].annotation, "modified, uncommitted");
        }
        other => panic!("expected drift report, got {other:?}"),
    }
    assert_eq!(worktree_snapshot(scenario.checkout.path())?, before);
    assert!(
        !conflict::load_session(scenario.checkout.path())?
            .expect("session")
            .materialized
    );
    Ok(())
}

#[test]
fn unresolved_markers_refuse_final_continue_with_file_list() -> Result<()> {
    let scenario = conflict_scenario()?;
    run_apply(&scenario);
    continue_cmd::run(
        &ctx(&scenario),
        ContinueOptions { materialize: true },
        &mut progress::noop(),
    );

    let report = continue_cmd::run(
        &ctx(&scenario),
        ContinueOptions { materialize: false },
        &mut progress::noop(),
    );

    match report {
        ContinueReport::Unresolved { files, exit } => {
            assert_eq!(exit, 2);
            assert_eq!(
                files,
                vec![PathBuf::from("chrome/app/chrome_main_delegate.cc")]
            );
        }
        other => panic!("expected unresolved report, got {other:?}"),
    }
    assert!(conflict::session_path(scenario.checkout.path())?.exists());
    Ok(())
}

#[test]
fn separator_line_alone_does_not_block_final_continue() -> Result<()> {
    let scenario = conflict_scenario()?;
    run_apply(&scenario);
    continue_cmd::run(
        &ctx(&scenario),
        ContinueOptions { materialize: true },
        &mut progress::noop(),
    );
    scenario.checkout.write_file(
        "chrome/app/chrome_main_delegate.cc",
        "resolved heading\n=======\nresolved bootstrap\n",
    )?;

    let report = continue_cmd::run(
        &ctx(&scenario),
        ContinueOptions { materialize: false },
        &mut progress::noop(),
    );

    assert!(matches!(report, ContinueReport::Completed { .. }));
    assert!(!conflict::session_path(scenario.checkout.path())?.exists());
    Ok(())
}

#[test]
fn resolved_continue_converges_authors_new_base_trailers_and_clears_session() -> Result<()> {
    let scenario = conflict_scenario()?;
    run_apply(&scenario);
    continue_cmd::run(
        &ctx(&scenario),
        ContinueOptions { materialize: true },
        &mut progress::noop(),
    );
    scenario
        .checkout
        .write_file("chrome/app/chrome_main_delegate.cc", "resolved bootstrap\n")?;

    let report = continue_cmd::run(
        &ctx(&scenario),
        ContinueOptions { materialize: false },
        &mut progress::noop(),
    );

    match &report {
        ContinueReport::Completed {
            base,
            store_rev,
            commits_authored,
            exit,
        } => {
            assert_eq!(base, "149.0.7250.0");
            assert_eq!(
                store_rev,
                &scenario
                    .store
                    .git()
                    .run_str(&["rev-parse", "--short", &scenario.store_rev])?
            );
            assert_eq!(*commits_authored, 2);
            assert_eq!(*exit, 0);
        }
        other => panic!("expected completed report, got {other:?}"),
    }
    let human = continue_cmd::render_human(&report);
    assert!(human.contains("✓ converged on base 149.0.7250.0 · 2 feature commits authored"));
    assert!(human.contains("Bpatch-Store-Rev:"));
    assert!(human.contains("Bpatch-Base: 149.0.7250.0"));

    assert!(!conflict::session_path(scenario.checkout.path())?.exists());
    assert_eq!(
        scenario
            .checkout
            .read_file("chrome/app/chrome_main_delegate.cc")?,
        "resolved bootstrap\n"
    );
    assert_eq!(
        scenario
            .checkout
            .read_file("chrome/browser/ui/llmchat/clean.cc")?,
        "clean feature\n"
    );

    let git = scenario.checkout.git_adapter();
    assert_eq!(git.commit_subject("HEAD^")?, "feat: bootstrap");
    assert_eq!(git.commit_subject("HEAD")?, "feat: llmchat");
    assert_apply_base(&git, "HEAD^", &scenario.store_rev, &scenario.new_base)?;
    assert_apply_base(&git, "HEAD", &scenario.store_rev, &scenario.new_base)?;

    let second = continue_cmd::run(
        &ctx(&scenario),
        ContinueOptions { materialize: false },
        &mut progress::noop(),
    );
    match second {
        ContinueReport::NoSession { reason, exit } => {
            assert_eq!(reason, "no conflict session");
            assert_eq!(exit, 1);
        }
        other => panic!("expected no-session report, got {other:?}"),
    }
    Ok(())
}

fn run_apply(scenario: &ConflictScenario) -> ApplyReport {
    cli_apply::run(
        &ctx(scenario),
        ApplyOptions { pull: false },
        &mut progress::noop(),
    )
}

fn ctx(scenario: &ConflictScenario) -> StateContext {
    StateContext::new(scenario.checkout.path(), &scenario.store_dir)
}

fn conflict_scenario() -> Result<ConflictScenario> {
    let checkout = FixtureRepo::new()?;
    let old_base = write_old_base(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_store(&store, &old_base)?;

    checkout.write_file("chrome/app/chrome_main_delegate.cc", "feature bootstrap\n")?;
    checkout.write_file("chrome/browser/ui/llmchat/clean.cc", "clean feature\n")?;
    checkout.git().run(&["add", "-A"])?;
    let store_rev = commit_store_from_index(
        &store,
        &checkout,
        &old_base,
        &[
            "chrome/app/chrome_main_delegate.cc",
            "chrome/browser/ui/llmchat/clean.cc",
        ],
        "store old-base target",
    )?;

    checkout.git().run(&["reset", "--hard", &old_base])?;
    checkout.write_file(
        "chrome/VERSION",
        "MAJOR=149\nMINOR=0\nBUILD=7250\nPATCH=0\n",
    )?;
    checkout.write_file("chrome/app/chrome_main_delegate.cc", "upstream bootstrap\n")?;
    let new_base = checkout.commit("Chromium 149.0.7250.0")?;

    Ok(ConflictScenario {
        checkout,
        store,
        store_dir,
        new_base,
        store_rev,
    })
}

fn write_old_base(repo: &FixtureRepo) -> Result<String> {
    repo.write_file(
        "chrome/VERSION",
        "MAJOR=148\nMINOR=0\nBUILD=7204\nPATCH=1\n",
    )?;
    repo.write_file("chrome/app/chrome_main_delegate.cc", "base bootstrap\n")?;
    repo.write_file("chrome/browser/ui/llmchat/clean.cc", "clean base\n")?;
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
  bootstrap:
    description: "feat: bootstrap"
    files:
      - chrome/app/
  llmchat:
    description: "feat: llmchat"
    files:
      - chrome/browser/ui/llmchat/
"#,
    )?;
    store.commit("seed store")?;
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

fn assert_apply_base(
    git: &bpatch::git::GitAdapter,
    rev: &str,
    store_rev: &str,
    base: &str,
) -> Result<()> {
    let trailers =
        state::parse_apply_trailers(&git.commit_trailers(rev)?)?.expect("apply trailers");
    assert_eq!(trailers.store_rev, store_rev);
    assert_eq!(trailers.base, base);
    Ok(())
}

fn worktree_snapshot(root: &Path) -> Result<u64> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for path in files {
        path.hash(&mut hasher);
        fs::read(root.join(&path))
            .with_context(|| format!("reading {}", root.join(&path).display()))?
            .hash(&mut hasher);
    }
    Ok(hasher.finish())
}

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading dir {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == ".git") {
            continue;
        }
        if path.is_dir() {
            collect_files(root, &path, files)?;
        } else if path.is_file() {
            files.push(path.strip_prefix(root)?.to_owned());
        }
    }
    Ok(())
}
