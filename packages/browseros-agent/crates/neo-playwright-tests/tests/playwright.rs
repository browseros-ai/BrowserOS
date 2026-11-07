//! Each original upstream case is one Cargo trial. The opt-in suite records expected Neo gaps.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use libtest_mimic::{Arguments, Completion, Trial};
use neo_playwright_tests::model::{Case, Pair, ResultRecord, Verdict};
use neo_playwright_tests::report;
use serde::de::DeserializeOwned;
use serde_json::json;

const CRATE: &str = env!("CARGO_MANIFEST_DIR");
const WORKER: &str = env!("CARGO_BIN_EXE_neo-playwright-tests-worker");

#[expect(
    clippy::print_stderr,
    reason = "The custom Cargo CLI reports setup failures on stderr."
)]
fn main() {
    if let Err(error) = run() {
        eprintln!("neo-playwright-tests: {error}");
        process::exit(2);
    }
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    fs::write(&temporary, data).map_err(|error| error.to_string())?;
    fs::rename(temporary, path).map_err(|error| error.to_string())
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn execute(case: &Case, root: &Path, browser: &Path) -> ResultRecord {
    let prefix: String = case.name.chars().take(100).collect();
    let suffix = case.name.rsplit("__").next().unwrap_or("invalid");
    let directory = root.join("cases").join(format!("{prefix}-{suffix}"));
    let result = (|| {
        fs::create_dir_all(root.join("cases")).map_err(|error| error.to_string())?;
        fs::create_dir(&directory).map_err(|error| error.to_string())?;
        let settings = directory.join("settings.json");
        write_json(
            &settings,
            &json!({
                "entry": case, "parentPid": process::id(), "overlay": root.join("overlay"), "outputDir": directory,
            "executable": browser, "worker": root.join("native-worker"),
            }),
        )?;
        let stdout = fs::File::create(directory.join("supervisor-stdout.log"))
            .map_err(|error| error.to_string())?;
        let stderr = fs::File::create(directory.join("supervisor-stderr.log"))
            .map_err(|error| error.to_string())?;
        let status = Command::new("node")
            .arg(root.join("harness/js/run-case.mjs"))
            .arg(&settings)
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .status()
            .map_err(|error| error.to_string())?;
        if !status.success() {
            return Err(format!(
                "Case supervisor exited {status}; see {}",
                directory.display()
            ));
        }
        let pair: Pair = read_json(&directory.join("pair.json"))?;
        if pair.entry.key != case.key {
            return Err("Case evidence belongs to another upstream identity".into());
        }
        Ok(report::classify(&pair))
    })();
    let record = result.unwrap_or_else(|error| report::harness_error(case.clone(), error));
    // Case writers never touch the aggregate. Interrupted runs retain their exclusive
    // result files, while the parent alone publishes the final scoreboard atomically.
    if let Err(error) = write_json(&directory.join("verdict.json"), &record) {
        return report::harness_error(case.clone(), error);
    }
    record
}

fn run() -> Result<(), String> {
    let mut arguments = Arguments::from_args();
    if arguments.test_threads.is_none() {
        arguments.test_threads = Some(4);
    }
    let agent = Path::new(CRATE).join("../..");
    let output = env::var_os("NEO_TESTS_OUTPUT").map_or_else(
        || agent.join("target/neo-playwright-tests/runs"),
        PathBuf::from,
    );
    fs::create_dir_all(&output).map_err(|error| error.to_string())?;
    let output = output.canonicalize().map_err(|error| error.to_string())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = Arc::new(output.join(format!("run-{stamp}-{}", process::id())));
    fs::create_dir(root.as_path()).map_err(|error| error.to_string())?;
    let status = Command::new("node")
        .arg(Path::new(CRATE).join("js/collect.mjs"))
        .arg(root.as_path())
        .env("NEO_TESTS_WORKER", WORKER)
        .env(
            "NEO_TESTS_PARENT_EXE",
            env::current_exe().map_err(|error| error.to_string())?,
        )
        .stdin(Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!(
            "Upstream collection failed; see {}",
            root.display()
        ));
    }
    let catalog: Vec<Case> = read_json(&root.join("catalog.json"))?;
    let browser = Arc::new(env::var_os("NEO_E2E_BROWSER").map(PathBuf::from));
    if !arguments.list && browser.as_ref().as_ref().is_none_or(|path| !path.is_file()) {
        return Err("Set NEO_E2E_BROWSER to the BrowserOS neo executable; no compatibility result was recorded".into());
    }
    let results = Arc::new(Mutex::new(Vec::new()));
    let trials: Vec<_> = catalog
        .iter()
        .cloned()
        .map(|case| {
            let root = Arc::clone(&root);
            let browser = Arc::clone(&browser);
            let results = Arc::clone(&results);
            Trial::ignorable_test(case.name.clone(), move || {
                let record = if let Some(browser) = browser.as_ref() {
                    execute(&case, &root, browser)
                } else {
                    report::harness_error(case, "Browser unavailable".into())
                };
                let cli_reason = record.reason.lines().next().unwrap_or(&record.reason);
                let completion = match record.verdict {
                    Verdict::Pass => Ok(Completion::Completed),
                    Verdict::Unsupported | Verdict::Skip | Verdict::NotApplicable => {
                        Ok(Completion::Ignored {
                            reason: Some(format!("{:?}: {cli_reason}", record.verdict)),
                        })
                    }
                    _ => Err(format!("{:?}: {cli_reason}", record.verdict).into()),
                };
                results
                    .lock()
                    .map_err(|_| libtest_mimic::Failed::from("Result collector poisoned"))?
                    .push(record);
                completion
            })
        })
        .collect();
    let selected = trials
        .iter()
        .filter(|trial| !arguments.is_filtered_out(trial) && !arguments.is_ignored(trial))
        .count();
    let conclusion = libtest_mimic::run(&arguments, trials);
    if arguments.list {
        return Ok(());
    }
    let mut results = results
        .lock()
        .map_err(|_| "Result collector poisoned")?
        .clone();
    results.sort_by(|left, right| left.case.name.cmp(&right.case.name));
    publish(&root, &arguments, &results, catalog.len(), selected)?;
    // An empty/all-unsupported slice has no compatibility evidence. Its individual
    // Cargo trials remain honestly ignored, but the suite cannot advertise a green pass.
    if selected == 0 || results.len() != selected || !results.iter().any(|record| record.comparable)
    {
        return Err(
            "No fully comparable case completed; this run establishes no complete compatibility coverage"
                .into(),
        );
    }
    conclusion.exit();
}

