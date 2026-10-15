//! Owns the child independently of SDK dispatch. A single supervisor reaps it,
//! drains a bounded diagnostic tail, and escalates EOF shutdown to group kill;
//! connection handles never wait on or manipulate the child directly.

use std::{
    collections::VecDeque,
    process::Stdio,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use tokio::{
    io::{AsyncReadExt, BufReader},
    process::{ChildStdin, ChildStdout, Command},
    sync::{mpsc, watch},
};

use crate::{AcpError, ExitReport, LaunchSpec};

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) struct Process {
    pub(crate) pid: Option<u32>,
    tail: Arc<Mutex<VecDeque<String>>>,
    exit: watch::Receiver<Option<ExitReport>>,
    shutdown: mpsc::UnboundedSender<Duration>,
}

impl Process {
    pub(crate) fn spawn(
        spec: LaunchSpec,
        tail_lines: usize,
    ) -> Result<(Self, ChildStdin, ChildStdout), AcpError> {
        let mut command = Command::new(&spec.program);
        command
            .args(spec.args)
            .envs(spec.env)
            .current_dir(spec.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let spawn_error = |source| AcpError::Spawn {
            program: spec.program.display().to_string(),
            source,
        };
        let mut child = command.spawn().map_err(spawn_error)?;
        let missing_pipe = || std::io::Error::other("missing piped ACP process handle");
        let stdin = child
            .stdin
            .take()
            .ok_or_else(missing_pipe)
            .map_err(spawn_error)?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(missing_pipe)
            .map_err(spawn_error)?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(missing_pipe)
            .map_err(spawn_error)?;
        let pid = child.id();
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        let stderr_tail = tail.clone();
        let mut drain = tokio::spawn(async move {
            // Bound individual lines as well as their count: agents can emit a
            // giant unterminated diagnostic while stdout is still healthy.
            let mut reader = BufReader::new(stderr);
            let mut bytes = [0; 4096];
            let mut line = VecDeque::new();
            loop {
                match reader.read(&mut bytes).await {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        for &byte in &bytes[..count] {
                            if byte == b'\n' {
                                push_line(&stderr_tail, &mut line, tail_lines);
                            } else {
                                if line.len() == 8192 {
                                    line.pop_front();
                                }
                                line.push_back(byte);
                            }
                        }
                    }
                }
            }
            if !line.is_empty() {
                push_line(&stderr_tail, &mut line, tail_lines);
            }
        });
        let (exit_tx, exit) = watch::channel(None);
        let (shutdown, mut requests) = mpsc::unbounded_channel();
        let report_tail = tail.clone();
        tokio::spawn(async move {
            let status = tokio::select! {
                status = child.wait() => status.ok(),
                grace = requests.recv() => {
                    match tokio::time::timeout(grace.unwrap_or(Duration::ZERO), child.wait()).await {
                        Ok(status) => status.ok(),
                        Err(_) => {
                            kill_group(pid);
                            let _ = child.start_kill();
                            child.wait().await.ok()
                        }
                    }
                }
            };
            // A wrapper may exit before its descendants. They retain our pipe
            // descriptors, so group cleanup is also required after normal exit.
            kill_group(pid);
            if tokio::time::timeout(Duration::from_secs(1), &mut drain)
                .await
                .is_err()
            {
                drain.abort();
                let _ = drain.await;
            }
            let report = ExitReport {
                status,
                stderr_tail: lock(&report_tail).iter().cloned().collect(),
            };
            exit_tx.send_replace(Some(report));
        });
        Ok((
            Self {
                pid,
                tail,
                exit,
                shutdown,
            },
            stdin,
            stdout,
        ))
    }

    pub(crate) fn tail(&self) -> Vec<String> {
        lock(&self.tail).iter().cloned().collect()
    }
    pub(crate) fn report(&self) -> Option<ExitReport> {
        self.exit.borrow().clone()
    }
    pub(crate) fn stop(&self, grace: Duration) {
        let _ = self.shutdown.send(grace);
    }
    pub(crate) async fn closed(&self) -> ExitReport {
        let mut exit = self.exit.clone();
        loop {
            if let Some(report) = exit.borrow_and_update().clone() {
                return report;
            }
            if exit.changed().await.is_err() {
                return ExitReport {
                    status: None,
                    stderr_tail: self.tail(),
                };
            }
        }
    }
}

fn push_line(tail: &Mutex<VecDeque<String>>, line: &mut VecDeque<u8>, limit: usize) {
    let bytes: Vec<_> = line.drain(..).collect();
    if limit == 0 {
        return;
    }
    let mut tail = lock(tail);
    if tail.len() == limit {
        tail.pop_front();
    }
    tail.push_back(
        String::from_utf8_lossy(&bytes)
            .trim_end_matches('\r')
            .to_string(),
    );
}

#[cfg(unix)]
fn kill_group(pid: Option<u32>) {
    if let Some(pid) = pid
        .and_then(|pid| i32::try_from(pid).ok())
        .and_then(rustix::process::Pid::from_raw)
    {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
}

#[cfg(not(unix))]
fn kill_group(_: Option<u32>) {}
