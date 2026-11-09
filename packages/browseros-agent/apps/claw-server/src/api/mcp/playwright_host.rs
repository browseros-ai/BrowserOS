//! Neo: run-scoped host exchanges. Current MCP uses SEP-2322 continuations because rmcp's
//! stateless HTTP transport discards standalone response POSTs. Legacy sessions can answer
//! in-flight requests. Only the handler polls peer calls, preserving rmcp's task-local origin.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use futures_util::{StreamExt, stream::FuturesUnordered};
use neo_runtime::{HostCallback, HostRequest};
use rmcp::{
    ErrorData as McpError, Peer, RoleServer,
    model::{
        ClientResult, ElicitRequest, ElicitRequestParams, ElicitResult, ElicitationAction,
        ElicitationSchema, InputRequest, InputRequiredResult, InputResponses, ProtocolVersion,
        RequestMetaObject, ServerRequest,
    },
    service::RequestContext,
};
use serde_json::{Value, json};
use tokio::{
    sync::{Mutex, mpsc, oneshot},
    task::JoinHandle,
};
use tokio_util::{
    sync::{CancellationToken, DropGuard},
    task::TaskTracker,
};

use super::Reply;

/// Neo: bound parked run ownership on a shared server.
const MAX_PARKED: usize = 128;
/// Neo: go-sdk v1.8 permits nine input responses before its tenth-call retry limit.
const MAX_ROUNDS: usize = 9;

pub(super) struct Job {
    request: HostRequest,
    answer: oneshot::Sender<Result<Value, String>>,
    ended: CancellationToken,
}

pub(super) fn channel() -> (HostCallback, mpsc::Receiver<Job>) {
    let (sender, receiver) = mpsc::channel(1);
    let callback: HostCallback = Arc::new(move |request| {
        let sender = sender.clone();
        Box::pin(async move {
            let ended = CancellationToken::new();
            // Runtime drops this future on hook timeout/cancel, including while queued.
            let _end = ended.clone().drop_guard();
            let (answer, response) = oneshot::channel();
            sender
                .send(Job {
                    request,
                    answer,
                    ended,
                })
                .await
                .map_err(|_| "host connection closed".to_string())?;
            response
                .await
                .map_err(|_| "host connection closed".to_string())?
        })
    });
    (callback, receiver)
}

pub(super) struct Active {
    execution: JoinHandle<Reply>,
    inbox: mpsc::Receiver<Job>,
    cancel: CancellationToken,
    _cancel_on_drop: DropGuard,
    binding: Value,
    rounds: usize,
}

impl Active {
    pub(super) fn new(
        execution: JoinHandle<Reply>,
        inbox: mpsc::Receiver<Job>,
        cancel: CancellationToken,
        binding: Value,
    ) -> Self {
        Self {
            execution,
            inbox,
            _cancel_on_drop: cancel.clone().drop_guard(),
            cancel,
            binding,
            rounds: 0,
        }
    }
}

struct Parked {
    active: Active,
    job: Job,
}

#[derive(Clone, Default)]
pub(super) struct Runs(Arc<Mutex<HashMap<String, Parked>>>);

pub(super) enum Step {
    Complete(Reply),
    Input(InputRequiredResult),
}

impl Runs {
    pub(super) async fn resume(
        &self,
        state: &str,
        binding: &Value,
        answers: Option<InputResponses>,
        context: &RequestContext<RoleServer>,
    ) -> Result<Active, McpError> {
        let mut parked = self.0.lock().await;
        // Check before consuming: a malformed retry cannot steal a legitimate continuation.
        if !parked
            .get(state)
            .is_some_and(|run| run.active.binding == *binding && !run.job.ended.is_cancelled())
        {
            return Err(McpError::invalid_params(
                "unknown, expired or mismatched host continuation",
                None,
            ));
        }
        let Some(run) = parked.remove(state) else {
            return Err(McpError::invalid_params(
                "host continuation already consumed",
                None,
            ));
        };
        let result = if supports_form(context) {
            answers
                .and_then(|mut answers| {
                    (answers.len() == 1)
                        .then(|| answers.remove("host"))
                        .flatten()
                })
                .ok_or_else(|| "missing host answer".to_string())
                .and_then(parse_answer)
        } else {
            Err("client does not support form elicitation".to_string())
        };
        let _ = run.job.answer.send(result);
        Ok(run.active)
    }

