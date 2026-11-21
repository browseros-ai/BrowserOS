//! The engine child's body: neo-server's own `main`, run in the second copy of this executable
//! (`browseros_server __neo-engine --config <file>`). It runs before any of the server's own
//! logging or runtime exists, and follows the engine's exit contract: 2 only for "the agent
//! port is taken", 1 for every other failure. The parent reads these; they are never the
//! server's own exit code.

use std::io::Read;
use std::process::ExitCode;

use neo_server::{App, AppConfig, Logging, StartError};

/// The engine's exit code for "the agent port is in use"; the parent then picks another port.
pub(super) const EXIT_PORT_IN_USE: u8 = 2;

pub(in crate::browser) fn engine_main(config: &std::path::Path) -> ExitCode {
    // The lifeline: the parent holds the only write end of this pipe. EOF arrives when the
    // parent closes it at shutdown, and also when the parent is killed outright, which no
    // signal handler or destructor in the parent would survive. The engine must never outlive
    // the server, so this exits at once, without destructors.
    let lifeline = std::thread::Builder::new()
        .name("neo-engine-lifeline".to_owned())
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
        });
    if let Err(error) = lifeline {
        report(&error);
        return ExitCode::FAILURE;
    }
    let config = match AppConfig::load(config) {
        Ok(config) => config,
        Err(error) => {
            report(&error);
            return ExitCode::FAILURE;
        }
    };
    // Declared before the runtime, so it drops after it and shutdown's last lines are written.
    let _logging = match Logging::install(&config) {
        Ok(logging) => logging,
        Err(error) => {
            report(&format!("installing logging: {error}"));
            return ExitCode::FAILURE;
        }
    };
    // The engine's own runtime: its QuickJS call chain needs 8 MiB worker stacks.
    let runtime = match App::runtime() {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(%error, "building the engine's runtime");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(async { App::start(config).await?.serve().await }) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error @ StartError::PortInUse(_)) => {
            tracing::error!(%error, "the engine's port is taken");
            ExitCode::from(EXIT_PORT_IN_USE)
        }
        Err(error) => {
            tracing::error!(%error, "the engine stopped");
            ExitCode::FAILURE
        }
    }
}

/// Before the engine's log opens, stderr (the server's, inherited) is the only place to say why.
#[expect(clippy::print_stderr, reason = "the engine's log is not open yet")]
fn report(error: &dyn std::fmt::Display) {
    eprintln!("browseros_server __neo-engine: {error}");
}
