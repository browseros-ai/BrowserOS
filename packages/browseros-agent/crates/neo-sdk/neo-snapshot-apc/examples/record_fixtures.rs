//! Records each fixture's `AnnotatedPageContent` protobuf (`tests/fixtures/<name>.pb`) and its
//! oracle (`tests/fixtures/<name>.oracle.json`: what Playwright's own ARIA engine says about the
//! same page load) from a live Chromium 158.
//!
//! Run from `packages/browseros-agent` against a contract-v6 BrowserOS build
//! (`tests/fixtures/README.md`, "Recording environment"):
//! ```sh
//! NEO_E2E_BROWSER="/Users/shadowfax/code/chromium-1/src/out/Default_browseros_arm64/BrowserOS Dev.app/Contents/MacOS/BrowserOS Dev" \
//!   bun run test:browser -- cargo run -p neo-snapshot-apc --example record_fixtures -- --contract
//! ```
//! Optionally pass fixture names to re-record a subset. `--contract` types into the fields marked
//! `data-typed="contract"` as well; the oracle tests refuse recordings below contract v4.
//!
//! `--option-places` re-reads only the oracle, to add each listed `<option>`'s `option` place
//! (its select and index) to an oracle recorded before the reader wrote one. It loads and types
//! as a recording does, reads the oracle and closes the tab; it runs no before-capture step and
//! no APC, and never writes a `.pb`. The fresh read must equal the committed oracle in every
//! value but the engine banner, the CDP frame ids and the `option` fields, which is what lets it
//! run on a browser without the APC patches (the oracle is Playwright's engine reading the DOM)
//! and leave a `.pb` recorded on an older contract in place. The written file is the committed
//! oracle plus the places.

#[path = "../tests/support/fixture_pages.rs"]
#[expect(
    dead_code,
    reason = "the capture_fixture_* helpers serve tests/live_capture.rs"
)]
mod fixture_pages;
#[path = "../tests/support/fixture_server.rs"]
mod fixture_server;
#[path = "../tests/support/oracle.rs"]
mod oracle;
#[path = "../tests/support/oracle_capture.rs"]
mod oracle_capture;
#[path = "../tests/support/tree_counts.rs"]
mod tree_counts;

use std::error::Error;
use std::path::{Path, PathBuf};
use std::{env, fs};

use browseros_cdp::CdpClient;
use fixture_pages::{ApcContract, CANARY_PREFIX, FIXTURES};
use fixture_server::FixtureServer;
use oracle::Oracle;
use oracle_capture::OracleEngine;
use serde::Deserialize;
use serde_json::Value;

/// What a run does to each requested fixture.
#[derive(Clone, Copy)]
enum Mode {
    /// Records the `.pb` and the oracle from one page load.
    Record,
    /// Merges a fresh oracle read's option places into the committed oracle (`--option-places`).
    OptionPlaces,
}

/// What one fixture's recording wrote, for its summary line.
struct Recorded {
    pb_bytes: usize,
    oracle_bytes: usize,
    oracle_frames: usize,
    apc_frames: usize,
    apc_nodes: usize,
    elements: usize,
    refs: usize,
    canaries: usize,
}

