# Data-safety hardening handoff

The DB layer now takes an OS instance lock in the BrowserOS directory, makes a consistent
one-time `VACUUM INTO` backup (fsync then atomic rename), admits every store mutation with
`BEGIN IMMEDIATE` and bounded BUSY retry, opens ahead schemas read-only, and reports corruption
recreation separately with preserved-original path and per-table salvage counts. Salvage
continues table by table and row by row; broken schema-page tags are repaired on scratch only
and accepted only after a full SQLite integrity check. Other page recovery uses the optional
`sqlite3 .recover` CLI when available. No recovery output or token values are logged.

The TS server does not take the Rust instance lock. SQLite write admission handles its OTA
write overlap. This serializes transactions; it cannot resolve stale application-level
read/modify/write state accumulated before a transaction began.

Two caller integrations are outside P2-fix's owned files and remain required:

1. `http/agents.rs`, `http/conversations.rs`, `http/wire/crud.rs`, and the generated API contract
   use UUID ids / closed agent and target enums. Management list/get routes must consume the
   new `list_stored` / `get_stored` methods and serialize their raw strings. Delete methods now
   accept raw string ids too, but HTTP path validation also needs to allow those ids. Existing
   typed methods remain as execution views and return a clear error for unsupported rows
   instead of silently dropping them. Their UUID normalization is not used in lossless reads.
2. `chat/history.rs::persist` swallows `save_conversation` errors. P8-fix owns chat and must
   propagate these into the turn outcome. DB saves already return admission/commit errors;
   the BUSY exhaustion regression proves the error and unchanged persisted history.

DECIDED: Keep lossless storage rows separate from validated execution rows — WHY: changing
`AgentKind`/`TargetType` or UUID row fields breaks exhaustive matches and ACP execution in the
explicitly unowned chat/HTTP interfaces. This preserves their source contract while exposing
all stored data for the required management-route follow-up.

DECIDED: OAuth refresh deletes durable tokens only for HTTP 400/401 with `invalid_grant` or
`invalid_token` — WHY: the TS behavior logs users out during provider outages; transient,
ambiguous, malformed and transport errors must preserve credentials for a later retry.
