//! Neo: Reclassifies saved pairs without browsers, preserving the original scoreboard and evidence.
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use neo_playwright_tests::model::{Case, Pair, ResultRecord};
use neo_playwright_tests::report;
use serde_json::{Value, json};

fn classify_saved(run: &Path, old: &Value) -> Result<(ResultRecord, bool), Box<dyn Error>> {
    let case: Case = serde_json::from_value(old["case"].clone())?;
    let prefix: String = case.name.chars().take(100).collect();
    let suffix = case.name.rsplit("__").next().ok_or("invalid case name")?;
    let pair_path = run
        .join("cases")
        .join(format!("{prefix}-{suffix}"))
        .join("pair.json");
    let pair = fs::read(&pair_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Pair>(&bytes).ok());
    if let Some(pair) = pair {
        if pair.entry.key != case.key {
            return Err(format!("wrong case evidence: {}", pair_path.display()).into());
        }
        return Ok((report::classify(&pair), true));
    }
    // Missing/unreadable pair files originally produced a supervisor HarnessError.
    // Preserve that diagnostic; absent evidence must not become a newly inferred result.
    if old["verdict"] != "HarnessError" {
        return Err(format!("cannot recover evidence for {}", case.name).into());
    }
    Ok((
        report::harness_error(
            case,
            old["reason"]
                .as_str()
                .ok_or("missing original reason")?
                .into(),
        ),
        false,
    ))
}

fn scoring_source() -> Result<Value, Box<dyn Error>> {
    let directory = env!("CARGO_MANIFEST_DIR");
    let revision = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(directory)
        .output()?;
    let dirty = Command::new("git")
        .args([
            "status",
            "--porcelain",
            "--",
            "Cargo.toml",
            "src",
            "examples",
        ])
        .current_dir(directory)
        .output()?;
    if !revision.status.success() || !dirty.status.success() {
        return Err("cannot identify scoring source".into());
    }
    Ok(
        json!({"revision":String::from_utf8(revision.stdout)?.trim(),"dirty":!dirty.stdout.is_empty()}),
    )
}

fn replay(run: &Path) -> Result<Value, Box<dyn Error>> {
    let mut scoreboard: Value = serde_json::from_slice(&fs::read(run.join("scoreboard.json"))?)?;
    if scoreboard["status"] != "completed" {
        return Err("re-score requires a completed run".into());
    }
    let previous = scoreboard["results"]
        .as_array()
        .ok_or("missing original results")?;
    if scoreboard["completedCount"] != json!(previous.len())
        || scoreboard["selectedCount"] != json!(previous.len())
    {
        return Err("original result count is incomplete".into());
    }
    let mut records = Vec::new();
    let mut changes = Vec::new();
    let mut replayed = 0;
    let mut retained = 0;
    for old in previous {
        let (record, readable) = classify_saved(run, old)?;
        if readable {
            replayed += 1;
        } else {
            retained += 1;
        }
        // This additive evidence field was absent in the original schema-3 producer.
        let mut normalized = old.clone();
        normalized
            .as_object_mut()
            .ok_or("invalid result record")?
            .entry("cleanupBlurNotes")
            .or_insert_with(|| json!([]));
        if serde_json::to_value(&record)? != normalized {
            changes.push(json!({
                "key":record.case.key,"suite":record.case.file,
                "before":old["verdict"],"beforeReason":old["reason"],
                "after":record.verdict,"afterReason":record.reason,
                "cleanupBlurNotes":record.cleanup_blur_notes,
            }));
        }
        records.push(record);
    }
    let source = scoring_source()?;
    let provenance = json!({
        "scoringRevision":source["revision"],"scoringTreeDirty":source["dirty"],
        "originalRunDirectory":run,"originalScoreboard":run.join("scoreboard.json"),
        "originalSummary":scoreboard["summary"],
        "replayedPairCount":replayed,"retainedUnreadableEvidenceCount":retained,
        "changedCaseCount":changes.len(),
        "reclassifiedCaseCount":changes.iter().filter(|change| change["before"] != change["after"]).count(),
        "changes":changes,"browserRunsAdded":0,
    });
    scoreboard["results"] = serde_json::to_value(&records)?;
    scoreboard["summary"] = serde_json::to_value(report::summarize(&records))?;
    scoreboard["missingMethods"] = serde_json::to_value(report::missing_methods(&records))?;
    scoreboard["rescore"] = provenance;
    Ok(scoreboard)
}

#[expect(
    clippy::print_stdout,
    reason = "The offline CLI reports its output directory and totals."
)]
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.len() != 2 {
        return Err(
            "usage: cargo run -p neo-playwright-tests --example rescore -- RUN NEW_OUTPUT".into(),
        );
    }
    let run = PathBuf::from(&args[0]).canonicalize()?;
    let output = PathBuf::from(&args[1]);
    let scoreboard = replay(&run)?;
    // Exclusive creation prevents overwriting either original evidence or a previous replay.
    fs::create_dir(&output)?;
    for (name, value) in [
        ("scoreboard.json", &scoreboard),
        ("missing-methods.json", &scoreboard["missingMethods"]),
        ("rescore.json", &scoreboard["rescore"]),
    ] {
        fs::write(output.join(name), serde_json::to_vec_pretty(value)?)?;
    }
    println!(
        "{}",
        json!({"output":output,"counts":scoreboard["summary"]["totals"]["counts"],"changed":scoreboard["rescore"]["changedCaseCount"]})
    );
    Ok(())
}