#[expect(
    clippy::print_stdout,
    reason = "fixture recorder CLI prints one summary line per fixture"
)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut contract = ApcContract::Absent;
    let mut mode = Mode::Record;
    let mut requested = Vec::new();
    for arg in env::args().skip(1) {
        match arg.as_str() {
            "--contract" => contract = ApcContract::Present,
            "--option-places" => mode = Mode::OptionPlaces,
            flag if flag.starts_with("--") => return Err(format!("unknown flag {flag}").into()),
            _ => requested.push(arg),
        }
    }
    let names: Vec<&str> = if requested.is_empty() {
        FIXTURES.iter().map(|fixture| fixture.name).collect()
    } else {
        requested.iter().map(String::as_str).collect()
    };

    let fixtures_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    fs::create_dir_all(&fixtures_dir)?;
    let engine = OracleEngine::new()?;

    let server = FixtureServer::start(fixture_pages::all_fixture_pages()).await?;
    let client = fixture_pages::connect_cdp().await?;

    let result = async {
        let browser = oracle_capture::browser_product(&client).await?;
        println!("browser: {browser}; engine: {}", engine.banner());
        let recorder = Recorder {
            client: &client,
            server: &server,
            engine: &engine,
            browser,
            fixtures_dir,
            contract,
        };
        let mut failed = Vec::new();
        for name in names {
            let summary = match mode {
                Mode::Record => recorder.record(name).await.map(|recorded| {
                    format!(
                        ".pb {} bytes, oracle {} bytes, frames {} (APC {}), {} APC nodes, \
                         {} oracle elements, {} refs, {} canaries typed",
                        recorded.pb_bytes,
                        recorded.oracle_bytes,
                        recorded.oracle_frames,
                        recorded.apc_frames,
                        recorded.apc_nodes,
                        recorded.elements,
                        recorded.refs,
                        recorded.canaries,
                    )
                }),
                Mode::OptionPlaces => recorder.merge_option_places(name).await.map(|merged| {
                    let equal = "every other value equals the committed oracle";
                    match merged {
                        Merged::Unchanged { places } => {
                            format!("unchanged, nothing written: {places} option places, {equal}")
                        }
                        Merged::Written {
                            places,
                            oracle_bytes,
                            canaries,
                        } => format!(
                            "wrote {places} option places, oracle {oracle_bytes} bytes, \
                             {canaries} canaries typed; {equal}; .pb untouched"
                        ),
                    }
                }),
            };
            match summary {
                Ok(summary) => println!("{name}: {summary}"),
                Err(error) => {
                    println!("{name}: FAILED, nothing written: {error}");
                    failed.push(name);
                }
            }
        }
        if failed.is_empty() {
            Ok(())
        } else {
            Err(format!("{} fixture(s) failed: {}", failed.len(), failed.join(", ")).into())
        }
    }
    .await;

    client.disconnect().await;
    result
}

/// One recording run: the browser, the fixture server serving every page, and where the files go.
struct Recorder<'a> {
    client: &'a CdpClient,
    server: &'a FixtureServer,
    engine: &'a OracleEngine,
    browser: String,
    fixtures_dir: PathBuf,
    contract: ApcContract,
}

