//! Owns the child independently of SDK dispatch. A single supervisor reaps it,
//! drains a bounded diagnostic tail, and retires its process group gracefully
//! (drain, SIGTERM, SIGKILL); connection handles never touch the child directly.

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
    time::Instant,
};

use crate::{AcpError, ExitReport, LaunchSpec};

/// After SIGTERM, how long the leader or its group may take to exit before SIGKILL.
const TERM_GRACE: Duration = Duration::from_millis(500);
const GROUP_POLL: Duration = Duration::from_millis(20);

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
        stderr_grace: Duration,
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
        // Construct outside the future: even a supervisor dropped before its
        // first poll must kill descendants. Child::kill_on_drop reaches only
        // the direct child, so it cannot enforce this process-group invariant.
        let group_guard = ProcessGroupGuard(pid);
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
            let mut group_guard = group_guard;
            // Shutdown: stdin is already closed; the leader gets until the grace
            // deadline to exit on EOF, then SIGTERM, then SIGKILL.
            let (status, drain_until) = tokio::select! {
                status = child.wait() => (status.ok(), None),
                grace = requests.recv() => {
                    let deadline = Instant::now() + grace.unwrap_or(Duration::ZERO);
                    let status = match tokio::time::timeout_at(deadline, child.wait()).await {
                        Ok(status) => status.ok(),
                        Err(_) => {
                            group_guard.terminate();
                            match tokio::time::timeout(TERM_GRACE, child.wait()).await {
                                Ok(status) => status.ok(),
                                Err(_) => {
                                    group_guard.kill();
                                    let _ = child.start_kill();
                                    child.wait().await.ok()
                                }
                            }
                        }
                    };
                    (status, Some(deadline))
                }
            };
            // A wrapper may exit before its descendants, which retain our pipe
            // descriptors, so the group is always retired. Never with an instant
            // SIGKILL: `bun x` → adapter → agent CLI exits leader-first on EOF, and
            // Claude's CLI was killed mid-flush, losing the turn's last reply from
            // its resumable transcript. The report waits, so `closed()` still
            // means the whole group is gone.
            group_guard.retire(drain_until).await;
            if tokio::time::timeout(stderr_grace, &mut drain)
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

#[derive(Clone, Copy)]
enum GroupSignal {
    Term,
    Kill,
}

#[cfg(unix)]
fn group_id(pid: Option<u32>) -> Option<rustix::process::Pid> {
    pid.and_then(|pid| i32::try_from(pid).ok())
        .and_then(rustix::process::Pid::from_raw)
}

#[cfg(unix)]
fn signal_group(pid: Option<u32>, signal: GroupSignal) {
    let signal = match signal {
        GroupSignal::Term => rustix::process::Signal::TERM,
        GroupSignal::Kill => rustix::process::Signal::KILL,
    };
    if let Some(group) = group_id(pid) {
        let _ = rustix::process::kill_process_group(group, signal);
    }
}

/// Whether any member of the group is still alive (zombies count until reaped).
#[cfg(unix)]
fn group_alive(pid: Option<u32>) -> bool {
    group_id(pid).is_some_and(|group| rustix::process::test_kill_process_group(group).is_ok())
}

// No process groups: kill_on_drop reaches the direct child only.
#[cfg(not(unix))]
fn signal_group(_: Option<u32>, _: GroupSignal) {}

#[cfg(not(unix))]
fn group_alive(_: Option<u32>) -> bool {
    false
}

/// Owns the child's process group (the child leads it). Its `Drop` is the
/// synchronous fallback when runtime shutdown or task cancellation drops the
/// supervisor. Disarming once the group is gone avoids signaling a reused ID.
struct ProcessGroupGuard(Option<u32>);
impl ProcessGroupGuard {
    fn kill(&mut self) {
        signal_group(self.0.take(), GroupSignal::Kill);
    }

    fn terminate(&self) {
        signal_group(self.0, GroupSignal::Term);
    }

    fn alive(&self) -> bool {
        group_alive(self.0)
    }

    /// Lets descendants finish on their own until `drain_until` (requested
    /// shutdown only), then SIGTERM, then SIGKILL after [`TERM_GRACE`].
    async fn retire(&mut self, drain_until: Option<Instant>) {
        self.wait_empty(drain_until.unwrap_or_else(Instant::now))
            .await;
        if self.0.is_none() {
            return;
        }
        self.terminate();
        self.wait_empty(Instant::now() + TERM_GRACE).await;
        self.kill();
    }

    async fn wait_empty(&mut self, deadline: Instant) {
        while self.0.is_some() {
            if !self.alive() {
                self.0 = None;
                return;
            }
            if Instant::now() >= deadline {
                return;
            }
            tokio::time::sleep(GROUP_POLL.min(deadline.saturating_duration_since(Instant::now())))
                .await;
        }
    }
}
impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        self.kill();
    }
}
