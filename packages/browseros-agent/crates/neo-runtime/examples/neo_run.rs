//! Runs one script file against one tab of a running BrowserOS and prints the `RunOutcome` as
//! JSON on stdout. Exits 0 only when the run's status is `ok`.
//!
//! ```sh
//! cargo run -p neo-runtime --example neo_run -- --cdp-port 9222 --page <targetId> script.js
//! ```
//!
//! Once connected it writes `connected` on stderr, so a harness driving the tab knows the run
//! is about to start.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;
use std::{fs, io};

use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use clap::Parser;
use neo_runtime::{PageId, RunRequest, RunStatus, Runtime};
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
struct Args {
    /// The browser's remote debugging port.
    #[arg(long)]
    cdp_port: u16,
    /// The CDP target id of the tab to run against.
    #[arg(long)]
    page: String,
    #[arg(long, default_value_t = 30_000)]
    timeout_ms: u64,
    /// A file holding the body of an async function of `(page, context)`.
    script: PathBuf,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    match run(args).await {
        Ok(RunStatus::Ok) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(message) => {
            // Nothing useful to do if stderr itself is gone.
            let _ = writeln!(io::stderr(), "neo_run: {message}");
            ExitCode::from(2)
        }
    }
}

async fn run(args: Args) -> Result<RunStatus, String> {
    let code = fs::read_to_string(&args.script)
        .map_err(|error| format!("reading {}: {error}", args.script.display()))?;
    let options = ConnectOptions {
        // The default exits the process when the browser goes away; a run should report it.
        reconnect_policy: ReconnectPolicy::KeepTrying,
        ..ConnectOptions::new(args.cdp_port)
    };
    let cdp = CdpClient::connect(options)
        .await
        .map_err(|error| format!("connecting to port {}: {error}", args.cdp_port))?;
    let runtime = Runtime::connect(cdp.clone())
        .await
        .map_err(|error| error.to_string())?;
    writeln!(io::stderr(), "connected").map_err(|error| error.to_string())?;

    let outcome = runtime
        .run(RunRequest {
            code,
            page: PageId::new(args.page),
            timeout: Duration::from_millis(args.timeout_ms),
            cancel: CancellationToken::new(),
        })
        .await;
    cdp.disconnect().await;

    let json = serde_json::to_string(&outcome).map_err(|error| error.to_string())?;
    writeln!(io::stdout(), "{json}").map_err(|error| error.to_string())?;
    Ok(outcome.status)
}
