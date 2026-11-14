//! The SDK personality of the host executable. It runs before the host's
//! runtime/logger exists and follows the SDK binary's start and exit contract.

use std::{fmt::Display, io::Read};

use neo_server::{App, AppConfig, Logging, StartError};

pub(super) fn run() -> i32 {
    let mut args = std::env::args_os().skip(2);
    let config_path = match (args.next(), args.next(), args.next()) {
        (Some(flag), Some(path), None) if flag == "--config" => path,
        _ => {
            report("expected __neo-sdk --config <path>");
            return 1;
        }
    };
    // The parent owns the only write end of this pipe. Unlike a signal or a
    // Drop guard, EOF arrives even after the parent receives SIGKILL. No SDK
    // work may outlive it, so this path deliberately bypasses destructors.
    if let Err(error) = std::thread::Builder::new()
        .name("neo-parent-lifeline".to_owned())
        .spawn(|| {
            let mut byte = [0_u8; 1];
            loop {
                match std::io::stdin().read(&mut byte) {
                    Ok(0) => std::process::exit(0),
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => std::process::exit(1),
                }
            }
        })
    {
        report(&error);
        return 1;
    }
    let config = match AppConfig::load(std::path::Path::new(&config_path)) {
        Ok(config) => config,
        Err(error) => {
            report(&error);
            return 1;
        }
    };
    // Keep the log guard alive until after the runtime drains on normal exit.
    let _logging = match Logging::install(&config) {
        Ok(logging) => logging,
        Err(error) => {
            report(&error);
            return 1;
        }
    };
    let runtime = match App::runtime() {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(%error, "the Playwright runtime could not start");
            return 1;
        }
    };
    match runtime.block_on(async { App::start(config).await?.serve().await }) {
        Ok(()) => 0,
        Err(error @ StartError::PortInUse(_)) => {
            tracing::error!(%error, "the Playwright engine port is already in use");
            2
        }
        Err(error) => {
            tracing::error!(%error, "the Playwright engine stopped");
            1
        }
    }
}

#[expect(
    clippy::print_stderr,
    reason = "startup errors occur before SDK logging exists"
)]
fn report(error: impl Display) {
    eprintln!("Playwright engine: {error}");
}
