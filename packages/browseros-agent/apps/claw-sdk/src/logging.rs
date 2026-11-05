use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{panic, process};

use tracing::warn;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

use crate::config::{AppConfig, Defaults};

/// The process's logging, installed once by `main` and held until `main` returns: a daily file
/// under `$BROWSERCLAW_DIR/logs` plus stderr, both filtered by `CLAW_LOG`, and a panic hook.
///
/// The file writer is non-blocking: lines are buffered on a worker thread, and dropping this
/// value flushes them. That is why `main` returns an exit code instead of calling
/// `process::exit`, which would skip the drop and lose the last lines.
#[derive(Debug)]
pub struct Logging {
    _flush_on_drop: WorkerGuard,
}

impl Logging {
    /// Creates the logs directory, installs the panic hook and the global subscriber. A
    /// `CLAW_LOG` that does not parse falls back to the default filter with one warning: a typo
    /// in a log filter must not stop the browser's agent.
    pub fn install(config: &AppConfig) -> io::Result<Self> {
        let logs = config.env.state_dir.join("logs");
        fs::create_dir_all(&logs)?;
        install_panic_hook(logs.join("claw-sdk-panics.log"));

        let appender = RollingFileAppender::builder()
            .rotation(Rotation::DAILY)
            .filename_prefix("claw-sdk.log")
            .build(&logs)
            .map_err(io::Error::other)?;
        let (file_writer, flush_on_drop) = tracing_appender::non_blocking(appender);
        let (filter, bad_filter) = match EnvFilter::try_new(&config.env.log_filter) {
            Ok(filter) => (filter, None),
            Err(error) => (EnvFilter::new(Defaults::LOG_FILTER), Some(error)),
        };
        tracing_subscriber::registry()
            .with(filter)
            .with(
                fmt::layer()
                    .with_writer(io::stderr)
                    .with_ansi(io::stderr().is_terminal()),
            )
            .with(fmt::layer().with_ansi(false).with_writer(file_writer))
            .try_init()
            .map_err(io::Error::other)?;
        if let Some(error) = bad_filter {
            warn!(
                filter = %config.env.log_filter,
                %error,
                "CLAW_LOG does not parse; using the default filter"
            );
        }
        Ok(Self {
            _flush_on_drop: flush_on_drop,
        })
    }
}

/// Appends each panic to `path` synchronously, then runs the default hook. A panic that ends
/// the process loses the non-blocking writer's buffer, so without this a crash leaves no trace;
/// a panic contained in one task is found here too.
fn install_panic_hook(path: PathBuf) {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis());
        let line = format!(
            "{millis} v{} pid={} PANIC: {info}\n",
            env!("CARGO_PKG_VERSION"),
            process::id()
        );
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
            // Best effort: inside a panic hook there is nowhere left to report a failed write.
            let _ = file.write_all(line.as_bytes());
        }
        default_hook(info);
    }));
}
