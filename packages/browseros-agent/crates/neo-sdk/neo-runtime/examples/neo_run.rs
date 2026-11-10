//! Runs one script file against one tab of a running BrowserOS and prints the `RunOutcome` as
//! JSON on stdout. Exits 0 only when the run's status is `ok`.
//!
//! ```sh
//! cargo run -p neo-runtime --example neo_run -- --cdp-port 9222 --page <targetId> script.js
//! cargo run -p neo-runtime --example neo_run -- --cdp-port 9222 --page <targetId> \
//!   --artifacts /tmp/run-artifacts script.js   # page.screenshot({ path: 'a.png' }) writes here
//! cargo run -p neo-runtime --example neo_run -- --cdp-port 9222 --page <targetId> \
//!   --artifacts /tmp/run-artifacts --redirect-absolute-paths script.js   # as the MCP host runs
//! ```
//!
//! Without `--page`, the run opens a background tab of its own and starts there.
//!
//! Once connected it writes `connected` on stderr, so a harness driving the tab knows the run
//! is about to start.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;
use std::{fs, io};

#[expect(
    clippy::disallowed_types,
    reason = "the example host imports the client it connects and hands into Runtime's typed CDP door"
)]
use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use clap::Parser;
use neo_runtime::{AbsolutePaths, PageId, RunRequest, RunStatus, Runtime, StartPage};
use tokio::time;
use tokio_util::sync::CancellationToken;

/// The longest `neo_run` waits for a run's cleanup before disconnecting: two CDP requests at
/// browseros-cdp's 60 s request timeout, the most one cleanup task makes in a row.
const DRAIN_TIMEOUT: Duration = Duration::from_mins(2);

#[derive(Parser)]
struct Args {
    /// The browser's remote debugging port.
    #[arg(long)]
    cdp_port: u16,
    /// The CDP target id of the tab to run against. Omitted, the run opens a new background
    /// tab.
    #[arg(long)]
    page: Option<String>,
    #[arg(long, default_value_t = 30_000)]
    timeout_ms: u64,
    /// The run's artifact directory: where `path` options (`page.screenshot({ path })`,
    /// `page.pdf({ path })`) write, relative to it. Without it they are refused.
    #[arg(long)]
    artifacts: Option<PathBuf>,
    /// Take absolute `path` options as the MCP host does (`AbsolutePaths::Redirect`): inside the
    /// artifact directory as that file, outside it at `<artifacts>/<file name>`.
    #[arg(long)]
    redirect_absolute_paths: bool,
    /// A file of Playwright code: a function of `(page, context, playwright)`, one expression, or
    /// the body of an async function of `(page, context, playwright)`.
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

#[expect(
    clippy::disallowed_types,
    reason = "the example host connects a client and passes it into Runtime's typed CDP door"
)]
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
        .map_err(|error| error.to_string())?
        .with_absolute_paths(if args.redirect_absolute_paths {
            AbsolutePaths::Redirect
        } else {
            AbsolutePaths::Refuse
        });
    writeln!(io::stderr(), "connected").map_err(|error| error.to_string())?;

    let outcome = runtime
        .run(RunRequest {
            code,
            page: args
                .page
                .map_or(StartPage::NewTab, |page| StartPage::Page(PageId::new(page))),
            timeout: Duration::from_millis(args.timeout_ms),
            cancel: CancellationToken::new(),
            artifacts: args.artifacts,
        })
        .await;
    // A run's outcome does not settle its cleanup (a screenshot's late restore, a PDF stream's
    // close): drain it before going away, as `Runtime::drain` asks every host to.
    if time::timeout(DRAIN_TIMEOUT, runtime.drain()).await.is_err() {
        let _ = writeln!(
            io::stderr(),
            "neo_run: cleanup still pending after {DRAIN_TIMEOUT:?}"
        );
    }
    cdp.disconnect().await;

    let json = serde_json::to_string(&outcome).map_err(|error| error.to_string())?;
    writeln!(io::stdout(), "{json}").map_err(|error| error.to_string())?;
    Ok(outcome.status)
}
