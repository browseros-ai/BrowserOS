//! The installed logging stack keeps `SeaORM`'s bound values out of both real log sinks.

use std::error::Error;
use std::path::Path;
use std::process::Command;
use std::{env, fs};

use claw_sdk::{AppConfig, Logging};

const SECRET: &str = "neo-seaorm-bound-value-should-stay-private";

/// Each child installs the real global subscriber and panic hook once, without changing the
/// parent test process's environment. Exiting normally also checks the file worker's flush.
fn check_logging(filter: &str, present: &[&str], absent: &[&str]) -> Result<(), Box<dyn Error>> {
    let state = tempfile::tempdir()?;
    fs::write(state.path().join("launch.json"), "{}")?;
    let output = Command::new(env::current_exe()?)
        .args(["--ignored", "--exact", "logging_probe", "--nocapture"])
        .env_clear()
        .env("BROWSERCLAW_DIR", state.path())
        .env("HOME", state.path())
        .env("CLAW_LOG", filter)
        .output()?;
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        output.status.success(),
        "logging child failed for {filter:?}: {stderr}\n{}",
        String::from_utf8(output.stdout)?
    );
    let mut file = String::new();
    for entry in fs::read_dir(state.path().join("logs"))? {
        let entry = entry?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("claw-sdk.log")
        {
            file.push_str(&fs::read_to_string(entry.path())?);
        }
    }
    for (sink, text) in [("stderr", stderr), ("daily file", file)] {
        for expected in present {
            assert!(
                text.contains(expected),
                "{sink} must retain {expected:?} with CLAW_LOG={filter:?}: {text}"
            );
        }
        for forbidden in [
            SECRET,
            "sea-orm-trace-event",
            "sea-orm-nested-trace-event",
            "stmt=",
        ]
        .iter()
        .chain(absent)
        {
            assert!(
                !text.contains(forbidden),
                "{sink} must suppress {forbidden:?} with CLAW_LOG={filter:?}: {text}"
            );
        }
    }
    Ok(())
}

#[test]
fn logging_caps_sea_orm_even_when_the_host_requests_trace() -> Result<(), Box<dyn Error>> {
    for filter in [
        "trace",
        "trace,sea_orm=trace,sea_orm::driver::sqlx_sqlite=trace",
        "trace,[execute]=trace",
    ] {
        check_logging(
            filter,
            &[
                "app-trace",
                "app-debug",
                "app-info",
                "app-warn",
                "app-error",
                "sqlx-trace",
                "sea-orm-debug",
                "sea-orm-info",
                "sea-orm-warn",
                "sea-orm-error",
                "driver-warning-inside-sea-orm",
            ],
            &[],
        )?;
    }
    Ok(())
}

#[test]
fn logging_keeps_the_hosts_normal_level_filter() -> Result<(), Box<dyn Error>> {
    check_logging(
        "info",
        &[
            "app-info",
            "app-warn",
            "app-error",
            "sea-orm-info",
            "sea-orm-warn",
            "sea-orm-error",
            "driver-warning-inside-sea-orm",
        ],
        &["app-trace", "app-debug", "sqlx-trace", "sea-orm-debug"],
    )
}

#[test]
fn logging_keeps_the_invalid_filter_fallback() -> Result<(), Box<dyn Error>> {
    check_logging(
        "[",
        &[
            "CLAW_LOG does not parse; using the default filter",
            "app-info",
            "sea-orm-info",
            "rmcp-warn",
            "driver-warning-inside-sea-orm",
        ],
        &[
            "app-trace",
            "app-debug",
            "sqlx-trace",
            "sea-orm-debug",
            "rmcp-info",
        ],
    )
}

#[test]
#[ignore = "invoked in a subprocess by the logging tests; installs process-global logging"]
fn logging_probe() -> Result<(), Box<dyn Error>> {
    let state = env::var_os("BROWSERCLAW_DIR").ok_or("missing child state directory")?;
    let launch = Path::new(&state).join("launch.json");
    let config = AppConfig::load(&launch)?;
    let logging = Logging::install(&config)?;

    tracing::trace!(target: "claw_sdk", "app-trace");
    tracing::debug!(target: "claw_sdk", "app-debug");
    tracing::info!(target: "claw_sdk", "app-info");
    tracing::warn!(target: "claw_sdk", "app-warn");
    tracing::error!(target: "claw_sdk", "app-error");
    tracing::trace!(target: "sqlx::query", "sqlx-trace");
    tracing::trace!(target: "sea_orm", value = SECRET, "sea-orm-trace-event");
    tracing::trace!(target: "sea_orm::driver::sqlx_sqlite", value = SECRET, "sea-orm-nested-trace-event");
    tracing::debug!(target: "sea_orm::driver::sqlx_sqlite", "sea-orm-debug");
    tracing::info!(target: "sea_orm", "sea-orm-info");
    tracing::warn!(target: "sea_orm", "sea-orm-warn");
    tracing::error!(target: "sea_orm", "sea-orm-error");
    tracing::info!(target: "rmcp", "rmcp-info");
    tracing::warn!(target: "rmcp", "rmcp-warn");

    // SeaORM 1.1's SQLite driver instruments `execute` at TRACE with a Debug `stmt` field
    // containing bound values. An allowed driver event inherits those fields in both formatters
    // unless the span itself is refused, even if SeaORM's own events have been filtered out.
    let stmt = format!(
        "Statement {{ sql: SELECT ?, values: Some(Values([String(Some({SECRET:?}))])), \
         db_backend: Sqlite }}"
    );
    tracing::trace_span!(target: "sea_orm::driver::sqlx_sqlite", "execute", stmt = ?stmt)
        .in_scope(|| tracing::warn!(target: "sqlx::query", "driver-warning-inside-sea-orm"));
    drop(logging);
    Ok(())
}
