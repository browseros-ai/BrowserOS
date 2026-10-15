//! `filesystem_bash` (port of `bash.ts`): one shell command in the workspace with a timeout,
//! stdout then stderr, tail-truncated. The child is killed on timeout, on cancellation and when
//! the call is dropped (`kill_on_drop`), so a stopped turn never leaves a command running.

use super::{
    FsResult, Input,
    util::{DEFAULT_BASH_TIMEOUT, js_number, or_default, truncate_tail},
};
use std::{path::Path, process::Stdio, time::Duration};
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

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let read_stdout = tokio::spawn(read_all(stdout));
    let read_stderr = tokio::spawn(read_all(stderr));

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

    let (stdout_text, stderr_text) = if timed_out {
        let drained = tokio::time::timeout(DRAIN_AFTER_KILL, async {
            (read_stdout.await, read_stderr.await)
        })
        .await;
        match drained {
            Ok((out, err)) => (out.unwrap_or_default(), err.unwrap_or_default()),
            Err(_) => (String::new(), String::new()),
        }
    } else {
        (
            read_stdout.await.unwrap_or_default(),
            read_stderr.await.unwrap_or_default(),
        )
    };

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

async fn read_all<R: tokio::io::AsyncRead + Unpin>(reader: Option<R>) -> String {
    let Some(mut reader) = reader else {
        return String::new();
    };
    let mut bytes = Vec::new();
    let _ = reader.read_to_end(&mut bytes).await;
    String::from_utf8_lossy(&bytes).into_owned()
}
