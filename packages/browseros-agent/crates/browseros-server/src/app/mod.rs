//! `app`: the browser's launch contract and the process. It is the one place the modules meet:
//! `App::assemble` builds every handle from the constructors the other modules export, and
//! nothing else in the crate wires one module to another.
//!
//! ```text
//!   main ── args ──┬─ --version        print the crate version, exit 0, touch nothing
//!                  ├─ --print-openapi  print the generated contract, exit 0
//!                  ├─ __neo-engine     become the browser engine child (browser::engine_main)
//!                  └─ --config <file>  serve: launch file → state dir → logs → store → install id
//!                                      → engine (background) → App::assemble → bind → serve
//!                                      → SIGTERM / POST /system/shutdown → shutdown order
//! ```
//!
//! Exit codes are the browser's: 0 after `POST /system/shutdown`, 2 only when the server port is
//! taken (the browser then relaunches on the next port), 3 after SIGTERM or Ctrl-C (so the
//! browser restarts a server it did not stop itself), 1 for everything else, usage errors
//! included. Frozen after the skeleton: later chunks fill constructors, never this wiring.

mod cli;
mod launch;
mod process;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio_util::sync::CancellationToken;

use crate::agent::{Agent, AgentRuntime};
use crate::browser::Browser;
use crate::conversations::Conversations;
use crate::model::Models;
use crate::store::Store;
use crate::turn::TurnRunner;

pub use launch::{Launch, LaunchError, read_install_id, resolve_state_dir};

/// The handles every handler gets (the house `AppState` pattern). Built once, by `assemble`,
/// from constructors that exist from the skeleton on; later chunks fill the constructors'
/// bodies and never edit this wiring.
#[derive(Clone)]
pub struct App {
    pub conversations: Conversations,
    pub store: Store,
    pub models: Models,
    pub agent: Agent,
    pub browser: Browser,
    pub launch: Arc<Launch>,
    /// `POST /system/shutdown` asks for an exit through this.
    pub shutdown: Shutdown,
}

/// What the command line asked for. `--version`, `--print-openapi`, a usage error and `--help`
/// are distinct outcomes: only the first prints a bare dotted version and exits 0 (the
/// browser's probe reads at most 4096 bytes and must parse it); a usage error exits 1, because
/// the browser reads 2 as "port in use".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    PrintVersion,
    PrintOpenApi,
    Serve(std::path::PathBuf),
    Engine(std::path::PathBuf),
}

/// The parts `App::assemble` is built from: what differs between the real process and a test.
pub struct Parts {
    pub launch: Launch,
    pub store: Store,
    pub models: Models,
    pub browser: Browser,
    /// `None` runs turns through `agent::Agent`; a test of `conversations` passes a scripted
    /// runner instead.
    pub runner: Option<Arc<dyn TurnRunner>>,
    pub shutdown: Shutdown,
}

impl App {
    /// Wires every module from its constructor. Must run inside the tokio runtime, because
    /// `Conversations::new` starts its housekeeping task.
    pub fn assemble(parts: Parts) -> App {
        let launch = Arc::new(parts.launch);
        let agent = Agent::new(
            parts.models.clone(),
            parts.browser.clone(),
            parts.store.clone(),
            AgentRuntime {
                bun: launch.bundled_bun(),
                state_dir: launch.state_dir.join("agents"),
            },
        );
        let runner = parts
            .runner
            .unwrap_or_else(|| Arc::new(agent.clone()) as Arc<dyn TurnRunner>);
        let conversations = Conversations::new(parts.store.clone(), runner, parts.browser.clone());
        App {
            conversations,
            store: parts.store,
            models: parts.models,
            agent,
            browser: parts.browser,
            launch,
            shutdown: parts.shutdown,
        }
    }
}

/// The process's stop signal and why it stopped. Clones share one state.
#[derive(Clone, Default)]
pub struct Shutdown {
    token: CancellationToken,
    exit_requested: Arc<AtomicBool>,
}

impl Shutdown {
    pub fn new() -> Self {
        Self::default()
    }

    /// `POST /system/shutdown`: stop, then exit 0.
    pub fn request_exit(&self) {
        self.exit_requested.store(true, Ordering::SeqCst);
        self.token.cancel();
    }

    /// SIGTERM or Ctrl-C: stop, then exit 3.
    pub fn signal(&self) {
        self.token.cancel();
    }

    /// Fires when either of the above happened.
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    fn exit_requested(&self) -> bool {
        self.exit_requested.load(Ordering::SeqCst)
    }
}

/// Returns an exit code and never calls `exit`, so destructors run and the log is flushed.
pub fn main() -> std::process::ExitCode {
    let action = match cli::parse(std::env::args_os()) {
        Ok(action) => action,
        Err(code) => return code,
    };
    match action {
        Action::PrintVersion => {
            #[expect(
                clippy::print_stdout,
                reason = "the browser parses --version from stdout"
            )]
            {
                println!("{}", crate::VERSION);
            }
            std::process::ExitCode::SUCCESS
        }
        Action::PrintOpenApi => process::print_openapi(),
        Action::Engine(config) => engine(&config),
        Action::Serve(config) => process::serve(&config),
    }
}

/// The engine child's body is `browser`'s; this is only the dispatch. Its exit code is the
/// engine's own and is never the server's: the parent reads it as a failed start.
#[cfg(unix)]
fn engine(config: &std::path::Path) -> std::process::ExitCode {
    crate::browser::engine_main(config)
}

/// There is no engine on Windows; a parent never starts one there.
#[cfg(not(unix))]
fn engine(_config: &std::path::Path) -> std::process::ExitCode {
    std::process::ExitCode::FAILURE
}