    pub(super) async fn drive(
        &self,
        mut active: Active,
        context: &RequestContext<RoleServer>,
        tasks: &TaskTracker,
        shutdown: &CancellationToken,
    ) -> Result<Step, McpError> {
        let mrtr = context
            .protocol_version()
            .is_some_and(|v| v >= ProtocolVersion::V_2026_07_28);
        let mut pending = FuturesUnordered::new();
        let mut open = true;
        loop {
            tokio::select! {
                result = &mut active.execution => return result.map(Step::Complete)
                    .map_err(|e| McpError::internal_error(e.to_string(), None)),
                () = context.ct.cancelled(), if !active.cancel.is_cancelled() => active.cancel.cancel(),
                job = active.inbox.recv(), if open => {
                    let Some(job) = job else { open = false; continue; };
                    if job.ended.is_cancelled() { continue; }
                    if !supports_form(context) {
                        let _ = job.answer.send(Err("client does not support form elicitation".to_string()));
                        continue;
                    }
                    if !mrtr {
                        pending.push(elicit(job, context.peer.clone()));
                        continue;
                    }
                    if active.rounds >= MAX_ROUNDS {
                        let _ = job.answer.send(Err("host round limit reached".to_string()));
                        continue;
                    }
                    let request = match elicitation(&job.request) {
                        Ok(request) => request,
                        Err(error) => { let _ = job.answer.send(Err(error)); continue; }
                    };
                    let mut parked = self.0.lock().await;
                    if parked.len() >= MAX_PARKED {
                        let _ = job.answer.send(Err("too many pending host runs".to_string()));
                        continue;
                    }
                    let state = uuid::Uuid::new_v4().to_string();
                    let ended = job.ended.clone();
                    active.rounds += 1;
                    parked.insert(state.clone(), Parked { active, job });
                    drop(parked);
                    let runs = self.clone();
                    let key = state.clone();
                    let shutdown = shutdown.clone();
                    // No parked run survives the hook waiting for it, or server shutdown.
                    tasks.spawn(async move {
                        tokio::select! { () = ended.cancelled() => {}, () = shutdown.cancelled() => {} }
                        runs.0.lock().await.remove(&key);
                    });
                    return Ok(Step::Input(InputRequiredResult::new(
                        Some(BTreeMap::from([("host".to_string(), InputRequest::Elicitation(request))])),
                        Some(state),
                    )));
                }
                _ = pending.next(), if !pending.is_empty() => {}
            }
        }
    }
}

fn supports_form(context: &RequestContext<RoleServer>) -> bool {
    context
        .client_capabilities()
        .and_then(|c| c.elicitation)
        .is_some_and(|c| c.form.is_some() || c.url.is_none())
}

fn elicitation(request: &HostRequest) -> Result<ElicitRequest, String> {
    let mut meta = RequestMetaObject::new();
    meta.insert(
        "com.browseros.neo/host".to_string(),
        json!({"name":request.name,"args":request.args}),
    );
    Ok(ElicitRequest::new(
        ElicitRequestParams::FormElicitationParams {
            meta: Some(meta),
            message: format!("Neo hook host request: {}", request.name),
            requested_schema: ElicitationSchema::builder()
                .required_string_property("json", |s| s)
                .build()
                .map_err(|e| e.to_string())?,
        },
    ))
}

fn parse_answer(value: Value) -> Result<Value, String> {
    let answer: ElicitResult =
        serde_json::from_value(value).map_err(|_| "invalid host answer".to_string())?;
    if answer.action != ElicitationAction::Accept {
        return Err("host request declined or cancelled".to_string());
    }
    let content = answer
        .content
        .ok_or_else(|| "missing host answer".to_string())?;
    let text = content
        .get("json")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing host JSON".to_string())?;
    serde_json::from_str(text).map_err(|_| "invalid host JSON".to_string())
}

async fn elicit(job: Job, peer: Peer<RoleServer>) {
    let result = async {
        let request = elicitation(&job.request)?;
        let answer = peer
            .send_request(ServerRequest::ElicitRequest(request))
            .await
            .map_err(|_| "host connection failed".to_string())?;
        match answer {
            ClientResult::ElicitResult(answer) => parse_answer(
                serde_json::to_value(answer).map_err(|_| "invalid host answer".to_string())?,
            ),
            _ => Err("invalid host answer".to_string()),
        }
    };
    tokio::select! {
        () = job.ended.cancelled() => {},
        answer = result => { let _ = job.answer.send(answer); }
    }
}