impl Recorder<'_> {
    /// Records `name`'s `.pb` and oracle from one page load: load and type, oracle, the fixture's
    /// before-capture step, APC. Either both files are written or neither.
    async fn record(&self, name: &str) -> Result<Recorded, Box<dyn Error>> {
        let client = self.client;
        let fixture = fixture_pages::fixture(name)?;
        let loaded =
            fixture_pages::load_fixture(client, self.server, fixture, self.contract).await?;
        let typed = loaded.typed.clone();
        let read = async {
            let oracle = oracle_capture::read_oracle(
                client,
                fixture.name,
                &loaded.session,
                &loaded.target_id,
                self.engine,
                &self.browser,
            )
            .await?;
            loaded.before_capture(client).await?;
            let (pb, apc) = loaded.capture_apc(client).await?;
            Ok::<_, Box<dyn Error>>((oracle, pb, apc))
        }
        .await;
        loaded.close(client).await;
        let (oracle, pb, apc) = read?;

        let mut oracle_json = serde_json::to_vec_pretty(&oracle)?;
        oracle_json.push(b'\n');
        let pb_name = format!("{name}.pb");
        let oracle_name = format!("{name}.oracle.json");

        // The canary guard: a typed secret in either file means Blink (or the oracle's masking)
        // did not withhold it. Neither file is written, so a leak can never be committed.
        for (file, bytes) in [(&pb_name, &pb), (&oracle_name, &oracle_json)] {
            if let Some(found) = leaked_canary(bytes, &typed) {
                return Err(format!(
                    "{name}: {file} contains the typed secret {found:?}: Blink did not withhold it"
                )
                .into());
            }
        }

        publish(
            &self.fixtures_dir,
            &[
                (pb_name.as_str(), &pb),
                (oracle_name.as_str(), &oracle_json),
            ],
        )?;
        let counts = tree_counts::count_frames_and_nodes(&apc);
        Ok(Recorded {
            pb_bytes: pb.len(),
            oracle_bytes: oracle_json.len(),
            oracle_frames: oracle.frames.len(),
            apc_frames: counts.frames,
            apc_nodes: counts.nodes,
            elements: oracle.frames.iter().map(|frame| frame.elements.len()).sum(),
            refs: oracle.frames.iter().map(|frame| frame.refs.len()).sum(),
            canaries: typed.len(),
        })
    }

    /// Re-reads `name`'s oracle on a fresh load and, if the read equals the committed oracle in
    /// everything but the banner, the frame ids and the option places, writes the committed
    /// oracle with the read's places. Writes nothing when any other value differs.
    async fn merge_option_places(&self, name: &str) -> Result<Merged, Box<dyn Error>> {
        let client = self.client;
        let fixture = fixture_pages::fixture(name)?;
        let oracle_name = format!("{name}.oracle.json");
        let committed = read_committed(&self.fixtures_dir.join(&oracle_name))?;
        let loaded =
            fixture_pages::load_fixture(client, self.server, fixture, self.contract).await?;
        let typed = loaded.typed.clone();
        let fresh = oracle_capture::read_oracle(
            client,
            fixture.name,
            &loaded.session,
            &loaded.target_id,
            self.engine,
            &self.browser,
        )
        .await;
        loaded.close(client).await;

        let merged = with_option_places(&committed, fresh?)
            .map_err(|path| format!("the fresh oracle differs from {oracle_name} at {path}"))?;
        let places = merged
            .frames
            .iter()
            .flat_map(|frame| &frame.elements)
            .filter(|element| element.option.is_some())
            .count();
        if merged == committed {
            return Ok(Merged::Unchanged { places });
        }
        let mut oracle_json = serde_json::to_vec_pretty(&merged)?;
        oracle_json.push(b'\n');
        if let Some(found) = leaked_canary(&oracle_json, &typed) {
            return Err(format!(
                "{name}: {oracle_name} contains the typed secret {found:?}: the oracle's masking \
                 did not withhold it"
            )
            .into());
        }
        publish(&self.fixtures_dir, &[(oracle_name.as_str(), &oracle_json)])?;
        Ok(Merged::Written {
            places,
            oracle_bytes: oracle_json.len(),
            canaries: typed.len(),
        })
    }
}

/// What `--option-places` did to one fixture's oracle, for its summary line.
enum Merged {
    /// The committed oracle already holds every place the fresh read found (none, on a page
    /// without a select).
    Unchanged { places: usize },
    /// The committed oracle was rewritten with the read's places.
    Written {
        places: usize,
        oracle_bytes: usize,
        canaries: usize,
    },
}

/// Reads the oracle a fixture has committed.
fn read_committed(path: &Path) -> Result<Oracle, Box<dyn Error>> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    // An ARIA tree nests two JSON levels per element; deep-nesting's passes serde's default 128.
    let mut reader = serde_json::Deserializer::from_str(&text);
    reader.disable_recursion_limit();
    let oracle =
        Oracle::deserialize(&mut reader).map_err(|error| format!("{}: {error}", path.display()))?;
    reader.end()?;
    Ok(oracle)
}

/// `committed` with each element's `option` taken from `fresh`, or the JSON path of the first
/// value where `fresh` differs from `committed` beyond what a re-read may change: the engine
/// banner, a frame's CDP `frame` and `parent` ids (fresh on every load), and an `option` the
/// committed oracle lacks. Every backendNodeId, owner and ref must be the committed one, so the
/// result still joins the committed `.pb`.
fn with_option_places(committed: &Oracle, fresh: Oracle) -> Result<Oracle, String> {
    let mut merged = fresh;
    merged.engine.clone_from(&committed.engine);
    if merged.frames.len() != committed.frames.len() {
        return Err(format!(
            "$.frames: {} frames, committed {}",
            merged.frames.len(),
            committed.frames.len()
        ));
    }
    // Frames are in the same order on both reads: siblings sort by URL first, and no fixture has
    // two sibling frames at one URL, so the order never rests on the fresh frame ids.
    for (frame, old) in merged.frames.iter_mut().zip(&committed.frames) {
        frame.frame.clone_from(&old.frame);
        frame.parent.clone_from(&old.parent);
    }
    let mut compared = merged.clone();
    for (frame, old) in compared.frames.iter_mut().zip(&committed.frames) {
        for (element, old) in frame.elements.iter_mut().zip(&old.elements) {
            if old.option.is_none() {
                element.option = None;
            }
        }
    }
    let (committed, compared) = (
        serde_json::to_value(committed).map_err(|error| error.to_string())?,
        serde_json::to_value(&compared).map_err(|error| error.to_string())?,
    );
    match first_difference(&committed, &compared, "$") {
        Some(path) => Err(path),
        None => Ok(merged),
    }
}

