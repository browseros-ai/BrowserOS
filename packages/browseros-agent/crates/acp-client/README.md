# acp-client

Generic ACP v1 client for one supervised agent process. This is the only workspace
crate importing `agent-client-protocol` (pinned to `=3.3.0`, no SDK features).
Callers use its re-exported `schema` and `ProtocolVersion` without depending on the
SDK's connection implementation.

`AgentConnection::connect` overlays the supplied environment on the inherited
process environment, uses the requested cwd, and initializes ACP with filesystem
and terminal capabilities disabled. `features()` describes the agent's advertised
load/resume/close, MCP, and attachment support. Unsupported HTTP/SSE MCP entries
are rejected before creating or attaching a session.

Create sessions with `new_session`; reattach with `resume_session` when advertised,
otherwise `load_session`. Load replay is swallowed. Apply desired configuration
with `set_config_option` first, then `set_mode`, after **every** attachment. Agents
can reset model or mode across reattachment. `set_config_option` returns the
agent's full option list and refreshes the local snapshot at the response's wire
position. Attachment snapshots are established before dispatching notifications
that follow new/load/resume replies. `modes()` and `config_options()` return owned snapshots shared across
session aliases; mode/config notifications refresh them even between turns.
Transcript notifications remain turn-scoped and load replay is discarded.

```rust,ignore
let session = connection.new_session(params).await?;
session.set_config_option("model", "chosen-model").await?;
session.set_mode("code").await?;
let mut turn = session.prompt(content)?;
let canceller = turn.canceller(); // cloneable; may be moved to the host stop path
while let Some(event) = turn.next().await {
    match event {
        acp_client::TurnEvent::Update(update) => handle_update(update),
        acp_client::TurnEvent::Done(result) => { handle_result(result); break; }
    }
}
```

One turn runs per session. Other sessions on the same connection may run
concurrently. Every successful `prompt` yields updates followed by exactly one
`Done`; `next` then returns `None`. Cancelling sends one `session/cancel`, cancels
pending permissions, and keeps delivering updates until the prompt response.
Stale cancellers cannot cancel later turns. Dropping `Turn` requests cancellation
and lets the background task drain. The caller owns a whole-turn deadline and
should shut down an unresponsive agent after its cancellation grace period.

Notification delivery uses an unbounded channel so a slow consumer cannot stall
SDK dispatch and prevent permission or cancellation progress. Consumers should
keep reading or drop the turn. Unknown update tags are logged with their tag and
method, then discarded without interrupting the turn; transcript payloads are not
included in that warning.

A `PermissionPolicy` returns an abstract decision. `ApproveAll` follows acpx
0.13.2: prefer `allow_once`, then `allow_always`, then the first offered option;
no options means `Cancelled`. `RejectAll` prefers `reject_once`, then
`reject_always`, otherwise `Cancelled`. Explicit always decisions prefer the
always kind before the once kind. Cancellation always returns `Cancelled`.
Async policies run outside dispatch, are dropped on cancel, and must not wait on
the same agent.

Explicit `shutdown(grace)` closes stdin and retires the process group on Unix in
three steps: until the grace deadline the child and its descendants may exit on
their own; then the group gets SIGTERM; 500 ms later, SIGKILL. Descendants that
outlive the child (`bun x` → adapter → agent CLI) are covered too, so a CLI that
writes its transcript after the adapter exits on EOF is never killed mid-flush.
A child that exits unasked (a crash) skips straight to SIGTERM for its leftovers.
It affects all connection clones and sessions. The
last connection/session handle also starts supervised cleanup, including during
an in-flight turn; turn streams, cancellers, and prompt tasks do not keep the
agent alive. Pending turns finish with `Closed` on caller teardown. Driver
failures use `Exited` when an exit report is already available. After child exit,
prompts allow `ClientOptions::response_grace` for a buffered response, then finish
with `Exited` even if an escaped descendant still holds stdout. On Unix a
supervisor drop guard also kills the process group if the runtime shuts down before cleanup finishes.
`closed()` waits for child reaping, group retirement and bounded stderr capture,
so it resolving means the whole group is gone; stderr is retained
for diagnostics, never forwarded into the conversation. Each retained line keeps
at most the final 8 KiB of incoming bytes.
Non-Unix systems terminate the direct child; descendant-group cleanup is Unix
only.

