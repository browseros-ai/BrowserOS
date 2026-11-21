//! The command line. Parsed with `try_parse`, never `parse`: clap's own usage exit is 2, and the
//! browser reads exit 2 as "port in use" and would relaunch on the next port forever.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::error::ErrorKind;
use clap::{Parser, Subcommand};

use super::Action;

#[derive(Debug, Parser)]
#[command(
    name = "browseros_server",
    disable_version_flag = true,
    disable_help_subcommand = true,
    args_conflicts_with_subcommands = true
)]
struct Cli {
    /// Print the version alone on stdout and exit.
    #[arg(long, conflicts_with_all = ["config", "print_openapi"])]
    version: bool,
    /// The browser's launch file (`<execution dir>/config.json`).
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Print the generated OpenAPI document and exit.
    #[arg(long, hide = true, conflicts_with = "config")]
    print_openapi: bool,
    #[command(subcommand)]
    engine: Option<Engine>,
}

#[derive(Debug, Subcommand)]
enum Engine {
    /// The browser engine child this server starts as a second copy of itself.
    #[command(name = "__neo-engine", hide = true)]
    NeoEngine {
        #[arg(long, value_name = "PATH")]
        config: PathBuf,
    },
}

/// The action, or the exit code to return at once: 0 after `--help`, 1 for any usage error.
pub(super) fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Action, ExitCode> {
    let cli = Cli::try_parse_from(args).map_err(|error| {
        // clap writes help to stdout and usage errors to stderr.
        let _ = error.print();
        match error.kind() {
            ErrorKind::DisplayHelp => ExitCode::SUCCESS,
            _ => ExitCode::FAILURE,
        }
    })?;
    match cli {
        Cli {
            engine: Some(Engine::NeoEngine { config }),
            ..
        } => Ok(Action::Engine(config)),
        Cli { version: true, .. } => Ok(Action::PrintVersion),
        Cli {
            print_openapi: true,
            ..
        } => Ok(Action::PrintOpenApi),
        Cli {
            config: Some(config),
            ..
        } => Ok(Action::Serve(config)),
        Cli { .. } => {
            let _ = clap::Error::raw(
                ErrorKind::MissingRequiredArgument,
                "one of --config <PATH>, --version or --print-openapi is required\n",
            )
            .print();
            Err(ExitCode::FAILURE)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(args: &[&str]) -> Result<Action, ExitCode> {
        parse(
            std::iter::once("browseros_server")
                .chain(args.iter().copied())
                .map(OsString::from),
        )
    }

    #[test]
    fn every_form_the_browser_and_the_codegen_use_parses() {
        assert_eq!(action(&["--version"]), Ok(Action::PrintVersion));
        assert_eq!(action(&["--print-openapi"]), Ok(Action::PrintOpenApi));
        assert_eq!(
            action(&["--config=/x/config.json"]),
            Ok(Action::Serve("/x/config.json".into()))
        );
        assert_eq!(
            action(&["--config", "/x/config.json"]),
            Ok(Action::Serve("/x/config.json".into()))
        );
        assert_eq!(
            action(&["__neo-engine", "--config", "/e.json"]),
            Ok(Action::Engine("/e.json".into()))
        );
    }

    #[test]
    fn usage_errors_exit_1_never_2() {
        assert_eq!(action(&["--bogus"]), Err(ExitCode::FAILURE));
        assert_eq!(action(&[]), Err(ExitCode::FAILURE));
        assert_eq!(action(&["--config"]), Err(ExitCode::FAILURE));
        assert_eq!(action(&["--help"]), Err(ExitCode::SUCCESS));
    }
}
