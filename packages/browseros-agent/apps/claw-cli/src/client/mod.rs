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

use crate::exit::{Code, Failure};
use crate::io;
use crate::state::{State, Store};
use result::ToolOutput;
use transport::Transport;

/// JSON-RPC ids only have to be unique within a connection, and one process
/// makes a handful of calls.
static NEXT_ID: AtomicU64 = AtomicU64::new(2);

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

pub struct Session {
    transport: Transport,
    store: Store,
    state: State,
    /// Set when this process performed the handshake, so the notice is printed
    /// once rather than per call.
    announced: bool,
}

impl Session {
    /// Binds a resolved endpoint to a scope file, without talking to the server.
    pub fn new(transport: Transport, store: Store, mut state: State) -> Self {
        state.rebind(transport.endpoint());
        Self {
            transport,
            store,
            state,
            announced: false,
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

    /// Performs `initialize` + `notifications/initialized` and stores the id.
    ///
    /// # Errors
    /// Fails when the server refuses the handshake, which is not recoverable
    /// by retrying.
    pub async fn handshake(&mut self, auto: bool) -> Result<()> {
        let message = transport::initialize_message("browseros-cli", crate::VERSION);
        let reply = self.transport.post(&message, None).await?;
        let Some(session_id) = reply.session_id.clone() else {
            return Err(Failure::no_browser(format!(
                "the server did not issue a session id (HTTP {}): {}",
                reply.status,
                reply.raw.trim()
            ))
            .into());
        };
        let _ = self
            .transport
            .post(&transport::initialized_message(), Some(&session_id))
            .await;

        self.persist(|state| {
            state.mcp_session_id = Some(session_id.clone());
            if state.started_at.is_none() {
                state.started_at = Some(crate::state::now_stamp());
            }
        })?;

        if auto && !self.announced {
            self.announced = true;
            io::note(&format!(
                "[started session {} in this directory]",
                short(&session_id)
            ));
        }
        Ok(())
    }

    /// Releases the transport session. The server answers 202.
    ///
    /// # Errors
    /// Propagates a connection failure.
    pub async fn release(&mut self) -> Result<Option<u16>> {
        let Some(session_id) = self.state.mcp_session_id.clone() else {
            return Ok(None);
        };
        let status = self.transport.delete_session(&session_id).await?;
        self.persist(|state| state.mcp_session_id = None)?;
        Ok(Some(status))
    }

    /// Calls one tool, pairing implicitly and recovering from a dead session.
    ///
    /// The recovery is exactly one retry. A loop here is how a restarting
    /// server turns into a tab storm, which is the failure the server's own
    /// `succeed_retired_session` comment describes from the other side.
    ///
    /// # Errors
    /// Returns a classified [`Failure`] for anything the caller should exit on.
    pub async fn call(&mut self, tool: &str, arguments: Value, auto: bool) -> Result<ToolOutput> {
        if self.state.mcp_session_id.is_none() {
            if !auto {
                return Err(Failure::no_browser(
                    "no session in this directory and --no-auto-session was given.\n  Run: browseros session start",
                )
                .into());
            }
            self.handshake(true).await?;
        }
        let arguments = self.with_agent_name(tool, arguments);

        let first = self.send(tool, &arguments).await?;
        if !first.is_session_gone() {
            return self.interpret(first);
        }

        // The session died with the server. Re-handshake, then retry once.
        self.handshake(false).await?;
        let retry = self.send(tool, &arguments).await?;
        if retry.is_session_gone() {
            return Err(Failure::no_browser(
                "the server refused a freshly created session. It may be shutting down.",
            )
            .into());
        }
        self.warn_recovered();
        self.interpret(retry)
    }

    /// Fetches the tool list once and caches the names against the server version.
    ///
    /// This is the single largest response the CLI will ever read, about 62 KB,
    /// so reading it per pairing rather than per command is the difference
    /// between the CLI being cheaper than MCP and being the same.
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
        if self.state.mcp_session_id.is_none() {
            self.handshake(true).await?;
        }
        let session_id = self.state.mcp_session_id.clone();
        let reply = self
            .transport
            .post(&transport::list_message(next_id()), session_id.as_deref())
            .await?;
        let names = reply
            .body
            .as_ref()
            .and_then(|body| body.pointer("/result/tools"))
            .and_then(Value::as_array)
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|tool| tool.get("name").and_then(Value::as_str))
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !names.is_empty() {
            let cached = names.clone();
            self.persist(|state| {
                state.tools = cached;
                state.server_version = Some(version);
            })?;
        }
        Ok(names)
    }

