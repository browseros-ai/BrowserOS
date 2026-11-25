# PC Connect: Upstash Box adapter

Status: design scaffold only. The adapter is not enabled until a Box runtime with the required API permissions is available.

## Goal

Expose an optional remote Linux execution target through the existing BrowserOS MCP service. Reuse the existing `rmcp` server and its transport/authentication boundaries; do not create a second MCP server or change the default BrowserOS runtime.

## Runtime model

- Existing target: local BrowserOS host.
- Optional remote target: an already-provisioned Upstash Box selected by an operator.
- Future target: GitHub Codespaces adapter, disabled unless the operator already has an available Codespace.
- No automatic Box/Codespace provisioning, paid plan upgrade, or keep-alive setting.

## Proposed tool surface

| Tool | Initial behavior | Required capability |
| --- | --- | --- |
| `pc_connect.status` | Report configured target metadata and reachability only | `status:read` |
| `pc_connect.exec` | Execute argv (never shell-concatenated user input) in a selected Box | `exec:write` |
| `pc_connect.files` | Read/write files under an explicitly configured workspace root | `files:read`, `files:write` |
| `pc_connect.browser` | Delegate to BrowserOS browser tooling; no raw CDP port exposure | `browser:control` |

The first implementation should ship status/read-only discovery before enabling execution or file writes.

## Security requirements

1. Keep the Upstash API key in the deployment secret store; never commit it, log it, or return it from status tools.
2. Bind each connection to an explicit Box ID; do not create or resume arbitrary resources implicitly.
3. Default to read-only. Require separately granted scopes for execution, file writes, and browser control.
4. Accept command arguments as an argv array. Do not interpolate untrusted strings into a shell command.
5. Enforce workspace path containment and reject traversal, symlink escapes, and writes outside the configured root.
6. Apply command timeout, output-size limits, cancellation, and per-request audit events with secrets and sensitive output redacted.
7. Do not expose Box SSH, Docker, or raw CDP ports publicly.
8. Fail closed when the Box API is unavailable or the connector lacks write scopes.

## Integration points to inspect during implementation

- `packages/browseros-agent/apps/claw-server/src/api/mcp/mod.rs`
- `packages/browseros-agent/apps/claw-server/src/api/mcp/dispatch.rs`
- `packages/browseros-agent/apps/claw-server/Cargo.toml`
- Existing `browseros-mcp::catalog` and `execute_tool` dispatch contracts.

The Box adapter should be isolated behind a trait so the existing MCP server remains usable without Upstash credentials. Prefer a small sidecar/adapter process if introducing an Upstash SDK dependency into the Rust workspace would create an unnecessary runtime or dependency burden.

## Acceptance gates

- Unit tests for scope checks, argv validation, path containment, timeout, output truncation, and redaction.
- Integration test against an existing non-production Box only after authorized write-capable access is granted.
- MCP initialize, tools/list, and tool-call smoke tests.
- Confirm no new Box, paid resource, production deployment, or change to `main`.