`ClientOptions` separates `shutdown_grace` (automatic teardown), `response_grace`
(final stdout response), and `stderr_grace` (diagnostic capture). All three default
to one second; `control_timeout` defaults to 30 seconds. Construct options with
`..ClientOptions::default()` when only some settings need overriding. Transport
EOF waits up to the combined response/stderr budgets for the exit report, so
configured stderr draining cannot lose a race to a shorter hard-coded deadline.
Explicit `shutdown(grace)` still controls that individual shutdown call.

Control requests have the configured deadline. A timed-out or dropped control
future retires the connection because a late reply (especially load replay)
would make its state ambiguous. Genuine agent RPC errors preserve
code/message/data. `-32000` and explicit `-32603` agent authentication failures
become `AuthRequired`, with advertised method IDs when initialization supplied
them. Invalid params (`-32602`) and errors naming an MCP server remain `Rpc`,
even when they mention authentication. SDK/transport failures become `Exited`
when the process exit report is available, otherwise `Closed`; SDK cancellation
errors are never exposed as agent RPC errors. A failed connection driver wakes
pending turns and permissions. Crashes include exit status and the stderr tail.

## Verification

From the Cargo workspace (`packages/browseros-agent`):

```sh
cargo test -p acp-client
cargo clippy -p acp-client --all-targets -- -D warnings
cargo fmt --all --check
ACP_LIVE=1 cargo test -p acp-client --test live -- --ignored --nocapture
```

The process tests use 30-second failure caps and generous independent I/O graces;
healthy peers complete without waiting for those caps. Timeout tests advance the
Tokio clock only after the fake agent acknowledges receiving the request. Unix
descendant tests observe socket EOF instead of inferring exit from heartbeat
silence, and cancellation uses an explicit acknowledgment/release handshake.
The single-threaded fake agent also avoids creating a runtime worker per CPU in
every subprocess when the suite runs concurrently.

The `acp-client-fake-agent` binary is a test fixture, not an agent launcher. Normal
tests use it with no network. The ignored live test uses `~/.local/bin/fx acp` and
`npx -y @agentclientprotocol/claude-agent-acp@0.75.1`, with the existing host logins.
It verifies prompting and fresh-process reattachment for both. Claude is requested
with model `haiku` to keep smoke-test cost small; it does not alter host settings.

Live result (2026-10-14): fx passed initialize/new/prompt/fresh-process resume/
prompt/close/shutdown. The pinned Claude adapter refused initialization with
`Internal error: Failed to authenticate: OAuth session expired and could not be
refreshed` (`-32603`, `errorKind=authentication_failed`). Refreshing the owner's
Claude login is required to complete its live smoke; fake-agent gates remain
independent of host credentials.

## Implementation decisions

- DECIDED: Keep session updates by value.
  WHY: The policy/runtime crate can consume the schema directly without SDK
  implementation dependencies or an extra allocation per event.
- DECIDED: Retire a connection when a control times out or its future is dropped.
  WHY: Late load replay and control side effects cannot safely mix with a later
  turn. The caller can launch a replacement connection.
- DECIDED: Test fixtures use a Cargo binary target and its `CARGO_BIN_EXE` path.
  WHY: This workspace separates Cargo's build directory and final artifact
  directory; deriving a sibling fixture path from the test executable is wrong.
- DECIDED: Record the generated workspace lockfile changes with this crate.
  WHY: The new pinned SDK and its transitive dependencies need reproducible
  resolution for downstream pieces and locked workspace builds.
- DECIDED: Return owned values from `modes()` and `config_options()`.
  WHY: Notifications mutate shared session state asynchronously; borrowing that
  mutable data after releasing its lock would be unsafe. Aliases share one snapshot.
- DECIDED: Preserve explicit always-policy preferences while giving the built-in
  approve/reject policies acpx 0.13.2 fallback ordering.
  WHY: Callers asking for a persistent decision should keep that preference;
  the built-in policies need compatibility when once options are unavailable.

- DECIDED: Retire the process group gracefully (descendants may finish within the
  shutdown grace, then SIGTERM, then SIGKILL) instead of SIGKILLing it the instant
  the child exits.
  WHY: Live acp-agents resume test: Claude's CLI, a grandchild of `bun x`, was still
  writing the turn's last reply to its transcript when the adapter exited on EOF;
  the instant group kill lost it and the resumed session answered "No response requested."
- DECIDED: Keep stdout, stderr, and automatic-shutdown graces independently configurable.
  WHY: A short missing-response regression must not shorten startup or diagnostic
  capture budgets on a CPU-starved host.
