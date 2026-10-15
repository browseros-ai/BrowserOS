//! JSON logging uses stderr and an optional execution-directory file; stdout belongs
//! to the binary's --version protocol. Rotation keeps one .old backup and failures
//! degrade to stderr rather than turning a full disk or closed pipe into a crash.

use crate::config::EnvConfig;
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};
use tracing_subscriber::{EnvFilter, fmt::MakeWriter};
const MAX_BYTES: u64 = 50 * 1024 * 1024;
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const ROTATION_CHECK: Duration = Duration::from_secs(5 * 60);

pub fn init(env: &EnvConfig) {
    install(env, None);
}
pub fn init_file(env: &EnvConfig, execution_dir: &Path) {
    let sink = RotatingFile::open(execution_dir)
        .map_err(|error| {
            let _ = writeln!(
                io::stderr().lock(),
                "Failed to configure log directory {}: {error}",
                execution_dir.display()
            );
            error
        })
        .ok();
    install(env, sink);
}
fn install(env: &EnvConfig, sink: Option<RotatingFile>) {
    let default = if env.node_env.as_deref() == Some("development") {
        "debug"
    } else {
        "info"
    };
    let level = match env.log_level.as_deref().unwrap_or(default) {
        "silent" => "off",
        "fatal" => "error",
        level => level,
    };
    let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new(default));
    let writer = LogWriter(sink.map(|sink| Arc::new(Mutex::new(sink))));
    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .with_writer(writer)
        .try_init();
}

struct RotatingFile {
    path: PathBuf,
    file: Option<File>,
    next_check: Instant,
}
impl RotatingFile {
    fn open(directory: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        let path = directory.join("browseros-server.log");
        if let Ok(metadata) = std::fs::metadata(&path)
            && (metadata.len() > MAX_BYTES
                || metadata
                    .modified()
                    .ok()
                    .and_then(|time| SystemTime::now().duration_since(time).ok())
                    .is_some_and(|age| age > MAX_AGE))
        {
            rotate(&path)?;
        }
        let file = OpenOptions::new().append(true).create(true).open(&path)?;
        Ok(Self {
            path,
            file: Some(file),
            next_check: Instant::now() + ROTATION_CHECK,
        })
    }
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        if Instant::now() >= self.next_check {
            self.next_check = Instant::now() + ROTATION_CHECK;
            if self
                .file
                .as_ref()
                .and_then(|file| file.metadata().ok())
                .is_some_and(|metadata| metadata.len() > MAX_BYTES)
            {
                // Release the handle before rename, including on Windows. Reopen even
                // when rotation fails so a transient filesystem error cannot disable logs.
                self.file.take();
                let rotated = rotate(&self.path);
                self.file = OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(&self.path)
                    .ok();
                rotated?;
            }
        }
        match &mut self.file {
            Some(file) => file.write_all(bytes),
            None => Err(io::Error::other("log file unavailable")),
        }
    }
}
fn rotate(path: &Path) -> io::Result<()> {
    let backup = path.with_file_name("browseros-server.log.old");
    match std::fs::remove_file(&backup) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    std::fs::rename(path, backup)
}

#[derive(Clone)]
struct LogWriter(Option<Arc<Mutex<RotatingFile>>>);
impl<'a> MakeWriter<'a> for LogWriter {
    type Writer = Self;
    fn make_writer(&'a self) -> Self {
        self.clone()
    }
}
impl Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let _ = io::stderr().lock().write_all(bytes);
        if let Some(sink) = &self.0
            && let Ok(mut sink) = sink.lock()
        {
            let _ = sink.write(bytes);
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
