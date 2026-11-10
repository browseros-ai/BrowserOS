//! The `browseros-claw-sdk` binary: the process contract Chromium relies on.
//!
//! - `--version` prints the dotted version alone on stdout and exits 0, before any config,
//!   logging or state directory. Chromium's probe reads at most 4096 bytes of stdout within 10 s.
//! - `--config <path>` serves until SIGTERM, then exits 0. Chromium sends SIGKILL 5 s later.
//! - Exit 2 means only "the agent port is in use", and Chromium relaunches Neo on the next port.
//!   Every other failure exits 1, argument errors included, because clap's own usage exit is 2.
//!
//! `main` returns an `ExitCode` and never calls `process::exit`, so destructors run and the log
//! writer flushes its buffer before the process ends.

use std::fmt::Display;
use std::hint;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use clap::error::ErrorKind;
use neo_server::{App, AppConfig, Logging, StartError};
use tracing::error;

/// The exit code Chromium reads as "port in use, relaunch on the next one". Nothing else may
/// return it.
const EXIT_PORT_IN_USE: u8 = 2;

/// The packaging marker. The release CLI and the image's `neo-server-info` search the binary for
/// it to tell claw-sdk from claw-server, because Chromium launches both under one file name, and
/// they refuse a build without it. The format (`name=version;`) matches claw-server's
/// `browseros-claw-server-version=` marker.
const VERSION_MARKER: &str = concat!(
    "browseros-claw-sdk-version=",
    env!("CARGO_PKG_VERSION"),
    ";"
);

/// Chromium's arguments. Parsed with `try_parse`, never `parse`, so a bad flag exits 1.
#[derive(Debug, Parser)]
#[command(name = "browseros-claw-sdk", disable_version_flag = true)]
struct Cli {
    /// Chromium's launch JSON.
    #[arg(long, required_unless_present = "version", conflicts_with = "version")]
    config: Option<PathBuf>,
    /// Print the version alone on stdout and exit.
    #[arg(long)]
    version: bool,
}

fn main() -> ExitCode {
    // Keeps the marker's bytes in the binary through LTO and stripping.
    hint::black_box(VERSION_MARKER);
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            // clap writes help to stdout and usage errors to stderr.
            let _ = error.print();
            return match error.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => ExitCode::SUCCESS,
                _ => ExitCode::FAILURE,
            };
        }
    };
    if cli.version {
        #[expect(clippy::print_stdout, reason = "Chromium parses --version from stdout")]
        {
            println!("{}", env!("CARGO_PKG_VERSION"));
        }
        return ExitCode::SUCCESS;
    }
    let Some(config_path) = cli.config else {
        // clap's `required_unless_present` already refused this.
        return ExitCode::FAILURE;
    };

    let config = match AppConfig::load(&config_path) {
        Ok(config) => config,
        Err(error) => {
            report_before_logging(&error);
            return ExitCode::FAILURE;
        }
    };
    // Declared before the runtime, so it drops after it: the last log lines of shutdown still
    // reach the file.
    let _logging = match Logging::install(&config) {
        Ok(logging) => logging,
        Err(error) => {
            report_before_logging(&format!("installing logging: {error}"));
            return ExitCode::FAILURE;
        }
    };
    let runtime = match App::runtime() {
        Ok(runtime) => runtime,
        Err(error) => {
            error!(%error, "building the tokio runtime");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(async { App::start(config).await?.serve().await }) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error @ StartError::PortInUse(_)) => {
            error!(%error, "another process holds the agent port; Chromium relaunches on the next");
            ExitCode::from(EXIT_PORT_IN_USE)
        }
        Err(error) => {
            error!(%error, "claw-sdk stopped");
            ExitCode::FAILURE
        }
    }
}

/// Before the log file opens, stderr is the only place a failure can go.
#[expect(clippy::print_stderr, reason = "the log file is not open yet")]
fn report_before_logging(error: &dyn Display) {
    eprintln!("browseros-claw-sdk: {error}");
}
