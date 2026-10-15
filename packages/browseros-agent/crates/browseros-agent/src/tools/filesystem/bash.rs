//! `filesystem_bash` (port of `bash.ts`): one shell command in the workspace with a timeout,
//! stdout then stderr, tail-truncated. The child is killed on timeout, on cancellation and when
//! the call is dropped (`kill_on_drop`), so a stopped turn never leaves a command running.

use super::{
    FsResult, Input,
    util::{DEFAULT_BASH_TIMEOUT, js_number, or_default, truncate_tail},
};
use std::{
    path::Path,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

/// After a kill, how long to wait for the pipes to drain. A grandchild that inherited them could
/// otherwise keep the call open forever.
const DRAIN_AFTER_KILL: Duration = Duration::from_secs(2);

fn shell() -> (String, &'static str) {
    if cfg!(windows) {
        ("cmd.exe".to_owned(), "/c")
    } else {
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|shell| !shell.is_empty())
            .unwrap_or_else(|| "/bin/sh".to_owned());
        (shell, "-c")
    }
}

pub(super) async fn run(cwd: &Path, input: &Input, cancel: CancellationToken) -> Result<FsResult, String> {
    let command = input.str("command").unwrap_or_default();
    let timeout_seconds = or_default(input.number("timeout"), DEFAULT_BASH_TIMEOUT);
    let (shell, flag) = shell();

    // `resolve(cwd)`: lexical, not canonical — the shell sees the workspace path as configured.
    let mut child = tokio::process::Command::new(shell)
        .arg(flag)
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| error.to_string())?;

    let stdout = Captured::default();
    let stderr = Captured::default();
    let read_stdout = tokio::spawn(capture(child.stdout.take(), stdout.clone()));
    let read_stderr = tokio::spawn(capture(child.stderr.take(), stderr.clone()));

    let timeout = Duration::from_secs_f64(timeout_seconds.max(0.0));
    let mut timed_out = false;
    let status = tokio::select! {
        biased;
        () = cancel.cancelled() => return Err("Command cancelled".to_owned()),
        status = child.wait() => status.map_err(|error| error.to_string())?,
        () = tokio::time::sleep(timeout) => {
            timed_out = true;
            let _ = child.start_kill();
            child.wait().await.map_err(|error| error.to_string())?
        }
    };

    let drained = async {
        let _ = read_stdout.await;
        let _ = read_stderr.await;
    };
    if timed_out {
        // Keep what was read so far even if the pipes never close.
        let _ = tokio::time::timeout(DRAIN_AFTER_KILL, drained).await;
    } else {
        drained.await;
    }
    let (stdout_text, stderr_text) = (stdout.text(), stderr.text());

    let mut output = stdout_text;
    if !stderr_text.is_empty() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&stderr_text);
    }

    if timed_out {
        let truncated = truncate_tail(&output);
        return Ok(FsResult::error(format!(
            "Command timed out after {}s\n\n{}",
            js_number(timeout_seconds),
            truncated.content
        )));
    }

    let truncated = truncate_tail(&output);
    let mut result = truncated.content;
    if truncated.truncated {
        result = format!(
            "(Output truncated. Showing last {} of {} lines)\n{result}",
            truncated.kept_lines, truncated.total_lines
        );
    }

    let exit_code = exit_code(status);
    if exit_code != 0 {
        result.push_str(&format!("\n\n[Exit code: {exit_code}]"));
        return Ok(FsResult::error(result));
    }
    if result.is_empty() {
        result = "(no output)".to_owned();
    }
    Ok(FsResult::text(result))
}

/// Bun reports a signal-killed process as `128 + signal`.
fn exit_code(status: std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    1
}

/// Output read so far; shared with the reader task so a timed-out call still reports it.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        self.0
            .lock()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default()
    }
}

async fn capture<R: tokio::io::AsyncRead + Unpin>(reader: Option<R>, into: Captured) {
    let Some(mut reader) = reader else {
        return;
    };
    let mut chunk = [0u8; 8192];
    while let Ok(read) = reader.read(&mut chunk).await {
        if read == 0 {
            break;
        }
        if let Ok(mut bytes) = into.0.lock() {
            bytes.extend_from_slice(&chunk[..read]);
        }
    }
}
