mod fixtures;

use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use bpatch::engine::lock::CheckoutLock;
use bpatch::engine::state::{TRAILER_ANNOTATED, TRAILER_BASE, TRAILER_STORE_REV, TRAILER_TREE};
use bpatch::process::Git;
use bpatch::store::Store;
use fixtures::FixtureRepo;
use serde_json::Value;

struct CmdOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

struct AppliedScenario {
    checkout: FixtureRepo,
    store: FixtureRepo,
    store_dir: PathBuf,
    base: String,
    rev1_commit: String,
}

#[test]
fn top_level_help_teaches_chromium_workflow() -> Result<()> {
    let output = Command::new(env!("CARGO_BIN_EXE_bpatch"))
        .arg("--help")
        .output()
        .context("running bpatch --help")?;
    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;

    assert!(output.status.success(), "{stderr}");
    assert!(stderr.is_empty());
    assert!(!stdout.contains('\u{1b}'));
    assert!(stdout.contains("GETTING STARTED:"));
    assert!(stdout.contains("bpatch init /abs/path/to/chromium_patches"));
    assert!(stdout.contains("Or run bpatch init from inside chromium_patches."));
    assert!(stdout.contains("Run bpatch from inside a Chromium checkout."));
    assert!(stdout.contains("GLOBAL FLAGS:"));
    assert!(
        stdout.contains("--store <STORE>  Overrides the config file for store-reading commands.")
    );
    assert!(stdout.contains("--json           Emits a single JSON object"));
    assert!(stdout.contains("EXAMPLES:"));
    assert!(stdout.contains("bpatch status"));
    assert!(stdout.contains("bpatch diff"));
    assert!(stdout.contains("bpatch apply"));
    assert!(stdout.contains("bpatch annotate"));
    assert!(stdout.contains("bpatch extract <rev1>..<rev2> --feature <name>"));
    assert!(stdout.contains("bpatch apply -> bpatch continue --materialize"));
    assert!(stdout.contains("EXIT CODES:"));
    assert!(stdout.contains("0  Initialized, converged, applied"));
    assert!(stdout.contains("2  Conflicts are pending"));
    assert!(stdout.contains("3  Drift/refusal or extract needs a feature decision."));
    assert!(stdout.contains("1  CLI, git, lock, config, or unexpected error."));

    let short = Command::new(env!("CARGO_BIN_EXE_bpatch"))
        .arg("-h")
        .output()
        .context("running bpatch -h")?;
    let short_stdout = String::from_utf8(short.stdout)?;
    let short_stderr = String::from_utf8(short.stderr)?;

    assert!(short.status.success(), "{short_stderr}");
    assert!(short_stderr.is_empty());
    assert!(!short_stdout.contains("GETTING STARTED:"));
    assert!(!short_stdout.contains("EXIT CODES:"));
    Ok(())
}

#[test]
fn sim1_extract_hack_commit_renders_routing_net_fold_and_next_step() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_extract_base(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_extract_store(&store, &base)?;
    store.commit("seed store")?;

    checkout.write_file("chrome/browser/ui/llmchat/tmp_probe.cc", "tmp\n")?;
    checkout.commit("probe")?;
    checkout.write_file("chrome/browser/ui/llmchat/panel.cc", "panel changed\n")?;
    checkout.write_file("chrome/browser/ui/llmchat/panel.h", "panel h changed\n")?;
    checkout.write_file("chrome/browser/ui/llmchat/resize_util.cc", "resize\n")?;
    checkout.remove_file("chrome/browser/ui/llmchat/tmp_probe.cc")?;
    let rev = checkout.commit("fix llm chat panel resize jitter")?;

    let out = run_bpatch(
        checkout.path(),
        Some(&store_dir),
        vec!["extract".into(), rev, "--accept-suggestions".into()],
    )?;

    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stderr.is_empty());
    assert!(
        out.stdout
            .contains("extract: 3 files changed vs base 148.0.7204.1")
    );
    assert!(out.stdout.contains("M  chrome/browser/ui/llmchat/panel.cc"));
    assert!(out.stdout.contains("→ feature: llmchat (matched)"));
    assert!(
        out.stdout
            .contains("A  chrome/browser/ui/llmchat/resize_util.cc")
    );
    assert!(out.stdout.contains("→ feature: llmchat (nearest path)"));
    assert!(
        out.stdout
            .contains("net-fold: chrome/browser/ui/llmchat/tmp_probe.cc")
    );
    assert!(out.stdout.contains("→ no patch"));
    assert!(
        out.stdout
            .contains("store: chromium_patches 3 patches updated, .features.yaml unchanged")
    );
    assert!(
        out.stdout
            .contains("next: bpatch extract --commit to commit the store repo")
    );
    Ok(())
}

#[test]
fn sim2_daily_loop_status_diff_apply_converged_and_feature_list() -> Result<()> {
    let scenario = applied_rev1_scenario()?;
    write_checkout_rev2(&scenario.checkout, false)?;
    commit_store_from_index(
        &scenario.store,
        &scenario.checkout,
        &scenario.base,
        &[
            "chrome/browser/ui/llmchat/panel.cc",
            "chrome/browser/ui/llmchat/panel.h",
            "chrome/browser/ui/llmchat/resize_util.cc",
        ],
        "store rev2",
    )?;
    scenario
        .checkout
        .git()
        .run(&["reset", "--hard", &scenario.rev1_commit])?;

    let status = run_bpatch(
        scenario.checkout.path(),
        Some(&scenario.store_dir),
        strs(&["status"]),
    )?;
    assert_eq!(status.code, 0, "{}", status.stderr);
    assert!(status.stdout.contains("base     148.0.7204.1"));
    assert!(status.stdout.contains("store    "));
    assert!(status.stdout.contains("applied  store @"));
    assert!(
        status
            .stdout
            .contains("·  1 feature commit  ·  last: feat: llmchat")
    );
    assert!(status.stdout.contains("tree     clean — no drift"));

    let diff = run_bpatch(
        scenario.checkout.path(),
        Some(&scenario.store_dir),
        strs(&["diff"]),
    )?;
    assert_eq!(diff.code, 0, "{}", diff.stderr);
    assert!(
        diff.stdout
            .contains("apply would touch 3 files · 1 feature:")
    );
    assert!(diff.stdout.contains("llmchat"));
    assert!(
        diff.stdout
            .contains("M    chrome/browser/ui/llmchat/panel.cc")
    );
    assert!(
        diff.stdout
            .contains("A    chrome/browser/ui/llmchat/resize_util.cc")
    );
    assert!(
        diff.stdout
            .contains("no BUILD.gn / *.gni / include-fanout files touched → small incremental")
    );

    let apply = run_bpatch(
        scenario.checkout.path(),
        Some(&scenario.store_dir),
        strs(&["apply"]),
    )?;
    assert_eq!(apply.code, 0, "{}", apply.stderr);
    assert!(apply.stdout.contains("apply: store "));
    assert!(apply.stdout.contains("✓ 3 files written"));
    assert!(apply.stdout.contains("\"feat: llmchat #2\""));
    assert!(
        apply
            .stdout
            .contains("converged. → incremental build will recompile")
    );

    let converged = run_bpatch(
        scenario.checkout.path(),
        Some(&scenario.store_dir),
        strs(&["apply"]),
    )?;
    assert_eq!(converged.code, 0, "{}", converged.stderr);
    assert!(converged.stdout.contains("already converged at store"));
    assert!(converged.stdout.contains("— nothing to do."));

    let features = run_bpatch(
        scenario.checkout.path(),
        Some(&scenario.store_dir),
        strs(&["feature", "list"]),
    )?;
    assert_eq!(features.code, 0, "{}", features.stderr);
    assert!(features.stdout.contains("feature"));
    assert!(features.stdout.contains("llmchat"));
    assert!(features.stdout.contains("feat: llmchat"));
    assert!(features.stdout.lines().any(|line| {
        line.contains("llmchat") && line.split_whitespace().collect::<Vec<_>>()[1..3] == ["3", "2"]
    }));
    Ok(())
}