/// The path of the first value where `fresh` differs from `committed`, with both values.
fn first_difference(committed: &Value, fresh: &Value, path: &str) -> Option<String> {
    match (committed, fresh) {
        (Value::Object(old), Value::Object(new)) => old
            .keys()
            .chain(new.keys().filter(|key| !old.contains_key(*key)))
            .find_map(|key| match (old.get(key), new.get(key)) {
                (Some(old), Some(new)) => first_difference(old, new, &format!("{path}.{key}")),
                (old, new) => Some(format!("{path}.{key}: committed {old:?}, fresh {new:?}")),
            }),
        (Value::Array(old), Value::Array(new)) => old
            .iter()
            .zip(new)
            .enumerate()
            .find_map(|(index, (old, new))| first_difference(old, new, &format!("{path}[{index}]")))
            .or_else(|| {
                (old.len() != new.len())
                    .then(|| format!("{path}: {} entries, committed {}", new.len(), old.len()))
            }),
        (old, new) => (old != new).then(|| format!("{path}: committed {old}, fresh {new}")),
    }
}

/// Writes a fixture's files together so that the directory never holds a `.pb` beside an oracle
/// of another page load, which would break every backendNodeId join without any test noticing.
/// A recording writes both files of its one load. `--option-places` writes the oracle alone
/// from another load, and keeps the joins by writing only once that load's oracle has proved
/// equal to the committed one in every backendNodeId, owner and ref.
/// Each file is written beside its target first, so a failed write leaves the previous files in
/// place. If a later rename fails after an earlier one replaced its file, that file is removed:
/// a missing file fails tests/oracles.rs, and git still holds the previous pair.
fn publish(dir: &Path, files: &[(&str, &Vec<u8>)]) -> Result<(), Box<dyn Error>> {
    let staged: Vec<(PathBuf, PathBuf)> = files
        .iter()
        .map(|(name, _)| (dir.join(format!("{name}.partial")), dir.join(name)))
        .collect();
    let written = files
        .iter()
        .zip(&staged)
        .try_for_each(|((_, bytes), (partial, _))| fs::write(partial, bytes));
    if let Err(error) = written {
        for (partial, _) in &staged {
            let _removed = fs::remove_file(partial);
        }
        return Err(error.into());
    }
    let mut published: Vec<&PathBuf> = Vec::new();
    for (partial, target) in &staged {
        if let Err(error) = fs::rename(partial, target) {
            for (partial, _) in &staged {
                let _removed = fs::remove_file(partial);
            }
            for target in &published {
                let _removed = fs::remove_file(target);
            }
            return Err(format!(
                "publishing {} failed ({error}); removed {published:?} so no fixture is left half written",
                target.display()
            )
            .into());
        }
        published.push(target);
    }
    Ok(())
}

/// The first of [`CANARY_PREFIX`] and each typed canary found in `bytes`. The prefix alone
/// also catches a secret cut short.
fn leaked_canary<'a>(bytes: &[u8], typed: &'a [String]) -> Option<&'a str> {
    let contains = |needle: &str| {
        bytes
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
    };
    if contains(CANARY_PREFIX) {
        return Some(CANARY_PREFIX);
    }
    typed
        .iter()
        .map(String::as_str)
        .find(|canary| contains(canary))
}
