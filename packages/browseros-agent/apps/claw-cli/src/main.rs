use std::process::ExitCode;

use claw_cli::{exit, io};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => ExitCode::from(u8::try_from(code.value()).unwrap_or(1)),
        Err(error) => {
            io::fail(&format!("browseros: {error}"));
            let code = exit::code_of(&error);
            ExitCode::from(u8::try_from(code.value()).unwrap_or(1))
        }
    }
}

async fn run() -> anyhow::Result<exit::Code> {
    io::out(&format!("browseros {}", claw_cli::VERSION));
    Ok(exit::Code::Ok)
}