#[test]
fn sim3_cron_json_pull_converged_and_lock_error_are_machine_only() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_apply_base(&checkout)?;
    let remote_store = FixtureRepo::new()?;
    let remote_store_dir = seed_apply_store(&remote_store, &base)?;

    write_checkout_rev1(&checkout)?;
    checkout.git().run(&["add", "-A"])?;
    let rev1_tree = checkout.git().run_str(&["write-tree"])?;
    let rev1_store = commit_store_from_index(
        &remote_store,
        &checkout,
        &base,
        &[
            "chrome/browser/ui/llmchat/panel.cc",
            "chrome/browser/ui/llmchat/panel.h",
        ],
        "store rev1",
    )?;
    let rev1_commit = checkout.commit_with_trailers(
        "feat: llmchat",
        &[
            (TRAILER_STORE_REV, rev1_store.as_str()),
            (TRAILER_BASE, base.as_str()),
            (TRAILER_TREE, rev1_tree.as_str()),
        ],
    )?;

    let local_root = tempfile::tempdir()?;
    Git::new(local_root.path()).run(&[
        "clone",
        remote_store.path().to_str().expect("utf-8 path"),
        "store",
    ])?;
    let local_store_dir = local_root.path().join("store/chromium_patches");

    write_checkout_rev2(&checkout, false)?;
    commit_store_from_index(
        &remote_store,
        &checkout,
        &base,
        &[
            "chrome/browser/ui/llmchat/panel.cc",
            "chrome/browser/ui/llmchat/panel.h",
            "chrome/browser/ui/llmchat/resize_util.cc",
        ],
        "store rev2",
    )?;
    checkout.git().run(&["reset", "--hard", &rev1_commit])?;

    let applied = run_bpatch(
        checkout.path(),
        Some(&local_store_dir),
        strs(&["apply", "--pull", "--json"]),
    )?;
    assert_eq!(applied.code, 0, "{}", applied.stderr);
    assert!(applied.stderr.is_empty());
    assert!(!applied.stdout.contains('\r'));
    let json = parse_json(&applied.stdout)?;
    assert_eq!(json["result"], "applied");
    assert_eq!(json["base"], "148.0.7204.1");
    assert_eq!(json["files_changed"], 3);
    assert_eq!(json["commits"][0]["feature"], "llmchat");
    assert_eq!(json["commits"][0]["seq"], 2);
    assert_eq!(json["exit"], 0);
    assert_eq!(
        Git::new(remote_store_dir).run_str(&["rev-parse", "--short", "HEAD"])?,
        json["store_rev"].as_str().expect("store rev")
    );

    let night2 = run_bpatch(
        checkout.path(),
        Some(&local_store_dir),
        strs(&["apply", "--pull", "--json"]),
    )?;
    assert_eq!(night2.code, 0, "{}", night2.stderr);
    let json = parse_json(&night2.stdout)?;
    assert_eq!(json["result"], "converged");
    assert_eq!(json["files_changed"], 0);
    assert_eq!(json["exit"], 0);

    let _lock = CheckoutLock::acquire(checkout.path())?;
    let locked = run_bpatch(
        checkout.path(),
        Some(&local_store_dir),
        strs(&["apply", "--json"]),
    )?;
    assert_eq!(locked.code, 1);
    let json = parse_json(&locked.stdout)?;
    assert_eq!(json["result"], "error");
    assert!(
        json["reason"]
            .as_str()
            .unwrap()
            .contains("lock held by pid")
    );
    assert_eq!(json["exit"], 1);
    Ok(())
}

#[test]
fn sim4_drift_refusal_prints_annotations_and_remedies() -> Result<()> {
    let scenario = applied_rev1_scenario()?;
    write_checkout_rev2(&scenario.checkout, false)?;
    commit_store_from_index(
        &scenario.store,
        &scenario.checkout,
        &scenario.base,
        &[
            "chrome/browser/ui/llmchat/panel.cc",
            "chrome/browser/ui/llmchat/panel.h",
            "chrome/browser/ui/llmchat/resize_util.cc",
        ],
        "store rev2",
    )?;
    scenario
        .checkout
        .git()
        .run(&["reset", "--hard", &scenario.rev1_commit])?;
    scenario
        .checkout
        .write_file("chrome/browser/ui/llmchat/panel.cc", "manual committed\n")?;
    scenario.checkout.commit("manual edit")?;
    scenario
        .checkout
        .write_file("chrome/BUILD.gn", "manual build drift\n")?;

    let out = run_bpatch(
        scenario.checkout.path(),
        Some(&scenario.store_dir),
        strs(&["apply"]),
    )?;

    assert_eq!(out.code, 3);
    assert!(
        out.stdout
            .contains("drift: working tree differs from its bpatch baseline in 2 files:")
    );
    assert!(out.stdout.contains("chrome/browser/ui/llmchat/panel.cc"));
    assert!(out.stdout.contains("(modified since feat: llmchat)"));
    assert!(out.stdout.contains("chrome/BUILD.gn"));
    assert!(out.stdout.contains("(modified, uncommitted)"));
    assert!(out.stdout.contains("refusing to touch a drifted tree."));
    assert!(
        out.stdout
            .contains("keep the edits →  commit them, then: bpatch extract <rev>")
    );
    assert!(
        out.stdout
            .contains("discard them  →  git checkout -- <file>")
    );
    assert!(out.stdout.contains("exit 3"));
    Ok(())
}

