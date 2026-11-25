//! Records real `AnnotatedPageContent` protobuf fixtures from a live Chromium 158 browser.
//!
//! Run from `packages/browseros-agent`:
//! ```sh
//! NEO_E2E_BROWSER="/Users/shadowfax/code/chromium-3/src/out/Default_browseros_arm64/BrowserOS Dev.app/Contents/MacOS/BrowserOS Dev" \
//!   bun run test:browser -- cargo run -p neo-snapshot-apc --example record_fixtures
//! ```
//! Optionally pass one or more fixture names to re-record a subset.

#[path = "../tests/support/fixture_pages.rs"]
#[expect(
    dead_code,
    reason = "capture_fixture_raw_json serves tests/live_capture.rs"
)]
mod fixture_pages;
#[path = "../tests/support/fixture_server.rs"]
mod fixture_server;

use std::error::Error;
use std::path::PathBuf;
use std::{env, fs};

use fixture_pages::{
    ALL_FIXTURES, all_fixture_pages, capture_fixture_typed, connect_cdp, count_frames_and_nodes,
};
use fixture_server::FixtureServer;

#[expect(
    clippy::print_stdout,
    reason = "fixture recorder CLI prints captured fixture summaries"
)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let requested: Vec<String> = env::args().skip(1).collect();
    let names: Vec<&str> = if requested.is_empty() {
        ALL_FIXTURES.to_vec()
    } else {
        requested.iter().map(String::as_str).collect()
    };

    let fixtures_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    fs::create_dir_all(&fixtures_dir)?;

    let server = FixtureServer::start(all_fixture_pages()).await?;
    let client = connect_cdp().await?;

    let result = async {
        for name in names {
            let (raw_bytes, apc) = capture_fixture_typed(&client, &server, name).await?;
            let (frames, nodes) = count_frames_and_nodes(&apc);
            let out_path = fixtures_dir.join(format!("{name}.pb"));
            fs::write(&out_path, &raw_bytes)?;
            println!(
                "{name}: {} bytes, {frames} frames, {nodes} nodes -> {}",
                raw_bytes.len(),
                out_path.display()
            );
        }
        Ok::<(), Box<dyn Error>>(())
    }
    .await;

    client.disconnect().await;
    result
}
