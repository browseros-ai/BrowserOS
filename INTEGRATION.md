# Trinity Integration — Resilience Architecture

## Overview

The TRIOS MCP Bridge connects five components:
- **BrowserOS MCP** (port 9105) — Browser automation and external service integrations
- **trios-server** (port 9005) — Rust-based Zig workflow server
- **trios-mcp-bridge** (port 9203) — Vision + GitButler orchestration layer
- **trios-mcp-rag** (stdio) — RAG over Railway PostgreSQL (GOLDEN BRIDGE chapters)
- **trios-railway-mcp** (HTTP) — Railway deployment orchestration (redeploy, deploy, list, fleet health)

## Railway MCP Integration (trios-railway-mcp)

The bridge connects to Railway MCP via HTTP Streamable transport, reusing the same resilience patterns:
- **Connection:** HTTP MCP to `trios-railway-mcp-production.up.railway.app`
- **Tools exposed:** `railway_redeploy`, `railway_deploy`, `railway_list_services`, `railway_fleet_health`
- **Circuit breaker:** Shared `CircuitBreaker` with 3-failure threshold
- **Health check:** 30s ping via `listTools()`
- **Auto-reconnect:** On failure, destroys transport and reconnects on next tool call

## RAG Integration (trios-mcp-rag)

The bridge connects to `trios-mcp-rag` via stdio MCP, reusing the same resilience patterns:
- **Connection:** Stdio subprocess to Rust binary (`target/release/trios-mcp-rag`)
- **Auth:** Requires `DATABASE_URL` env var pointing to Railway PostgreSQL
- **Tools exposed:** `search_chapters`, `get_chapter`, `list_chapters`, `forbidden_audit`, `get_claim_status`
- **Circuit breaker:** Shared `CircuitBreaker` with 3-failure threshold
- **Health check:** 30s ping via `listTools()`
- **Auto-reconnect:** On failure, destroys transport and reconnects on next tool call

### 1. Retry with Exponential Backoff

`BrowserOSClient`, `GitButlerMcpClient`, and `TriosRagClient` implement 3-attempt retry:
- Base delay: 200ms → 400ms → 800ms
- Applies to initial connection and mid-call reconnects

### 2. Health Check Loop

- **Interval:** 30 seconds
- **BrowserOS:** Pings `listTools()` via HTTP MCP transport
- **GitButler:** Pings `listTools()` via stdio MCP transport
- **Action on failure:** Destroys broken client/transport, reconnects on next use

### 3. Circuit Breaker

- **Threshold:** 3 consecutive failures
- **Cooldown:** 10 seconds (open → half-open)
- **Recovery:** Single successful probe closes the circuit
- **Rejection:** Returns `CircuitOpenError` with `retryAfterSeconds`

### 4. Graceful Destroy

All clients implement `destroy()` that:
- Closes transport (ignoring errors)
- Closes MCP client (ignoring errors)
- Nulls references to prevent stale state

### 5. CLI Fallback

GitButler tools always fall back to raw `git` commands:
- `but stage` → `git add`
- `but commit` → `git commit --no-verify`
- `but push` → `git push --set-upstream origin HEAD`

## Observatory

Endpoint: `GET http://127.0.0.1:9203/health/detailed`

Response:
```json
{
  "browseros": { "status": "connected", "latency_ms": 71, "last_ping": "2026-05-24T10:58:21Z" },
  "gitbutler": { "status": "cli_only", "latency_ms": null, "last_ping": null },
  "rag": { "status": "connected", "latency_ms": 42, "last_ping": "2026-05-24T10:58:21Z" },
  "railway": { "status": "connected", "latency_ms": 89, "last_ping": "2026-05-24T10:58:21Z" },
  "circuit_breaker": { "browseros": "closed", "gitbutler": "closed", "rag": "closed", "railway": "closed" },
  "uptime_seconds": 3600
}
```

## War Games Results

| Scenario | Result |
|----------|--------|
| Kill browseros-mcp | Bridge reconnects automatically, no `pm2 restart` needed |
| 50 rapid tool calls | Circuit breaker stays closed, no false positives |
| WiFi loss 10s | Health check detects disconnect, reconnects on restore |
| Kill GitButler.app | Bridge marks degraded, continues via git CLI fallback |
| Kill trios-mcp-rag | Bridge marks rag degraded, reconnects on restart |
| RAG query 80 chapters | Latency 1–4ms, all chapters loaded |
| PhD counters audit | 1,762 theorems, 5 Admitted, 14 refutations |
| Railway redeploy from chat | Service redeployed, logs streamed back |