#[test]
fn sim5_agent_base_bump_conflict_continue_repin_round_trips_store() -> Result<()> {
    let scenario = conflict_scenario()?;
    let before = worktree_snapshot(scenario.checkout.path())?;

    let apply = run_bpatch(
        scenario.checkout.path(),
        Some(&scenario.store_dir),
        strs(&["apply", "--json"]),
    )?;
    assert_eq!(
        apply.code, 2,
        "stdout:\n{}\nstderr:\n{}",
        apply.stdout, apply.stderr
    );
    let json = parse_json(&apply.stdout)?;
    assert_eq!(json["result"], "conflicts");
    assert_eq!(json["base"], "149.0.7250.0");
    assert_eq!(json["merged"], 1);
    assert_eq!(json["worktree_touched"], false);
    assert_eq!(json["exit"], 2);
    assert_eq!(json["conflicts"][0]["feature"], "bootstrap");
    assert_eq!(worktree_snapshot(scenario.checkout.path())?, before);

    let materialized = run_bpatch(
        scenario.checkout.path(),
        Some(&scenario.store_dir),
        strs(&["continue", "--materialize"]),
    )?;
    assert_eq!(materialized.code, 0, "{}", materialized.stderr);
    assert!(
        materialized
            .stdout
            .contains("1 file written with conflict markers; 1 clean file staged for convergence")
    );
    let conflicted = scenario
        .checkout
        .read_file("chrome/app/chrome_main_delegate.cc")?;
    assert!(conflicted.contains("<<<<<<<"));
    assert_eq!(
        scenario
            .checkout
            .read_file("chrome/browser/ui/llmchat/clean.cc")?,
        "clean base\n"
    );

    scenario
        .checkout
        .write_file("chrome/app/chrome_main_delegate.cc", "resolved bootstrap\n")?;
    let continued = run_bpatch(
        scenario.checkout.path(),
        Some(&scenario.store_dir),
        strs(&["continue"]),
    )?;
    assert_eq!(continued.code, 0, "{}", continued.stderr);
    assert!(
        continued
            .stdout
            .contains("✓ converged on base 149.0.7250.0 · 2 feature commits authored")
    );
    assert!(continued.stdout.contains("Bpatch-Base: 149.0.7250.0"));

    let repin = run_bpatch(
        scenario.checkout.path(),
        Some(&scenario.store_dir),
        strs(&["extract", "--repin"]),
    )?;
    assert_eq!(repin.code, 0, "{}", repin.stderr);
    assert!(
        repin
            .stdout
            .contains("re-diffed 2 patches against base 149.0.7250.0")
    );
    assert!(
        repin
            .stdout
            .contains("store base pin: 148.0.7204.1 → 149.0.7250.0")
    );
    let store = Store::load(&scenario.store_dir)?;
    assert_eq!(store.metadata().base_commit, scenario.new_base);

    let second_root = tempfile::tempdir()?;
    Git::new(second_root.path()).run(&[
        "clone",
        scenario.checkout.path().to_str().expect("utf-8 path"),
        "checkout",
    ])?;
    let second = second_root.path().join("checkout");
    let second_git = Git::new(&second);
    second_git.run(&["config", "user.name", "Bpatch Test"])?;
    second_git.run(&["config", "user.email", "bpatch@example.com"])?;
    second_git.run(&["reset", "--hard", &scenario.new_base])?;

    let applied = run_bpatch(
        &second,
        Some(&scenario.store_dir),
        strs(&["apply", "--json"]),
    )?;
    assert_eq!(applied.code, 0, "{}", applied.stderr);
    assert_eq!(parse_json(&applied.stdout)?["result"], "applied");
    let converged = run_bpatch(
        &second,
        Some(&scenario.store_dir),
        strs(&["apply", "--json"]),
    )?;
    assert_eq!(
        parse_json(&converged.stdout)?["result"],
        "converged",
        "{}",
        converged.stdout
    );
    Ok(())
}

#[test]
fn sim6_needs_feature_json_then_named_feature_extracts() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_extract_base(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_extract_store(&store, &base)?;
    store.commit("seed store")?;
    checkout.write_file("chrome/browser/browseros/wallet/service.cc", "wallet cc\n")?;
    checkout.write_file("chrome/browser/browseros/wallet/service.h", "wallet h\n")?;
    checkout.write_file("chrome/browser/browseros/wallet/BUILD.gn", "wallet build\n")?;
    checkout.write_file(
        "chrome/browser/browseros/BUILD.gn",
        "browseros build changed\n",
    )?;
    let rev = checkout.commit("wallet")?;

    let needs = run_bpatch(
        checkout.path(),
        Some(&store_dir),
        vec!["extract".into(), rev.clone(), "--json".into()],
    )?;
    assert_eq!(needs.code, 3);
    assert!(needs.stderr.is_empty());
    let json = parse_json(&needs.stdout)?;
    assert_eq!(json["result"], "needs-feature");
    assert_eq!(json["suggestion"], "wallet");
    assert_eq!(json["unmatched"].as_array().unwrap().len(), 3);
    assert_eq!(json["exit"], 3);
    assert!(!needs.stdout.contains("? feature"));

    let extracted = run_bpatch(
        checkout.path(),
        Some(&store_dir),
        vec![
            "extract".into(),
            rev,
            "--feature".into(),
            "wallet".into(),
            "--json".into(),
        ],
    )?;
    assert_eq!(extracted.code, 0, "{}", extracted.stderr);
    let json = parse_json(&extracted.stdout)?;
    assert_eq!(json["result"], "extracted");
    assert_eq!(json["patches"], 4);
    assert_eq!(json["new_features"][0], "wallet");
    assert_eq!(json["exit"], 0);
    Ok(())
}

#[test]
fn sim7_annotate_commits_claimed_resources_and_leaves_leftovers_then_rest() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_annotate_base(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_annotate_store(&store, &base)?;
    store.commit("seed annotate store")?;

    checkout.write_file("chrome/browser/browseros/core/service.cc", "core changed\n")?;
    checkout.plant_untracked("chrome/browser/browseros/core/new_file.cc", "new core\n")?;
    checkout.write_file("chrome/BROWSEROS_VERSION", "148.0.7204.1-browseros\n")?;
    checkout.plant_untracked("chrome/app/theme/chromium/logo.png", "logo\n")?;
    checkout.plant_untracked("chrome/browser/browseros/wallet/service.cc", "wallet\n")?;

    let annotated = run_bpatch(checkout.path(), Some(&store_dir), strs(&["annotate"]))?;
    assert_eq!(annotated.code, 3, "{}", annotated.stderr);
    assert!(annotated.stdout.contains("claimed 2 -> 1 feature commit"));
    assert!(
        annotated
            .stdout
            .contains("resources 2 -> 1 commit \"resource: bos_build outputs\"")
    );
    assert!(
        annotated
            .stdout
            .contains("unclaimed 1 (left in working tree):")
    );
    assert!(
        annotated
            .stdout
            .contains("chrome/browser/browseros/wallet/service.cc")
    );
    assert!(annotated.stdout.contains("bpatch feature add wallet"));

    let subjects = checkout.git().run_str(&["log", "--format=%s", "-2"])?;
    assert!(
        subjects
            .lines()
            .any(|line| line == "resource: bos_build outputs")
    );
    assert!(
        subjects
            .lines()
            .any(|line| line == "chore: browseros core infrastructure")
    );
    assert!(
        checkout
            .git()
            .run_str(&[
                "ls-tree",
                "-r",
                "--name-only",
                "HEAD",
                "--",
                "chrome/browser/browseros/core/new_file.cc",
            ])?
            .contains("chrome/browser/browseros/core/new_file.cc")
    );

    let message = checkout.git().run_str(&["log", "-1", "--format=%B"])?;
    assert!(message.contains(TRAILER_BASE));
    assert!(message.contains(TRAILER_ANNOTATED));
    assert!(!message.contains(TRAILER_STORE_REV));
    let status = checkout.status_porcelain()?;
    assert!(status.contains("chrome/browser/browseros/wallet"));
    assert!(!status.contains("chrome/browser/browseros/core/new_file.cc"));
    assert!(!status.contains("chrome/BROWSEROS_VERSION"));

    let rest = run_bpatch(
        checkout.path(),
        Some(&store_dir),
        strs(&["annotate", "--rest", "wip-frame-experiments"]),
    )?;
    assert_eq!(rest.code, 0, "{}", rest.stderr);
    assert!(
        rest.stdout
            .contains("rest 1 -> 1 commit \"wip: wip-frame-experiments\"")
    );
    assert_eq!(
        checkout.git().run_str(&["log", "-1", "--format=%s"])?,
        "wip: wip-frame-experiments"
    );
    assert!(checkout.status_porcelain()?.is_empty());
    Ok(())
}

