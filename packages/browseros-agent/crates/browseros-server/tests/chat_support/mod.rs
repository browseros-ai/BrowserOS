//! What the chat tests share: a `TurnRunner` driven by a script with gates the test opens, and
//! a reader for the server-sent events of an in-process response. Built on `tests/support`.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use axum::body::{Body, BodyDataStream};
use axum::http::Response;
use browseros_server::browser::ScriptedBrowser;
use browseros_server::store::Store;
use browseros_server::turn::{
    ConversationId, EventSink, Target, TurnEnd, TurnEvent, TurnInput, TurnRunner,
};
use futures_util::StreamExt as _;
use serde_json::Value;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

/// How long any single read in these tests may wait before the test fails instead of hanging.
pub const PATIENCE: Duration = Duration::from_secs(10);

/// One step of a scripted turn.
pub enum Step {
    Emit(TurnEvent),
    /// Waits until the test calls `Runner::open_gate`. Stop ends the turn here with `Stopped`.
    Gate,
    /// The scripted browser reports that this turn's task opened the tab.
    OpenTab(i64),
    /// Waits for Stop, then this long, then goes on with the script: a backend slow to unwind.
    AfterStop(Duration),
    /// Never returns, whatever happens: a backend that ignores Stop.
    Hang,
    Yield,
    Pause(Duration),
}

/// A scripted turn: its steps and how it ends.
pub struct Script {
    pub steps: Vec<Step>,
    pub end: TurnEnd,
}

/// The row as it was on disk right after a checkpoint event returned.
pub struct Observed {
    pub after: &'static str,
    pub row: Option<Value>,
}

/// Plays one script per turn, in order; a turn with no script left answers nothing and stops.
pub struct Runner {
    scripts: Mutex<VecDeque<Script>>,
    gate: Semaphore,
    store: OnceLock<Store>,
    browser: OnceLock<ScriptedBrowser>,
    inputs: Mutex<Vec<(Target, TurnInput)>>,
    observed: Mutex<Vec<Observed>>,
    ended: Mutex<Vec<ConversationId>>,
}

impl Runner {
    pub fn new(scripts: impl IntoIterator<Item = Script>) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            scripts: Mutex::new(scripts.into_iter().collect()),
            gate: Semaphore::new(0),
            store: OnceLock::new(),
            browser: OnceLock::new(),
            inputs: Mutex::new(Vec::new()),
            observed: Mutex::new(Vec::new()),
            ended: Mutex::new(Vec::new()),
        })
    }

    pub fn open_gate(&self) {
        self.gate.add_permits(1);
    }

    /// Makes every checkpoint read the row back from this store.
    pub fn observe(&self, store: Store) {
        let _ = self.store.set(store);
    }

    pub fn with_browser(&self, browser: ScriptedBrowser) {
        let _ = self.browser.set(browser);
    }

    pub fn inputs(&self) -> Vec<(Target, TurnInput)> {
        lock(&self.inputs).clone()
    }

    pub fn take_observed(&self) -> Vec<Observed> {
        std::mem::take(&mut *lock(&self.observed))
    }

    pub fn ended_sessions(&self) -> Vec<ConversationId> {
        lock(&self.ended).clone()
    }
}

#[async_trait::async_trait]
impl TurnRunner for Runner {
    async fn run_turn(
        &self,
        target: &Target,
        input: &TurnInput,
        sink: &mut dyn EventSink,
        cancel: &CancellationToken,
    ) -> TurnEnd {
        lock(&self.inputs).push((target.clone(), input.clone()));
        let Some(script) = lock(&self.scripts).pop_front() else {
            return TurnEnd::Done(browseros_server::turn::FinishReason::Stop);
        };
        for step in script.steps {
            match step {
                Step::Emit(event) => {
                    let after = match &event {
                        TurnEvent::ToolInput { .. } => Some("ToolInput"),
                        TurnEvent::ToolDone { .. } => Some("ToolDone"),
                        _ => None,
                    };
                    sink.emit(event).await;
                    if let (Some(after), Some(store)) = (after, self.store.get()) {
                        let row = store
                            .conversations()
                            .load(input.conversation)
                            .await
                            .ok()
                            .flatten()
                            .and_then(|row| serde_json::to_value(row.messages).ok());
                        lock(&self.observed).push(Observed { after, row });
                    }
                }
                Step::Gate => {
                    tokio::select! {
                        permit = self.gate.acquire() => {
                            if let Ok(permit) = permit {
                                permit.forget();
                            }
                        }
                        () = cancel.cancelled() => return TurnEnd::Stopped,
                    }
                }
                Step::OpenTab(tab) => {
                    if let Some(browser) = self.browser.get() {
                        browser.open_tab(&input.conversation.to_string(), tab);
                    }
                }
                Step::AfterStop(delay) => {
                    cancel.cancelled().await;
                    tokio::time::sleep(delay).await;
                }
                Step::Hang => std::future::pending::<()>().await,
                Step::Yield => tokio::task::yield_now().await,
                Step::Pause(duration) => tokio::time::sleep(duration).await,
            }
        }
        script.end
    }

    async fn end_session(&self, conversation: ConversationId) {
        lock(&self.ended).push(conversation);
    }
}

/// The `data` of each server-sent event of a response, read as it arrives.
pub struct Events {
    body: BodyDataStream,
    buffer: String,
}

impl Events {
    pub fn of(response: Response<Body>) -> Self {
        Self {
            body: response.into_body().into_data_stream(),
            buffer: String::new(),
        }
    }

    /// The next event's data, or `None` once the body ended (or nothing came for `PATIENCE`).
    pub async fn next(&mut self) -> Option<String> {
        loop {
            if let Some(end) = self.buffer.find("\n\n") {
                let event: String = self.buffer.drain(..end + 2).collect();
                let data: Vec<&str> = event
                    .lines()
                    .filter_map(|line| line.strip_prefix("data: "))
                    .collect();
                return Some(data.join("\n"));
            }
            match tokio::time::timeout(PATIENCE, self.body.next()).await {
                Ok(Some(Ok(bytes))) => self.buffer.push_str(&String::from_utf8_lossy(&bytes)),
                _ => return None,
            }
        }
    }

    /// Every event up to and including the first chunk of type `kind`.
    pub async fn until(&mut self, kind: &str) -> Vec<String> {
        let mut read = Vec::new();
        while let Some(data) = self.next().await {
            let found = chunk_type(&data) == kind;
            read.push(data);
            if found {
                return read;
            }
        }
        panic!("the stream ended before a `{kind}` chunk: {read:?}");
    }

    /// Every event until the body ends.
    pub async fn rest(&mut self) -> Vec<String> {
        let mut read = Vec::new();
        while let Some(data) = self.next().await {
            read.push(data);
        }
        read
    }
}

/// A chunk's `type`, or the data itself for `[DONE]`.
pub fn chunk_type(data: &str) -> String {
    serde_json::from_str::<Value>(data)
        .ok()
        .and_then(|chunk| chunk["type"].as_str().map(str::to_owned))
        .unwrap_or_else(|| data.to_owned())
}

pub fn chunk_types(events: &[String]) -> Vec<String> {
    events.iter().map(|data| chunk_type(data)).collect()
}

pub fn chunk(data: &str) -> Value {
    serde_json::from_str(data).unwrap_or(Value::Null)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
