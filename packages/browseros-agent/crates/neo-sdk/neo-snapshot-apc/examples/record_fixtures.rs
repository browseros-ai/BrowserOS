//! Records each fixture's `AnnotatedPageContent` protobuf (`tests/fixtures/<name>.pb`) and its
//! oracle (`tests/fixtures/<name>.oracle.json`: what Playwright's own ARIA engine says about the
//! same page load) from a live Chromium 158.
//!
//! Run from `packages/browseros-agent` against a contract-v4 BrowserOS build
//! (`tests/fixtures/README.md`, "Recording environment"):
//! ```sh
//! NEO_E2E_BROWSER="/Users/shadowfax/code/chromium-1/src/out/Default_browseros_arm64/BrowserOS Dev.app/Contents/MacOS/BrowserOS Dev" \
//!   bun run test:browser -- cargo run -p neo-snapshot-apc --example record_fixtures -- --contract
//! ```
//! Optionally pass fixture names to re-record a subset. `--contract` types into the fields marked
//! `data-typed="contract"` as well; the oracle tests refuse recordings below contract v4.

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
use oracle_capture::OracleEngine;

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
    let mut requested = Vec::new();
    for arg in env::args().skip(1) {
        match arg.as_str() {
            "--contract" => contract = ApcContract::Present,
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
            match recorder.record(name).await {
                Ok(recorded) => println!(
                    "{name}: .pb {} bytes, oracle {} bytes, frames {} (APC {}), {} APC nodes, \
                     {} oracle elements, {} refs, {} canaries typed",
                    recorded.pb_bytes,
                    recorded.oracle_bytes,
                    recorded.oracle_frames,
                    recorded.apc_frames,
                    recorded.apc_nodes,
                    recorded.elements,
                    recorded.refs,
                    recorded.canaries,
                ),
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
            let oracle =
                oracle_capture::read_oracle(client, &loaded, self.engine, &self.browser).await?;
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

        publish_pair(
            &self.fixtures_dir,
            [
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
}

/// Writes both files of one fixture so that the directory never holds a `.pb` beside an oracle
/// of another page load, which would break every backendNodeId join without any test noticing.
/// Each file is written beside its target first, so a failed write leaves the previous pair in
/// place. If the second rename fails after the first replaced its file, that file is removed:
/// a missing file fails tests/oracles.rs, and git still holds the previous pair.
fn publish_pair(dir: &Path, files: [(&str, &Vec<u8>); 2]) -> Result<(), Box<dyn Error>> {
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
                "publishing {} failed ({error}); removed {published:?} so no half pair remains",
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