    async fn send(&self, tool: &str, arguments: &Value) -> Result<transport::Reply> {
        let id = next_id();
        let message = transport::call_message(id, tool, arguments);
        let session_id = self.state.mcp_session_id.clone();
        tokio::select! {
            // A transport error here means the request never reached the tool,
            // so there is nothing to cancel; the classified error is the whole
            // answer.
            reply = self.transport.post(&message, session_id.as_deref()) => reply,
            // Dying on ctrl-C without telling the server leaves it running a
            // tool nobody will read, and on a page-changing tool that keeps
            // acting on the user's browser after the caller has gone.
            () = interrupted() => {
                cancel_in_flight(&self.transport, id, session_id.as_deref()).await;
                Err(Failure::new(
                    Code::Interrupted,
                    "interrupted. The server was told to stop the call.",
                )
                .into())
            }
        }
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

    fn warn_recovered(&self) {
        let id = self
            .state
            .mcp_session_id
            .as_deref()
            .map(short)
            .unwrap_or_default();
        let group = self
            .state
            .group_id
            .as_deref()
            .map(short)
            .unwrap_or_default();
        let pages = self
            .state
            .page
            .map(|page| format!(" (page {page})"))
            .unwrap_or_default();
        io::note(&format!(
            "[session was restarted by the server; recovered as {id}{}.\n Tabs opened before the restart{pages} remain attributed to the previous session.\n Close them, or re-open what you still need.]",
            if group.is_empty() {
                String::new()
            } else {
                format!(" and rejoined group {group}")
            }
        ));
    }
}

/// Builds a session from flags, resolving the endpoint and loading the scope.
///
/// # Errors
/// Returns [`Code::NoBrowser`] when nothing answers any candidate endpoint.
pub async fn connect(
    endpoint_flag: Option<&str>,
    agent: Option<&str>,
    read_timeout: Duration,
) -> Result<Session> {
    let dir = std::env::current_dir()?;
    let root = crate::state::worktree_root(&dir);
    let scope = crate::state::scope(&dir, root.as_deref());
    let store = Store::open(&scope)?;
    let cached = store.load();

    let resolved = crate::endpoint::resolve(endpoint_flag, cached.endpoint.as_deref()).await?;
    if let Some(other) = &resolved.ambiguous_with {
        io::note(&format!(
            "[both a release and a dev server are running; using {} ({}). The other is {other}.\n Pass --endpoint or set BROWSERCLAW_DIR to choose.]",
            resolved.url,
            resolved.origin.label()
        ));
    }

    let transport = Transport::new(&resolved.url, read_timeout)?;
    let mut session = Session::new(transport, store, cached);
    if let Some(agent) = agent {
        session.persist(|state| state.agent = Some(agent.to_string()))?;
    }
    Ok(session)
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

/// Tells the server to stop the call this client is abandoning.
///
/// `dispatch.rs` distinguishes this from a human pressing Stop in the cockpit:
/// this one is `client_cancel`, which yields `ProtocolCancelled` rather than an
/// operator cancellation, so the session is not retired.
///
/// Best effort on purpose. The caller is already leaving, and a server that
/// cannot be told is not a reason to hold the process open.
pub async fn cancel_in_flight(transport: &Transport, id: u64, session: Option<&str>) {
    let _ = transport
        .post_within(
            &transport::cancelled_message(id),
            session,
            Some(transport::CANCEL_TIMEOUT),
        )
        .await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
