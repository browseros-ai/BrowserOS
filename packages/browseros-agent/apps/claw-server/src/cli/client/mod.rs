//! The only place that talks to the server.
//!
//! Every command builds a tool name and a JSON argument object and hands it
//! over here. That single choke point is what makes the grammar test possible:
//! "command line in, tool call out" is a pure function everywhere else.

pub mod result;
pub mod transport;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

use crate::cli::exit::{Code, Failure};
use crate::cli::io;
use crate::cli::state::{State, Store};
use result::ToolOutput;
use transport::Transport;

/// JSON-RPC ids only have to be unique within a connection, and one process
/// makes a handful of calls.
static NEXT_ID: AtomicU64 = AtomicU64::new(2);

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// The argument every call presents its task's group in.
const GROUP_ARG: &str = "group";
/// The `_meta` key a result names its task's group under.
const GROUP_META: &str = "com.browseros.neo/group";
/// How the server starts its refusal on a computer without the ultrafast plan.
const NOT_ON_PLAN: &str = "ultrafast_required:";

/// A resolved server and this directory's scope file. There is no session: the
/// task is the tab group recorded in the scope, and that is all a call carries.
pub struct Client {
    transport: Transport,
    store: Store,
    state: State,
}

impl Client {
    /// Binds a resolved endpoint to a scope file, without talking to the server.
    pub fn new(transport: Transport, store: Store, mut state: State) -> Self {
        state.rebind(transport.endpoint());
        Self {
            transport,
            store,
            state,
        }
    }

    #[must_use]
    pub fn state(&self) -> &State {
        &self.state
    }

    #[must_use]
    pub fn transport(&self) -> &Transport {
        &self.transport
    }

    #[must_use]
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Persists whatever the caller changed, keeping the in-memory copy in step.
    ///
    /// # Errors
    /// Propagates a write failure.
    pub fn persist<F>(&mut self, mutate: F) -> Result<()>
    where
        F: FnOnce(&mut State),
    {
        let endpoint = self.transport.endpoint().to_string();
        let saved = self.store.update(|state| {
            state.rebind(&endpoint);
            mutate(state);
        })?;
        self.state = saved;
        Ok(())
    }

    /// Calls one tool as part of this directory's task.
    ///
    /// The tab group recorded here is presented on every call, and the server
    /// resolves the task from it; with none recorded, the call starts new work.
    /// The server names the task's group back in every result, and that is what
    /// the next command presents, so nothing has to be carried by the caller.
    ///
    /// # Errors
    /// Returns a classified [`Failure`] for anything the caller should exit on.
    pub async fn call(&mut self, tool: &str, arguments: Value) -> Result<ToolOutput> {
        let closing = tool == "tab_groups"
            && arguments.get("action").and_then(Value::as_str) == Some("close");
        let arguments = self.with_group(self.with_agent_name(tool, arguments));
        let reply = self.send(tool, &arguments).await?;
        let group = reply
            .body
            .as_ref()
            .and_then(group_named_by)
            .map(ToString::to_string);
        let output = self.interpret(reply)?;
        if output.is_error && output.text.starts_with(NOT_ON_PLAN) {
            let reason = output.text.trim_start_matches(NOT_ON_PLAN).trim();
            return Err(Failure::new(Code::NotOnPlan, reason.to_string()).into());
        }
        self.follow_group(group, !closing)?;
        Ok(output)
    }

