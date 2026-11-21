//! The browser test double (feature `test-support`, on in every test build, never in a release).
//!
//! `Browser::scripted` gives a `Browser` that offers one `run` tool and answers each call with
//! the next scripted reply, and a `ScriptedBrowser` handle the test keeps to see every call that
//! was made, which tasks are armed, and to say that a task opened a tab (which
//! `Browser::take_opened_tabs` then returns once). The own loop, the acpx
//! runner and the run registry test against it instead of a real engine.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use tokio_util::sync::CancellationToken;

use super::{Armed, Browser, Engine, Scope, Tool, ToolOutput};

/// The browser paragraph the double puts in the system prompt.
pub const GUIDANCE: &str = "The scripted test browser. Call `run` with a script.";

/// What the double answers to one call.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptedReply {
    /// Returns this output at once.
    Output(ToolOutput),
    /// Waits until the call's cancel token fires, then returns a "Stopped" error: a script that
    /// runs until Stop.
    UntilCancelled,
}

impl ScriptedReply {
    /// A successful call returning `text`.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Output(ToolOutput {
            text: text.into(),
            pictures: Vec::new(),
            is_error: false,
        })
    }

    /// A failed script with this message.
    pub fn error(text: impl Into<String>) -> Self {
        Self::Output(ToolOutput {
            text: text.into(),
            pictures: Vec::new(),
            is_error: true,
        })
    }
}

/// One call the double received.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedCall {
    pub scope: Scope,
    pub tool: String,
    pub args: serde_json::Value,
}

/// The test's handle on a scripted browser. Clones share one state.
#[derive(Clone)]
pub struct ScriptedBrowser {
    state: Arc<State>,
}

struct State {
    replies: Mutex<VecDeque<ScriptedReply>>,
    calls: Mutex<Vec<RecordedCall>>,
    armed: Mutex<Vec<(String, CancellationToken)>>,
    /// Tabs announced with `open_tab` and not yet taken, by task.
    opened: Mutex<Vec<(String, i64)>>,
}

impl Browser {
    /// A browser that answers calls with `replies`, in order. Once they are spent, a call fails
    /// with "no scripted reply left".
    pub fn scripted(
        replies: impl IntoIterator<Item = ScriptedReply>,
    ) -> (Browser, ScriptedBrowser) {
        let handle = ScriptedBrowser {
            state: Arc::new(State {
                replies: Mutex::new(replies.into_iter().collect()),
                calls: Mutex::new(Vec::new()),
                armed: Mutex::new(Vec::new()),
                opened: Mutex::new(Vec::new()),
            }),
        };
        let browser = Browser::from_engine(Engine::Scripted(handle.clone()));
        (browser, handle)
    }
}

impl ScriptedBrowser {
    /// Every call received so far, in order.
    pub fn calls(&self) -> Vec<RecordedCall> {
        lock(&self.state.calls).clone()
    }

    /// Whether an `Armed` for `task` is still held.
    pub fn is_armed(&self, task: &str) -> bool {
        lock(&self.state.armed)
            .iter()
            .any(|(armed, token)| armed == task && !token.is_cancelled())
    }

    /// Records that `task` opened tab `tab_id`, as the engine's tab census would show after a
    /// call. `Browser::take_opened_tabs` returns it once.
    pub fn open_tab(&self, task: &str, tab_id: i64) {
        lock(&self.state.opened).push((task.to_owned(), tab_id));
    }

    pub(super) fn take_opened_tabs(&self, task: &str) -> Vec<i64> {
        let mut opened = lock(&self.state.opened);
        let (taken, kept): (Vec<_>, Vec<_>) =
            opened.drain(..).partition(|(owner, _)| owner == task);
        *opened = kept;
        taken.into_iter().map(|(_, tab_id)| tab_id).collect()
    }

    pub(super) fn tools(&self) -> Vec<Tool> {
        vec![Tool {
            name: "run".into(),
            description: "Runs a script against the browser and returns its value.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "code": { "type": "string" } },
                "required": ["code"]
            }),
        }]
    }

    pub(super) async fn call(
        &self,
        scope: &Scope,
        tool: &str,
        args: serde_json::Value,
        cancel: &CancellationToken,
    ) -> ToolOutput {
        lock(&self.state.calls).push(RecordedCall {
            scope: scope.clone(),
            tool: tool.to_owned(),
            args,
        });
        let reply = lock(&self.state.replies).pop_front();
        match reply {
            Some(ScriptedReply::Output(output)) => output,
            Some(ScriptedReply::UntilCancelled) => {
                cancel.cancelled().await;
                stopped("Stopped")
            }
            None => stopped("no scripted reply left"),
        }
    }

    pub(super) fn arm(&self, scope: Scope) -> Armed {
        let cancel = CancellationToken::new();
        lock(&self.state.armed).push((scope.task.clone(), cancel.clone()));
        Armed {
            url: format!("http://127.0.0.1:0/mcp/{}", scope.task),
            cancel,
        }
    }
}

fn stopped(text: &str) -> ToolOutput {
    ToolOutput {
        text: text.to_owned(),
        pictures: Vec::new(),
        is_error: true,
    }
}

/// A test that panicked while holding a lock has already failed; its data is still usable.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