#[expect(
    clippy::print_stdout,
    reason = "This custom Cargo CLI prints the requested summary and method ranking."
)]
fn publish(
    root: &Path,
    arguments: &Arguments,
    results: &[ResultRecord],
    catalog_count: usize,
    selected: usize,
) -> Result<(), String> {
    let ranking = report::missing_methods(results);
    let summary = report::summarize(results);
    let complete = selected > 0 && selected == catalog_count && results.len() == selected;
    let scope = if selected == catalog_count {
        "full"
    } else {
        "filtered"
    };
    let status = if results.len() != selected {
        "interrupted"
    } else if selected == 0 {
        "empty"
    } else {
        "completed"
    };
    write_json(
        &root.join("scoreboard.json"),
        &json!({
            "schemaVersion": 2, "catalogCount": catalog_count, "selectedCount": selected,
            "completedCount": results.len(), "fullCorpus": complete,
            "scope": scope, "status": status,
            "filter": arguments.filter, "skip": arguments.skip,
            "metadata": read_json::<serde_json::Value>(&root.join("metadata.json"))?,
            "summary": summary, "missingMethods": ranking, "results": results,
        }),
    )?;
    write_json(&root.join("missing-methods.json"), &ranking)?;
    println!(
        "\nRun {status}; {scope} selection, {}/{selected} completed, {catalog_count} catalog cases",
        results.len()
    );
    print_totals("Overall", &summary.totals);
    if summary.totals.compatibility.denominator == 0 {
        println!("No compatibility measurements; pass/(pass+fail) has an empty denominator.");
    }
    println!("Per-feature results:");
    for (feature, totals) in &summary.features {
        print_totals(feature, totals);
    }
    println!("Per-suite results:");
    for (suite, totals) in &summary.suites {
        print_totals(suite, totals);
    }
    println!("Harness limitations ranked by distinct cases:");
    for (limitation, cases) in &summary.harness_limitations {
        println!("  {:4} {limitation}", cases.len());
    }
    println!("Missing Neo methods ranked by distinct cases:");
    for row in &ranking {
        println!(
            "  {:4} {} ({} complete reference, {} observed Neo, {} partial reference)",
            row.blocked_cases.len(),
            row.method,
            row.reference_path_cases.len(),
            row.observed_neo_cases.len(),
            row.partial_reference_cases.len()
        );
    }
    println!("Scoreboard: {}", root.join("scoreboard.json").display());
    Ok(())
}

#[expect(
    clippy::print_stdout,
    reason = "The Cargo CLI prints explicit denominators and exclusions for each summary group."
)]
fn print_totals(label: &str, totals: &report::Totals) {
    println!(
        "  {label}: pass/(pass+fail) {}/{}, fully comparable/eligible coverage {}/{}",
        totals.compatibility.numerator,
        totals.compatibility.denominator,
        totals.coverage.numerator,
        totals.coverage.denominator
    );
    println!(
        "    Pass {}, Fail {}; excluded: Unsupported {}, Skip {}, NotApplicable {}, ReferenceFailure {}, HarnessError {}",
        totals.counts[&Verdict::Pass],
        totals.counts[&Verdict::Fail],
        totals.counts[&Verdict::Unsupported],
        totals.counts[&Verdict::Skip],
        totals.counts[&Verdict::NotApplicable],
        totals.counts[&Verdict::ReferenceFailure],
        totals.counts[&Verdict::HarnessError]
    );
    println!(
        "    partial evidence {}, partial Fail {} (included in pass+fail), partial reference census {}, harness limitations {}",
        totals.partial_evidence_count,
        totals.partial_fail_count,
        totals.partial_reference_count,
        totals.harness_limitation_count
    );
}