#[test]
fn sim8_annotate_triage_json_then_feature_add_round_trip() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_annotate_base(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_annotate_store(&store, &base)?;
    store.commit("seed annotate store")?;

    checkout.plant_untracked("chrome/browser/browseros/wallet/service.cc", "wallet\n")?;

    let triage = run_bpatch(
        checkout.path(),
        Some(&store_dir),
        strs(&["annotate", "--triage", "--json"]),
    )?;
    assert_eq!(triage.code, 3, "{}", triage.stderr);
    let json = parse_json(&triage.stdout)?;
    assert_eq!(json["result"], "triage");
    assert_eq!(
        json["unclaimed"][0]["path"],
        "chrome/browser/browseros/wallet/service.cc"
    );
    assert!(
        json["unclaimed"][0]["command"]
            .as_str()
            .expect("command")
            .contains("bpatch feature add wallet chrome/browser/browseros/wallet/")
    );

    let add = run_bpatch(
        checkout.path(),
        Some(&store_dir),
        strs(&[
            "feature",
            "add",
            "wallet",
            "chrome/browser/browseros/wallet/",
            "--desc",
            "feat: wallet",
            "--json",
        ]),
    )?;
    assert_eq!(add.code, 0, "{}", add.stderr);
    assert_eq!(parse_json(&add.stdout)?["result"], "feature-added");

    let annotated = run_bpatch(
        checkout.path(),
        Some(&store_dir),
        strs(&["annotate", "--json"]),
    )?;
    assert_eq!(annotated.code, 0, "{}", annotated.stderr);
    let json = parse_json(&annotated.stdout)?;
    assert_eq!(json["result"], "annotated");
    assert_eq!(json["claimed"], 1);
    assert!(checkout.status_porcelain()?.is_empty());
    Ok(())
}

#[test]
fn sim9_store_false_resource_commit_is_clean_for_status_and_diff() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_annotate_base(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_annotate_store(&store, &base)?;
    store.commit("seed annotate store")?;

    checkout.write_file("chrome/BROWSEROS_VERSION", "resource changed\n")?;
    checkout.plant_untracked(".browseros-patch/state.json", "{}\n")?;
    let annotated = run_bpatch(
        checkout.path(),
        Some(&store_dir),
        strs(&["annotate", "--json"]),
    )?;
    assert_eq!(annotated.code, 0, "{}", annotated.stderr);
    let json = parse_json(&annotated.stdout)?;
    assert_eq!(json["result"], "annotated");
    assert_eq!(json["resources"], 1);

    let status = run_bpatch(
        checkout.path(),
        Some(&store_dir),
        strs(&["status", "--json"]),
    )?;
    assert_eq!(status.code, 0, "{}", status.stderr);
    let json = parse_json(&status.stdout)?;
    assert_eq!(json["result"], "clean");
    assert_eq!(json["drift"].as_array().expect("drift").len(), 0);

    let diff = run_bpatch(checkout.path(), Some(&store_dir), strs(&["diff", "--json"]))?;
    assert_eq!(diff.code, 0, "{}", diff.stderr);
    let json = parse_json(&diff.stdout)?;
    assert_eq!(json["result"], "converged");
    assert_eq!(json["files_changed"], 0);

    let triage = run_bpatch(
        checkout.path(),
        Some(&store_dir),
        strs(&["annotate", "--triage", "--json"]),
    )?;
    assert_eq!(triage.code, 0, "{}", triage.stderr);
    assert_eq!(parse_json(&triage.stdout)?["result"], "clean");
    Ok(())
}

#[test]
fn sim10_feature_add_from_dirty_collapses_dirs_and_is_idempotent() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_annotate_base(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_capture_store(&store, &base)?;
    store.commit("seed capture store")?;

    checkout.write_file("chrome/app/theme/chromium/icon16.png", "icon16 changed\n")?;
    checkout.write_file("chrome/app/theme/chromium/icon32.png", "icon32 changed\n")?;
    checkout.write_file("chrome/BROWSEROS_VERSION", "resource changed\n")?;

    let add = run_bpatch(
        checkout.path(),
        Some(&store_dir),
        strs(&[
            "feature",
            "add",
            "build-resources",
            "--store=false",
            "--from-dirty",
            "--json",
        ]),
    )?;
    assert_eq!(add.code, 0, "{}", add.stderr);
    let json = parse_json(&add.stdout)?;
    assert_eq!(json["result"], "feature-added");
    assert_eq!(json["store"], false);
    assert_eq!(json["appended"], 3);
    assert_eq!(json["skipped"], 0);
    assert_eq!(json["collapsed_dirs"], 1);
    assert_eq!(json["collapsed_files"], 1);
    let features = fs::read_to_string(store_dir.join(".features.yaml"))?;
    assert!(features.contains("    store: false"));
    assert!(features.contains("      - \"chrome/app/theme/chromium/\""));
    assert!(features.contains("      - \"chrome/BROWSEROS_VERSION\""));

    let repeat = run_bpatch(
        checkout.path(),
        Some(&store_dir),
        strs(&[
            "feature",
            "add",
            "build-resources",
            "--store=false",
            "--from-dirty",
            "--json",
        ]),
    )?;
    assert_eq!(repeat.code, 0, "{}", repeat.stderr);
    let json = parse_json(&repeat.stdout)?;
    assert_eq!(json["appended"], 0);
    assert_eq!(json["skipped"], 3);
    Ok(())
}

