//! Runner N of conformance tier 1 (design D15): every case in `tests/conformance/cases/` run
//! by the real prelude behind claw-sdk's real bootstrap in QuickJS (`support/js_runner.rs`),
//! with every op going through `Playwright::invoke`, as claw-sdk's `run` tool will run an
//! agent's script. Runner P (`tests/conformance/runner-p.mjs`) runs the same files on real
//! Playwright, and `tests/conformance/diff.mjs` compares the two; this test only records Neo's
//! outcomes.
//!
//! Each case gets a fresh QuickJS runtime and a fresh `Playwright` over its own CDP
//! connection, so no run state, listener or ref map carries from one case to the next. The run
//! starts with no tab, as a new task does, so the door opens one for its first op, and every
//! tab the prelude knows of is closed when the case ends.
//!
//! `tests/conformance/run.sh tier1` runs it with everything it needs. By hand, it needs a
//! throwaway BrowserOS on `BROWSEROS_CDP_PORT`, `tests/conformance/server.mjs` on
//! `CONFORMANCE_PORT`, and `CONFORMANCE_OUT`, the directory the outcomes go to;
//! `CONFORMANCE_CASES` (names, space-separated) runs only those cases, and
//! `CONFORMANCE_CASES_DIR` (relative to `tests/conformance/`, `cases` by default) picks a tier's
//! directory, such as tier 2's `cases/tier2`:
//! `cargo test -p neo-playwright --test conformance_live -- --ignored`.

#[path = "support/js_runner.rs"]
mod js_runner;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "Runner N drives pages through scripts, not through live_run's direct helpers"
)]
mod live_run;

use std::path::{self, Path};
use std::sync::Arc;
use std::time::Duration;
use std::{env, fs};

use js_runner::{OpHost, run_script};
use live_run::{LiveRun, TestResult};
use serde_json::{Value, json};
use tokio::time;

/// The case contract both runners evaluate (`harness.js` documents it).
const HARNESS: &str = include_str!("conformance/harness.js");

/// Runs one case after `HARNESS`, with `source` and `port` bound before it. Neo's side of the
/// contract: `harness.pageEval` over the `__neo_test_eval` native, and the tabs the prelude
/// opened closed at the end.
///
/// `harness.js` compiles a case with the `AsyncFunction` constructor, whose code sees globals
/// only, while claw-sdk hands an agent's code the bootstrap's `console` and timers, and the
/// prelude's `expect` and `Buffer`, as its function's parameters. They become globals here for
/// the case, as Node's own and Playwright Test's `expect` are for Runner P.
const RUN_CASE: &str = r"
const skip = __conformance.needs(source)
if (skip !== null) return { skipped: skip }
Object.assign(globalThis, { console, setTimeout, clearTimeout, expect, Buffer })
const pageEval = async (target, expression) => {
  if (target?._id == null) throw new Error('harness.pageEval: the page is not open yet')
  const reply = JSON.parse(await __neo_test_eval(target._id, target._tab, String(expression)))
  if ('err' in reply) throw new Error(reply.err)
  return reply.ok
}
try {
  return await __conformance.run(source, { page, context, port, pageEval })
} finally {
  for (const opened of context.pages()) await opened.close().catch(() => {})
}
";

/// As long as Runner P gives a case; a case that runs past it is broken, not slow.
const CASE_BUDGET: Duration = Duration::from_secs(90);

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT and server.mjs on CONFORMANCE_PORT"]
async fn tests_that_every_conformance_case_runs_through_the_prelude() -> TestResult {
    let setting = |name: &str, what: &str| {
        env::var(name).map_err(|error| format!("set {name} to {what} ({error}), or use run.sh"))
    };
    let port: u16 = setting("CONFORMANCE_PORT", "server.mjs's port")?.parse()?;
    let out = path::absolute(setting("CONFORMANCE_OUT", "the outcomes' directory")?)?;
    let only: Vec<String> = env::var("CONFORMANCE_CASES")
        .map(|names| names.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default();
    fs::create_dir_all(&out)?;
    let tier = env::var("CONFORMANCE_CASES_DIR").unwrap_or_else(|_| "cases".to_owned());
    let conformance = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/conformance");
    let dir = conformance.join(tier);
    // Runner P works in the conformance directory too, so a case's relative paths
    // (`files/one.txt`) name the same files for both, as a script's name files in Neo's.
    env::set_current_dir(&conformance)?;
    for (name, source) in cases(&dir, &only)? {
        let outcome = run_case(port, &source).await;
        let kind = outcome
            .as_object()
            .and_then(|outcome| outcome.keys().next().cloned())
            .unwrap_or_default();
        #[expect(
            clippy::print_stdout,
            reason = "run.sh shows each case's outcome as it lands, as runner-p.mjs does"
        )]
        {
            println!("runner-n: {name}: {kind}");
        }
        fs::write(
            out.join(format!("{name}.json")),
            format!("{}\n", serde_json::to_string_pretty(&outcome)?),
        )?;
    }
    Ok(())
}

/// Every case file's name and source, sorted by name, or only the named ones.
fn cases(dir: &Path, only: &[String]) -> TestResult<Vec<(String, String)>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("js") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or("a case file is named in UTF-8")?
            .to_owned();
        if only.is_empty() || only.contains(&name) {
            found.push((name, fs::read_to_string(&path)?));
        }
    }
    found.sort();
    Ok(found)
}

/// One case's outcome: what `harness.js` shaped, or `{ "broke": .. }` when the runner failed.
async fn run_case(port: u16, source: &str) -> Value {
    let run = match LiveRun::connect().await {
        Ok(run) => run,
        Err(error) => return json!({ "broke": format!("connect: {error}") }),
    };
    let host: Arc<dyn OpHost> = Arc::new(run);
    let init = json!({ "page": null });
    let script = format!(
        "{HARNESS}\nconst source = {}\nconst port = {port}\n{RUN_CASE}",
        Value::from(source)
    );
    match time::timeout(CASE_BUDGET, run_script(&host, &init, &script)).await {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(error)) => json!({ "broke": error }),
        Err(_) => json!({ "broke": format!("the case ran past {CASE_BUDGET:?}") }),
    }
}
