//! The `browseros` command line.
//!
//! A second binary of this crate rather than a crate of its own: the CLI and
//! the server are released together, and the CLI needs no dependency the
//! server does not already declare.

use std::process::ExitCode;

use clap::Parser;

use claw_server::cli::{exec, exit, grammar::Cli, io};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    if let Some(lesson) = teach_instead() {
        io::fail(&format!("browseros: {lesson}"));
        return ExitCode::from(code_byte(exit::Code::Usage));
    }

    let parsed = match Cli::try_parse() {
        Ok(parsed) => parsed,
        Err(error) => {
            // clap prints help and version to stdout and errors to stderr, and
            // already knows which is which.
            let _ = error.print();
            let code = if error.use_stderr() {
                exit::Code::Usage
            } else {
                exit::Code::Ok
            };
            return ExitCode::from(code_byte(code));
        }
    };

    match exec::run(parsed).await {
        Ok(code) => ExitCode::from(code_byte(code)),
        Err(error) => {
            io::fail(&format!("browseros: {error}"));
            ExitCode::from(code_byte(exit::code_of(&error)))
        }
    }
}

fn code_byte(code: exit::Code) -> u8 {
    u8::try_from(code.value()).unwrap_or(1)
}

/// Turns "no such command" into the nearest thing neo can actually do.
///
/// An agent that has driven agent-browser will reach for `cookies`, `network`
/// or `state`. Answering with a bare usage error costs it several turns to
/// discover neo has no such tool; answering with the workaround costs one.
///
/// Checked before clap rather than from its error, because clap populates the
/// attempted name only when it has a suggestion to make, and these names have
/// no near neighbour in the grammar by design.
fn teach_instead() -> Option<String> {
    let attempted = std::env::args().nth(1)?;
    if attempted.starts_with('-') {
        return None;
    }
    claw_server::cli::capability::teach(&attempted)
}