#[test]
fn config_discovery_runs_from_subdirectory_and_missing_store_is_actionable() -> Result<()> {
    let scenario = applied_rev1_scenario()?;
    let home = tempfile::tempdir()?;
    let config_dir = home.path().join(".config/bpatch");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("config.toml"),
        format!("store = {:?}\n", scenario.store_dir.display().to_string()),
    )?;
    let subdir = scenario.checkout.path().join("chrome/browser/ui");

    let status = run_bpatch_with_home(&subdir, None, strs(&["status"]), home.path())?;
    assert_eq!(status.code, 0, "{}", status.stderr);
    assert!(status.stdout.contains("base     148.0.7204.1"));

    let missing_home = tempfile::tempdir()?;
    let missing = run_bpatch_with_home(
        scenario.checkout.path(),
        None,
        strs(&["status", "--json"]),
        missing_home.path(),
    )?;
    assert_eq!(missing.code, 1);
    let json = parse_json(&missing.stdout)?;
    assert_eq!(json["result"], "error");
    assert!(json["reason"].as_str().unwrap().contains("--store <dir>"));
    assert!(
        json["reason"]
            .as_str()
            .unwrap()
            .contains("bpatch init <dir>")
    );
    assert!(json["reason"].as_str().unwrap().contains("config.toml"));
    assert_eq!(json["exit"], 1);

    let malformed_home = tempfile::tempdir()?;
    let malformed_config_dir = malformed_home.path().join(".config/bpatch");
    fs::create_dir_all(&malformed_config_dir)?;
    fs::write(malformed_config_dir.join("config.toml"), "store = [")?;
    let malformed_json = run_bpatch_with_home(
        scenario.checkout.path(),
        None,
        strs(&["status", "--json"]),
        malformed_home.path(),
    )?;
    assert_eq!(malformed_json.code, 1);
    let json = parse_json(&malformed_json.stdout)?;
    let reason = json["reason"].as_str().expect("reason string");
    assert!(reason.contains("parsing"));
    assert!(reason.contains("config.toml"));
    assert!(reason.contains("expected"));

    let malformed_human = run_bpatch_with_home(
        scenario.checkout.path(),
        None,
        strs(&["status"]),
        malformed_home.path(),
    )?;
    assert_eq!(malformed_human.code, 1);
    assert!(malformed_human.stdout.is_empty());
    assert!(malformed_human.stderr.contains("error: parsing"));
    assert!(malformed_human.stderr.contains("expected"));
    Ok(())
}

#[test]
fn wrong_checkout_refuses_status_diff_apply_with_guidance() -> Result<()> {
    let scenario = applied_rev1_scenario()?;
    let wrong_checkout = scenario.store.path();
    let verbs = ["status", "diff", "apply"];

    for verb in verbs {
        let out = run_bpatch(wrong_checkout, Some(&scenario.store_dir), strs(&[verb]))?;
        assert_eq!(out.code, 3, "{verb} stderr:\n{}", out.stderr);
        assert!(out.stdout.is_empty());
        assert_wrong_checkout_reason(&out.stderr, wrong_checkout, &scenario.store_dir);
        assert!(!out.stderr.contains("git apply --cached"));

        let json_out = run_bpatch(
            wrong_checkout,
            Some(&scenario.store_dir),
            strs(&[verb, "--json"]),
        )?;
        assert_eq!(json_out.code, 3, "{verb} json stdout:\n{}", json_out.stdout);
        assert!(json_out.stderr.is_empty());
        let json = parse_json(&json_out.stdout)?;
        assert_eq!(json["result"], "error");
        assert_eq!(json["exit"], 3);
        assert_wrong_checkout_reason(
            json["reason"].as_str().expect("reason string"),
            wrong_checkout,
            &scenario.store_dir,
        );
    }

    Ok(())
}

#[test]
fn legacy_store_loads_and_first_write_migrates_metadata_names() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_extract_base(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_legacy_extract_store(&store, &base)?;
    store.commit("seed legacy store")?;

    let list = run_bpatch(
        checkout.path(),
        Some(&store_dir),
        strs(&["feature", "list", "--json"]),
    )?;
    assert_eq!(list.code, 0, "{}", list.stderr);
    let json = parse_json(&list.stdout)?;
    assert_eq!(json["features"][0]["name"], "bootstrap");
    assert!(store_dir.join("store.yaml").exists());
    assert!(store_dir.join("features.yaml").exists());
    assert!(!store_dir.join(".store.yaml").exists());
    assert!(!store_dir.join(".features.yaml").exists());

    let added = run_bpatch(
        checkout.path(),
        Some(&store_dir),
        strs(&[
            "feature",
            "add",
            "wallet",
            "--path",
            "chrome/browser/browseros/wallet/",
            "--json",
        ]),
    )?;
    assert_eq!(added.code, 0, "{}", added.stderr);
    let json = parse_json(&added.stdout)?;
    assert_eq!(json["result"], "feature-added");

    assert!(store_dir.join(".store.yaml").exists());
    assert!(store_dir.join(".features.yaml").exists());
    assert!(!store_dir.join("store.yaml").exists());
    assert!(!store_dir.join("features.yaml").exists());
    Ok(())
}

#[test]
fn store_with_both_metadata_spellings_refuses_to_load() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_extract_base(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_extract_store(&store, &base)?;
    fs::write(
        store_dir.join("store.yaml"),
        fs::read(store_dir.join(".store.yaml"))?,
    )?;
    store.commit("seed ambiguous store")?;

    let out = run_bpatch(
        checkout.path(),
        Some(&store_dir),
        strs(&["feature", "list", "--json"]),
    )?;
    assert_eq!(out.code, 1);
    let json = parse_json(&out.stdout)?;
    let reason = json["reason"].as_str().expect("reason");
    assert!(reason.contains("both .store.yaml and legacy store.yaml"));
    assert!(reason.contains("remove one spelling"));
    Ok(())
}

#[test]
fn init_writes_fresh_config_and_status_uses_it() -> Result<()> {
    let scenario = applied_rev1_scenario()?;
    let home = tempfile::tempdir()?;
    let outside_checkout = tempfile::tempdir()?;
    let config = home.path().join(".config/bpatch/config.toml");
    let store = scenario.store_dir.canonicalize()?;

    let init = run_bpatch_with_home(
        outside_checkout.path(),
        None,
        vec!["init".into(), scenario.store_dir.display().to_string()],
        home.path(),
    )?;
    assert_eq!(init.code, 0, "{}", init.stderr);
    assert!(init.stderr.is_empty());
    assert!(init.stdout.contains("initialized store "));
    assert!(init.stdout.contains(&store.display().to_string()));
    assert!(init.stdout.contains(&config.display().to_string()));

    let config_text = fs::read_to_string(&config)?;
    assert_eq!(config_text, format!("store = \"{}\"\n", store.display()));

    let status = run_bpatch_with_home(
        scenario.checkout.path(),
        None,
        strs(&["status"]),
        home.path(),
    )?;
    assert_eq!(status.code, 0, "{}", status.stderr);
    assert!(status.stdout.contains("base     148.0.7204.1"));
    Ok(())
}

