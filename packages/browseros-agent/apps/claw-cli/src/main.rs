use std::process::ExitCode;

use clap::Parser;

use claw_cli::{cli::Cli, exec, exit, io};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
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
