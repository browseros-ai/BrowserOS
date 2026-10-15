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

The follow-up closes the caller integrations:

- Management list/get routes consume `list_stored` / `get_stored` and project raw ids and unknown
  type/target strings. Get/delete still enforce TS's UUID path validation (non-UUID legacy ids
  remain listed, but those paths return 400). Valid UUID text is passed to SQLite unchanged,
  including case. Creation, updates, and the execution model retain the closed contract.
- `chat/history.rs::persist` returns save errors. The completion handoff carries both final
  messages and an optional error; the run emits that error before `[DONE]` and publishes final
  history with status `failed`. Scheduled cleanup still runs on failure. Scheduled built-in
  runs retain their no-persistence policy; ACP display history is persisted unconditionally.
- Instance-lock acquisition retries contention for up to two seconds. A short-lived descriptor
  inherited before exec, or a departing Rust process, no longer makes immediate reopen fail.
  A live owner still prevents startup, and non-contention I/O errors are returned immediately.

DECIDED: Keep lossless storage rows separate from validated execution rows — WHY: closed ACP
execution types cannot represent every legacy or future row; management reads must preserve
those rows without changing request validation or accepting unsupported execution targets.

DECIDED: A durable-save failure overrides completion/cancellation with a failed turn — WHY:
a reply that was not saved must not be reported as successfully persisted. Keep its messages
visible and finish resource cleanup before ending the stream.

DECIDED: OAuth refresh deletes durable tokens only for HTTP 400/401 with `invalid_grant` or
`invalid_token` — WHY: the TS behavior logs users out during provider outages; transient,
ambiguous, malformed and transport errors must preserve credentials for a later retry.
