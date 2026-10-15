# UI stream compatibility evidence

`capture-fixtures.ts` runs the installed **ai@7.0.114** implementation under Bun.
It writes SSE bytes through `createUIMessageStreamResponse` and captures the real
`processUIMessageStream` state after every input chunk. The SDK version guard and
`--check` mode make regeneration reviewable:

```sh
bun crates/ui-message-stream/tests/capture-fixtures.ts --check
cargo test -p ui-message-stream
bun test crates/ui-message-stream/tests/schema.test.ts
```

- `builtin`: runs `streamText` with `MockLanguageModelV3`, a dynamic MCP-style
  snapshot tool, a static schedule tool, an unknown tool, reasoning, text, and a
  second model step. This reconstructs the TS server's built-in turn using
  the real SDK translator and reducer. Error envelopes and tool outputs are fixed
  test data. The final assistant id is deliberately `""`, as TS persists it.
- `acp-legacy`: expands an abbreviated captured TS ACP sequence, fixes its UUID,
  and sends it through the real SDK reducer/framer. This is a reconstruction of
  the documented capture, not a fresh ACP subprocess capture. The spurious
  input-error updates the existing static tool; its raw input is cleared by the
  successful output. `acp` covers the corrected available-input sequence the Rust
  ACP translator emits instead.
- `prestream-error`, `midstream-error`, `abort`: reconstruct the TS error and abort
  lifecycle sequences. Interrupted text stays streaming, including after finish-step.
- `extensions`: exercises every other v7 variant, merged-stream overlap,
  metadata replacement/deep merge, approvals (including explicit null schema
  input), data upserts/transience, reset, and step-local/fallback tool lookup.
- `partial-inputs`: every character prefix is processed by the SDK. Pins repaired
  JSON, literal/number/container completion, incomplete Unicode escapes, secure
  JSON rejection, and non-JSON ACP text without inventing a partial input.
- `json-values`: opaque nested output covers insertion order, integer keys,
  escaped strings, Unicode, negative zero, and ECMAScript exponent thresholds.
- `headers.json`: actual headers from the SDK response.

Rust compares each persisted snapshot and final message, then checks complete
SSE bytes including `[DONE]` and key order. Each emitted fixture chunk must also
deserialize into generated `server_api::models::UiMessageChunk`. Bun validates
all the same payloads through the server-api contract's AJV schema and checks all 28 fixed types plus
the open data namespace are represented.

The reducer's interface is `push(&UiChunk)`. Because it cannot
return a protocol error, malformed ordering freezes the reducer and exposes an
additive `error()` accessor; it preserves the last valid message like the SDK's
failure path. Error/abort chunks are ordinary input and do not freeze reduction.
Before the first start, the message remains absent.

`ryu-js` supplies ECMAScript number formatting for SSE; serde_json's default
`1.0` / `1e-6` spelling differs from JSON.stringify's `1` / `0.000001`. Objects
preserve string-key insertion order and explicitly sort array-index keys first.