#[test]
fn init_preserves_existing_config_comments_and_keys() -> Result<()> {
    let scenario = applied_rev1_scenario()?;
    let home = tempfile::tempdir()?;
    let config_dir = home.path().join(".config/bpatch");
    let config = config_dir.join("config.toml");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        &config,
        r#"# bpatch config
store = "/tmp/old-store" # keep store note

[checkouts]
# keep alias note
main = "/tmp/chromium"
"#,
    )?;

    let init = run_bpatch_with_home(
        scenario.checkout.path(),
        None,
        vec!["init".into(), scenario.store_dir.display().to_string()],
        home.path(),
    )?;
    assert_eq!(init.code, 0, "{}", init.stderr);

    let store = scenario.store_dir.canonicalize()?;
    let config_text = fs::read_to_string(&config)?;
    assert!(config_text.contains("# bpatch config"));
    assert!(config_text.contains(&format!(
        "store = \"{}\" # keep store note",
        store.display()
    )));
    assert!(config_text.contains("[checkouts]"));
    assert!(config_text.contains("# keep alias note"));
    assert!(config_text.contains("main = \"/tmp/chromium\""));

    let alias_only_home = tempfile::tempdir()?;
    let alias_only_config_dir = alias_only_home.path().join(".config/bpatch");
    let alias_only_config = alias_only_config_dir.join("config.toml");
    fs::create_dir_all(&alias_only_config_dir)?;
    fs::write(
        &alias_only_config,
        r#"[checkouts]
# keep alias-only note
main = "/tmp/chromium"
"#,
    )?;
    let init = run_bpatch_with_home(
        scenario.checkout.path(),
        None,
        vec!["init".into(), scenario.store_dir.display().to_string()],
        alias_only_home.path(),
    )?;
    assert_eq!(init.code, 0, "{}", init.stderr);
    let alias_only_text = fs::read_to_string(&alias_only_config)?;
    assert!(alias_only_text.starts_with(&format!("store = \"{}\"\n", store.display())));
    assert!(alias_only_text.contains("[checkouts]"));
    assert!(alias_only_text.contains("# keep alias-only note"));
    assert!(alias_only_text.contains("main = \"/tmp/chromium\""));
    Ok(())
}

#[test]
fn extract_refuses_root_metadata_patch_paths() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_extract_base(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_extract_store(&store, &base)?;
    store.commit("seed store")?;

    checkout.write_file(".store.yaml", "not bpatch metadata\n")?;
    let rev = checkout.commit("bad metadata-like file")?;
    let out = run_bpatch(
        checkout.path(),
        Some(&store_dir),
        vec![
            "extract".into(),
            rev,
            "--feature".into(),
            "metadata".into(),
            "--json".into(),
        ],
    )?;

    assert_eq!(out.code, 1);
    let json = parse_json(&out.stdout)?;
    assert!(
        json["reason"]
            .as_str()
            .expect("reason")
            .contains("store path .store.yaml is reserved")
    );
    Ok(())
}

#[test]
fn init_rejects_directory_without_store_metadata() -> Result<()> {
    let home = tempfile::tempdir()?;
    let not_store = tempfile::tempdir()?;

    let init = run_bpatch_with_home(
        not_store.path(),
        None,
        vec!["init".into(), not_store.path().display().to_string()],
        home.path(),
    )?;
    assert_eq!(init.code, 1);
    assert!(init.stdout.is_empty());
    assert!(init.stderr.contains("missing .store.yaml"));
    assert!(!home.path().join(".config/bpatch/config.toml").exists());
    Ok(())
}

#[test]
fn fixture_store_visible_files_are_patch_files_and_nested_dot_patches_load() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_extract_base(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_extract_store(&store, &base)?;

    checkout.write_file("chrome/browser/ui/llmchat/panel.cc", "panel changed\n")?;
    checkout.write_file(
        "chrome/browser/browseros/bundled_extensions/.gitignore",
        "bundle-cache\n",
    )?;
    checkout.git().run(&["add", "-A"])?;
    commit_store_from_index(
        &store,
        &checkout,
        &base,
        &[
            "chrome/browser/ui/llmchat/panel.cc",
            "chrome/browser/browseros/bundled_extensions/.gitignore",
        ],
        "store patches",
    )?;

    assert_visible_files_are_patches(&store_dir)?;
    let model = Store::load(&store_dir)?;
    assert!(
        model
            .patches()
            .contains_key("chrome/browser/browseros/bundled_extensions/.gitignore")
    );
    Ok(())
}

#[test]
fn init_defaults_to_cwd_only_when_cwd_is_store() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_extract_base(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_extract_store(&store, &base)?;
    let home = tempfile::tempdir()?;

    let init = run_bpatch_with_home(&store_dir, None, strs(&["init"]), home.path())?;
    assert_eq!(init.code, 0, "{}", init.stderr);
    let config_text = fs::read_to_string(home.path().join(".config/bpatch/config.toml"))?;
    assert!(config_text.contains(&store_dir.canonicalize()?.display().to_string()));

    let missing_home = tempfile::tempdir()?;
    let missing =
        run_bpatch_with_home(checkout.path(), None, strs(&["init"]), missing_home.path())?;
    assert_eq!(missing.code, 1);
    assert!(missing.stderr.contains("init requires <STORE_DIR>"));
    assert!(missing.stderr.contains("bpatch init <dir>"));
    Ok(())
}

#[test]
fn init_json_output_is_single_object() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    let base = write_extract_base(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_extract_store(&store, &base)?;
    let home = tempfile::tempdir()?;
    let config = home.path().join(".config/bpatch/config.toml");

    let init = run_bpatch_with_home(&store_dir, None, strs(&["init", "--json"]), home.path())?;
    assert_eq!(init.code, 0, "{}", init.stderr);
    assert!(init.stderr.is_empty());
    assert_eq!(init.stdout.lines().count(), 1);
    let json = parse_json(&init.stdout)?;
    assert_eq!(json["result"], "initialized");
    assert_eq!(
        json["store"],
        store_dir.canonicalize()?.display().to_string()
    );
    assert_eq!(json["config"], config.display().to_string());
    assert_eq!(json["exit"], 0);
    Ok(())
}