    /// Fetches the tool list once and caches the names against the server version.
    ///
    /// This is the single largest response the CLI will ever read, about 62 KB,
    /// so reading it once per server version rather than per command is the
    /// difference between the CLI being cheaper than MCP and being the same.
    ///
    /// # Errors
    /// Propagates a connection failure; an unparsable list leaves the cache empty.
    pub async fn refresh_tools(&mut self) -> Result<Vec<String>> {
        let version = self
            .transport
            .system()
            .await
            .and_then(|info| {
                info.get("version")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .unwrap_or_default();
        if !self.state.tools.is_empty() && self.state.server_version.as_deref() == Some(&version) {
            return Ok(self.state.tools.clone());
        }
        let reply = self
            .transport
            .post(&transport::list_message(next_id()))
            .await?;
        let listed = reply
            .body
            .as_ref()
            .and_then(|body| body.pointer("/result/tools"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let names = listed
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        // The run budget is configurable per deployment and stated in the run
        // tool's own schema, so read it while the listing is already in hand
        // rather than carrying a constant that a deployment can contradict.
        let budget = listed
            .iter()
            .find(|tool| tool.get("name").and_then(Value::as_str) == Some("run"))
            .and_then(crate::cli::script::budget_from_schema);
        if !names.is_empty() {
            let cached = names.clone();
            self.persist(|state| {
                state.tools = cached;
                state.server_version = Some(version);
                state.run_budget_ms = budget;
            })?;
        }
        Ok(names)
    }

    async fn send(&self, tool: &str, arguments: &Value) -> Result<transport::Reply> {
        let message = transport::call_message(next_id(), tool, arguments);
        tokio::select! {
            // A transport error here means the request never reached the tool,
            // so there is nothing to cancel; the classified error is the whole
            // answer.
            reply = self.transport.post(&message) => reply,
            // Dropping the request closes its connection, which is how the
            // stateless protocol cancels: the server stops a tool whose caller
            // has gone, as a client cancellation rather than an operator Stop.
            () = interrupted() => Err(Failure::new(
                Code::Interrupted,
                "interrupted. The call was abandoned, which stops it on the server.",
            )
            .into()),
        }
    }

    /// Presents the task's group, when this directory has one.
    fn with_group(&self, arguments: Value) -> Value {
        let Some(group) = self.state.group_id.clone() else {
            return arguments;
        };
        let Value::Object(mut map) = arguments else {
            return arguments;
        };
        map.insert(GROUP_ARG.to_string(), Value::String(group));
        Value::Object(map)
    }

    /// Keeps the recorded group in step with the one the server says the task
    /// works in. A result naming none means the task has no group open, so the
    /// next page starts new work; `announce` says so unless the caller closed it.
    fn follow_group(&mut self, group: Option<String>, announce: bool) -> Result<()> {
        if self.state.group_id == group {
            return Ok(());
        }
        if group.is_none() && announce {
            io::note(
                "[this directory's task has ended because its tab group was closed. The next page you open starts a new task.]",
            );
        }
        self.persist(|state| {
            if group.is_none() {
                state.task = None;
            }
            state.group_id = group;
        })
    }

    /// `agentName` is a fallback identity for the calling product, gated on
    /// protocol 2026-07-28. A CLI invoked by Claude Code, by Codex and by a
    /// human should not all produce tabs labelled the same.
    fn with_agent_name(&self, tool: &str, arguments: Value) -> Value {
        let Some(agent) = self.state.agent.clone() else {
            return arguments;
        };
        // Only catalog tools carry the decorated argument; the server's own
        // tools reject unknown fields, and every tool is deny_unknown_fields.
        if !self.state.offers(tool) {
            return arguments;
        }
        let Value::Object(mut map) = arguments else {
            return arguments;
        };
        map.entry("agentName".to_string())
            .or_insert(Value::String(agent));
        Value::Object(map)
    }

    fn interpret(&self, reply: transport::Reply) -> Result<ToolOutput> {
        if reply.status == 403 {
            return Err(Failure::no_browser(
                "the server rejected the request as a browser fetch (403).\n  This build is sending an Origin or Sec-Fetch-Site header, which /mcp refuses.",
            )
            .into());
        }
        if reply.status == 415 {
            return Err(Failure::tool(
                "the server rejected the content type (415). Expected application/json.",
            )
            .into());
        }
        let Some(body) = reply.body else {
            return Err(Failure::tool(format!(
                "unreadable reply (HTTP {}): {}",
                reply.status,
                reply.raw.trim()
            ))
            .into());
        };
        if let Some(error) = body.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("the server reported an error");
            return Err(Failure::tool(message.to_string()).into());
        }
        let Some(call_result) = body.get("result") else {
            return Err(Failure::tool(format!("reply carried no result: {body}")).into());
        };
        Ok(result::from_call_result(call_result))
    }
}

/// Builds a client from flags, resolving the endpoint and loading the scope.
///
/// # Errors
/// Returns [`Code::NoBrowser`] when nothing answers any candidate endpoint.
pub async fn connect(
    endpoint_flag: Option<&str>,
    agent: Option<&str>,
    read_timeout: Duration,
) -> Result<Client> {
    let dir = std::env::current_dir()?;
    let root = crate::cli::state::worktree_root(&dir);
    let scope = crate::cli::state::scope(&dir, root.as_deref());
    let store = Store::open(&scope)?;
    let cached = store.load();

    let resolved = crate::cli::endpoint::resolve(endpoint_flag, cached.endpoint.as_deref()).await?;
    if let Some(other) = &resolved.ambiguous_with {
        io::note(&format!(
            "[both a release and a dev server are running; using {} ({}). The other is {other}.\n Pass --endpoint or set BROWSERCLAW_DIR to choose.]",
            resolved.url,
            resolved.origin.label()
        ));
    }

    let transport = Transport::new(&resolved.url, read_timeout)?;
    let mut client = Client::new(transport, store, cached);
    if let Some(agent) = agent {
        client.persist(|state| state.agent = Some(agent.to_string()))?;
    }
    Ok(client)
}

/// Resolves when the caller presses ctrl-C.
///
/// A failure to install the handler must never cancel a working call, so it
/// resolves to a future that never completes instead.
async fn interrupted() {
    if tokio::signal::ctrl_c().await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// First eight characters of an id, which is how the cockpit prints them.
#[must_use]
pub fn short(id: &str) -> String {
    id.chars().take(8).collect()
}

/// A convenience for the handful of local commands that still need a value.
#[must_use]
pub fn object(pairs: Vec<(&str, Value)>) -> Value {
    let mut map = serde_json::Map::new();
    for (key, value) in pairs {
        if !value.is_null() {
            map.insert(key.to_string(), value);
        }
    }
    Value::Object(map)
}

/// Used by `status` to say whether the browser link is up.
#[must_use]
pub fn link_summary(ready: Option<(u16, Value)>) -> String {
    match ready {
        Some((200, _)) => "connected".to_string(),
        Some((503, body)) => {
            let down = body
                .pointer("/browser/downForMs")
                .and_then(Value::as_i64)
                .map(|ms| format!(" (down for {}s)", ms / 1000))
                .unwrap_or_default();
            format!("not connected{down}")
        }
        Some((status, _)) => format!("unknown (HTTP {status})"),
        None => "unknown".to_string(),
    }
}

/// Classifies an HTTP status the CLI received outside a tool call.
#[must_use]
pub fn status_code(status: u16) -> Code {
    match status {
        200..=299 => Code::Ok,
        503 => Code::NoBrowser,
        _ => Code::ToolFailed,
    }
}

/// The group a reply's result names, if it names one.
fn group_named_by(body: &Value) -> Option<&str> {
    body.pointer("/result/_meta")?.get(GROUP_META)?.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_group_is_read_from_the_result_metadata() {
        let body = json!({ "result": { "_meta": { "com.browseros.neo/group": "G1" } } });
        assert_eq!(group_named_by(&body), Some("G1"));
        assert_eq!(group_named_by(&json!({ "result": {} })), None);
    }

    #[test]
    fn an_object_drops_null_arguments_rather_than_sending_them() {
        let value = object(vec![("page", Value::from(3)), ("url", Value::Null)]);
        assert_eq!(value, json!({ "page": 3 }));
    }

    #[test]
    fn link_summary_names_the_outage_length() {
        let summary = link_summary(Some((503, json!({ "browser": { "downForMs": 4200 } }))));
        assert_eq!(summary, "not connected (down for 4s)");
    }

    #[test]
    fn a_ready_server_is_just_connected() {
        assert_eq!(link_summary(Some((200, Value::Null))), "connected");
    }
}