#[test]
fn checkout_aliases_and_paths_target_fixture_from_unrelated_cwd() -> Result<()> {
    let scenario = applied_rev1_scenario()?;
    write_checkout_rev2(&scenario.checkout, false)?;
    commit_store_from_index(
        &scenario.store,
        &scenario.checkout,
        &scenario.base,
        &[
            "chrome/browser/ui/llmchat/panel.cc",
            "chrome/browser/ui/llmchat/panel.h",
            "chrome/browser/ui/llmchat/resize_util.cc",
        ],
        "store rev2",
    )?;
    scenario
        .checkout
        .git()
        .run(&["reset", "--hard", &scenario.rev1_commit])?;

    let home = tempfile::tempdir()?;
    write_bpatch_config(
        home.path(),
        Some(&scenario.store_dir),
        &[("ch1", scenario.checkout.path())],
    )?;
    let unrelated = tempfile::tempdir()?;

    let status = run_bpatch_with_home(
        unrelated.path(),
        None,
        strs(&["status", "ch1"]),
        home.path(),
    )?;
    assert_eq!(status.code, 0, "{}", status.stderr);
    assert!(status.stdout.contains("base     148.0.7204.1"));

    let flag_status = run_bpatch_with_home(
        unrelated.path(),
        None,
        strs(&["-C", "ch1", "status", "--json"]),
        home.path(),
    )?;
    assert_eq!(flag_status.code, 0, "{}", flag_status.stderr);
    let json = parse_json(&flag_status.stdout)?;
    assert_eq!(json["result"], "clean");

    let diff = run_bpatch_with_home(unrelated.path(), None, strs(&["diff", "ch1"]), home.path())?;
    assert_eq!(diff.code, 0, "{}", diff.stderr);
    assert!(diff.stdout.contains("apply would touch 3 files"));

    let apply = run_bpatch_with_home(unrelated.path(), None, strs(&["apply", "ch1"]), home.path())?;
    assert_eq!(apply.code, 0, "{}", apply.stderr);
    assert!(apply.stdout.contains("apply: store "));
    assert!(apply.stdout.contains("✓ 3 files written"));

    let raw_path = scenario.checkout.path().display().to_string();
    let raw = run_bpatch_with_home(
        unrelated.path(),
        None,
        vec!["status".into(), raw_path],
        home.path(),
    )?;
    assert_eq!(raw.code, 0, "{}", raw.stderr);
    assert!(raw.stdout.contains("tree     clean"));

    let repin = run_bpatch_with_home(
        unrelated.path(),
        None,
        strs(&["-C", "ch1", "extract", "--repin", "--json"]),
        home.path(),
    )?;
    assert_eq!(repin.code, 0, "{}", repin.stderr);
    let json = parse_json(&repin.stdout)?;
    assert_eq!(json["result"], "repinned");

    let unknown = run_bpatch_with_home(
        unrelated.path(),
        None,
        strs(&["status", "missing", "--json"]),
        home.path(),
    )?;
    assert_eq!(unknown.code, 1);
    let json = parse_json(&unknown.stdout)?;
    assert_eq!(json["result"], "error");
    assert!(
        json["reason"]
            .as_str()
            .unwrap()
            .contains("unknown checkout `missing`")
    );
    assert!(
        json["reason"]
            .as_str()
            .unwrap()
            .contains("known aliases: ch1")
    );

    let other = FixtureRepo::new()?;
    let disagree = run_bpatch_with_home(
        unrelated.path(),
        None,
        vec![
            "-C".into(),
            "ch1".into(),
            "status".into(),
            other.path().display().to_string(),
            "--json".into(),
        ],
        home.path(),
    )?;
    assert_eq!(disagree.code, 1);
    let json = parse_json(&disagree.stdout)?;
    assert!(
        json["reason"]
            .as_str()
            .unwrap()
            .contains("resolve to different checkouts"),
        "{}",
        json["reason"]
    );
    Ok(())
}

#[test]
fn alias_add_list_remove_round_trips_config_and_json() -> Result<()> {
    let checkout = FixtureRepo::new()?;
    write_apply_base(&checkout)?;
    let home = tempfile::tempdir()?;
    let config_dir = home.path().join(".config/bpatch");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("config.toml"),
        "# keep this comment\ncustom = \"preserve\"\n",
    )?;
    let cwd = tempfile::tempdir()?;

    let add = run_bpatch_with_home(
        cwd.path(),
        None,
        vec![
            "--json".into(),
            "alias".into(),
            "add".into(),
            "ch1".into(),
            checkout.path().display().to_string(),
        ],
        home.path(),
    )?;
    assert_eq!(add.code, 0, "{}", add.stderr);
    let json = parse_json(&add.stdout)?;
    assert_eq!(json["result"], "added");
    assert_eq!(json["alias"], "ch1");
    assert_eq!(json["exit"], 0);
    assert_eq!(
        json["path"].as_str().unwrap(),
        checkout.path().canonicalize()?.to_str().unwrap()
    );
    let after_add = fs::read_to_string(config_dir.join("config.toml"))?;
    assert!(after_add.contains("# keep this comment"));
    assert!(after_add.contains("custom = \"preserve\""));
    assert!(after_add.contains("[checkouts]"));
    assert!(after_add.contains("ch1"));

    let list = run_bpatch_with_home(
        cwd.path(),
        None,
        strs(&["alias", "list", "--json"]),
        home.path(),
    )?;
    assert_eq!(list.code, 0, "{}", list.stderr);
    let json = parse_json(&list.stdout)?;
    assert_eq!(json["result"], "listed");
    assert_eq!(
        json["checkouts"]["ch1"],
        checkout.path().canonicalize()?.to_str().unwrap()
    );
    assert_eq!(json["exit"], 0);

    let scoped_alias = run_bpatch_with_home(
        cwd.path(),
        None,
        strs(&["-C", "ch1", "alias", "list", "--json"]),
        home.path(),
    )?;
    assert_eq!(scoped_alias.code, 1);
    let json = parse_json(&scoped_alias.stdout)?;
    assert_eq!(json["result"], "error");
    assert!(
        json["reason"]
            .as_str()
            .unwrap()
            .contains("alias commands do not accept -C/--checkout")
    );

    let remove = run_bpatch_with_home(
        cwd.path(),
        None,
        strs(&["alias", "remove", "ch1", "--json"]),
        home.path(),
    )?;
    assert_eq!(remove.code, 0, "{}", remove.stderr);
    let json = parse_json(&remove.stdout)?;
    assert_eq!(json["result"], "removed");
    assert_eq!(json["alias"], "ch1");
    assert_eq!(json["exit"], 0);
    let after_remove = fs::read_to_string(config_dir.join("config.toml"))?;
    assert!(after_remove.contains("# keep this comment"));
    assert!(after_remove.contains("custom = \"preserve\""));
    assert!(!after_remove.contains("[checkouts]"));
    assert!(!after_remove.contains("ch1 ="));
    Ok(())
}

fn run_bpatch(cwd: &Path, store: Option<&Path>, args: Vec<String>) -> Result<CmdOutput> {
    let home = tempfile::tempdir()?;
    run_bpatch_with_home(cwd, store, args, home.path())
}

fn run_bpatch_with_home(
    cwd: &Path,
    store: Option<&Path>,
    args: Vec<String>,
    home: &Path,
) -> Result<CmdOutput> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bpatch"));
    command.current_dir(cwd).env("HOME", home);
    if let Some(store) = store {
        command.arg("--store").arg(store);
    }
    command.args(args);
    let output = command.output().context("running bpatch")?;
    Ok(CmdOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8(output.stdout)?,
        stderr: String::from_utf8(output.stderr)?,
    })
}

fn strs(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_string()).collect()
}

fn parse_json(stdout: &str) -> Result<Value> {
    Ok(serde_json::from_str(stdout.trim())?)
}

fn write_bpatch_config(
    home: &Path,
    store: Option<&Path>,
    checkouts: &[(&str, &Path)],
) -> Result<()> {
    let config_dir = home.join(".config/bpatch");
    fs::create_dir_all(&config_dir)?;
    let mut text = String::new();
    if let Some(store) = store {
        text.push_str(&format!("store = {:?}\n", store.display().to_string()));
    }
    if !checkouts.is_empty() {
        text.push_str("\n[checkouts]\n");
        for (alias, path) in checkouts {
            text.push_str(&format!("{} = {:?}\n", alias, path.display().to_string()));
        }
    }
    fs::write(config_dir.join("config.toml"), text)?;
    Ok(())
}

fn assert_wrong_checkout_reason(reason: &str, checkout: &Path, store_dir: &Path) {
    assert!(reason.contains(&checkout.display().to_string()), "{reason}");
    assert!(
        reason.contains(&store_dir.display().to_string()),
        "{reason}"
    );
    assert!(
        reason.contains("does not look like a Chromium checkout"),
        "{reason}"
    );
    assert!(
        reason.contains("chrome/browser/ui/llmchat/panel.cc not in index"),
        "{reason}"
    );
    assert!(
        reason.contains("cd into your chromium checkout and re-run"),
        "{reason}"
    );
}

fn applied_rev1_scenario() -> Result<AppliedScenario> {
    let checkout = FixtureRepo::new()?;
    let base = write_apply_base(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_apply_store(&store, &base)?;

    write_checkout_rev1(&checkout)?;
    checkout.git().run(&["add", "-A"])?;
    let rev1_tree = checkout.git().run_str(&["write-tree"])?;
    let rev1_store = commit_store_from_index(
        &store,
        &checkout,
        &base,
        &[
            "chrome/browser/ui/llmchat/panel.cc",
            "chrome/browser/ui/llmchat/panel.h",
        ],
        "store rev1",
    )?;
    let rev1_commit = checkout.commit_with_trailers(
        "feat: llmchat",
        &[
            (TRAILER_STORE_REV, rev1_store.as_str()),
            (TRAILER_BASE, base.as_str()),
            (TRAILER_TREE, rev1_tree.as_str()),
        ],
    )?;

    Ok(AppliedScenario {
        checkout,
        store,
        store_dir,
        base,
        rev1_commit,
    })
}

fn write_apply_base(repo: &FixtureRepo) -> Result<String> {
    repo.write_file(
        "chrome/VERSION",
        "MAJOR=148\nMINOR=0\nBUILD=7204\nPATCH=1\n",
    )?;
    repo.write_file("chrome/browser/ui/llmchat/panel.cc", "base panel\n")?;
    repo.write_file("chrome/browser/ui/llmchat/panel.h", "base header\n")?;
    repo.write_file("chrome/BUILD.gn", "base build\n")?;
    repo.commit("Chromium 148.0.7204.1")
}

fn write_checkout_rev1(repo: &FixtureRepo) -> Result<()> {
    repo.write_file("chrome/browser/ui/llmchat/panel.cc", "applied panel\n")?;
    repo.write_file("chrome/browser/ui/llmchat/panel.h", "applied header\n")
}

fn write_checkout_rev2(repo: &FixtureRepo, include_build: bool) -> Result<()> {
    repo.write_file("chrome/browser/ui/llmchat/panel.cc", "current panel\n")?;
    repo.write_file("chrome/browser/ui/llmchat/panel.h", "current header\n")?;
    repo.write_file("chrome/browser/ui/llmchat/resize_util.cc", "resize\n")?;
    if include_build {
        repo.write_file("chrome/BUILD.gn", "current build\n")?;
    }
    repo.git().run(&["add", "-A"])?;
    Ok(())
}

fn seed_apply_store(store: &FixtureRepo, base: &str) -> Result<PathBuf> {
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
    store.commit("seed store")?;
    Ok(store.path().join("chromium_patches"))
}

fn write_extract_base(repo: &FixtureRepo) -> Result<String> {
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

fn seed_extract_store(store: &FixtureRepo, base: &str) -> Result<PathBuf> {
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

fn write_annotate_base(repo: &FixtureRepo) -> Result<String> {
    repo.write_file(
        "chrome/VERSION",
        "MAJOR=148\nMINOR=0\nBUILD=7204\nPATCH=1\n",
    )?;
    repo.write_file("chrome/browser/browseros/core/service.cc", "core base\n")?;
    repo.write_file("chrome/BROWSEROS_VERSION", "base resource\n")?;
    repo.write_file("chrome/app/theme/chromium/icon16.png", "icon16 base\n")?;
    repo.write_file("chrome/app/theme/chromium/icon32.png", "icon32 base\n")?;
    repo.commit("Chromium 148.0.7204.1")
}

fn seed_annotate_store(store: &FixtureRepo, base: &str) -> Result<PathBuf> {
    let store_dir = seed_capture_store(store, base)?;
    store.write_file(
        "chromium_patches/.features.yaml",
        r#"version: "1.0"
features:
  browseros-core:
    description: "chore: browseros core infrastructure"
    files:
      - chrome/browser/browseros/core/
  build-resources:
    description: "resource: bos_build outputs"
    store: false
    files:
      - chrome/BROWSEROS_VERSION
      - chrome/app/theme/chromium/
"#,
    )?;
    Ok(store_dir)
}

fn seed_capture_store(store: &FixtureRepo, base: &str) -> Result<PathBuf> {
    store.write_file(
        "chromium_patches/.store.yaml",
        format!("base_commit: {base}\nbase_version: \"148.0.7204.1\"\n"),
    )?;
    store.write_file(
        "chromium_patches/.features.yaml",
        r#"version: "1.0"
features:
  browseros-core:
    description: "chore: browseros core infrastructure"
    files:
      - chrome/browser/browseros/core/
"#,
    )?;
    Ok(store.path().join("chromium_patches"))
}

fn seed_legacy_extract_store(store: &FixtureRepo, base: &str) -> Result<PathBuf> {
    let store_dir = seed_extract_store(store, base)?;
    fs::rename(store_dir.join(".store.yaml"), store_dir.join("store.yaml"))?;
    fs::rename(
        store_dir.join(".features.yaml"),
        store_dir.join("features.yaml"),
    )?;
    Ok(store_dir)
}

fn assert_visible_files_are_patches(root: &Path) -> Result<()> {
    fn visit(root: &Path, dir: &Path) -> Result<()> {
        for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                visit(root, &path)?;
                continue;
            }
            if !file_type.is_file()
                || path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with('.'))
            {
                continue;
            }
            let contents =
                fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            assert!(
                contents.starts_with(b"diff --git "),
                "{} is visible but not a patch",
                path.strip_prefix(root)?.display()
            );
        }
        Ok(())
    }

    visit(root, root)
}

struct ConflictScenario {
    checkout: FixtureRepo,
    _store: FixtureRepo,
    store_dir: PathBuf,
    new_base: String,
}

fn conflict_scenario() -> Result<ConflictScenario> {
    let checkout = FixtureRepo::new()?;
    let old_base = write_old_base(&checkout)?;
    let store = FixtureRepo::new()?;
    let store_dir = seed_conflict_store(&store, &old_base)?;

    checkout.write_file("chrome/app/chrome_main_delegate.cc", "feature bootstrap\n")?;
    checkout.write_file("chrome/browser/ui/llmchat/clean.cc", "clean feature\n")?;
    checkout.git().run(&["add", "-A"])?;
    commit_store_from_index(
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
        _store: store,
        store_dir,
        new_base,
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

fn seed_conflict_store(store: &FixtureRepo, base: &str) -> Result<PathBuf> {
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
