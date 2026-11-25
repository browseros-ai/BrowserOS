# BrowserOS desktop pairing, secure storage, and Cloudflare Tunnel

Status: implementation contract for the desktop integration. This document does not claim that a desktop pairing client or tunnel supervisor is already implemented.

## Current boundary

- BrowserOS exposes Streamable HTTP MCP locally at `/mcp`; the existing desktop MCP endpoint is resolved by `claw-app`.
- The Cloudflare Worker is a separate remote gateway. A hosted ChatGPT client cannot reach `127.0.0.1` on the user's computer.
- The current Worker candidate requires a static `PAIRING_BOOTSTRAP_SECRET` for `/pair/start` and `/pair/revoke`. Do not ship that secret in the extension, app bundle, environment defaults, or source. The desktop pairing UX is not yet present.
- The current Worker proxy needs a separate Streamable HTTP/SSE compliance review before it can be used for real MCP sessions.

## Target architecture

```text
ChatGPT -- OAuth 2.1 + PKCE --> Cloudflare Worker (public MCP endpoint)
                                      |
                               Access service token
                                      |
                           Cloudflare Access protected host
                                      |
                              Cloudflare Tunnel
                                      |
                       cloudflared on user's desktop
                                      |
                    http://127.0.0.1:<MCP port>/mcp
                                      |
                               BrowserOS MCP
```

The tunnel connector runs on the same trusted desktop as BrowserOS. It makes outbound connections; no inbound port forwarding is required. The origin must remain loopback-only. Never expose the MCP listener on `0.0.0.0`.

## Device pairing protocol (replace the static bootstrap-secret flow)

1. The native BrowserOS process creates an Ed25519 device keypair using the existing Rust crypto stack. The private key is written to the operating-system credential vault; only the public key and a non-secret device label leave the process.
2. The desktop requests a short-lived pairing challenge from the Worker, sending the public key and a bounded device label. The Worker returns a random, single-use code and a pairing transaction ID. The code expires after 5 minutes, is stored only as a hash, and is rate-limited by transaction and source.
3. The BrowserOS UI displays the code locally. It must never place the code in extension storage, telemetry, logs, URLs, or crash reports.
4. The user starts OAuth from ChatGPT. The Worker authorization page shows the requested scopes and asks the user to enter the local code and explicitly approve the device.
5. On approval, the Worker atomically consumes the pairing transaction and associates the public key with a new device record. The authorization code remains short-lived and PKCE-bound.
6. The desktop polls the transaction using a proof signed by its private key. The Worker verifies the signature against the submitted public key and returns only paired/revoked state and the device ID; it does not return the ChatGPT bearer token or any browser credentials.
7. Revoke marks the device record revoked immediately. Every MCP request re-checks device revocation. A new pairing is required after revocation or key loss.

Pairing codes must be single-use, cryptographically random, short-lived, rate-limited, and never accepted as bearer credentials on the MCP endpoint. The Worker must not accept a device ID alone as proof of possession.

## OS secure-storage contract

Add a native Rust interface owned by the desktop/server process, not the WXT extension:

- `store_private_key(device_id, key)`
- `load_private_key(device_id)`
- `delete_private_key(device_id)`
- `store_tunnel_credential(profile_id, credential)`
- `delete_tunnel_credential(profile_id)`

Requirements:

- Use the native credential vault: Windows Credential Manager, macOS Keychain, and a Linux Secret Service/KWallet provider. Do not silently fall back to plaintext files or browser storage.
- If no secure vault is available or unlocked, fail closed and explain how to enable the OS keyring. Do not write the secret to SQLite, JSON config, environment variables, extension storage, or logs.
- Store opaque references/metadata only in BrowserOS settings. Never serialize private key material.
- Restrict credential access to the current OS user. Do not sync secrets across profiles.
- Provide explicit disconnect/revoke and local credential deletion. Deleting a local key must not be represented as server-side revocation unless the revoke request was confirmed.
- Unit-test vault-unavailable, duplicate key, deletion, and error-redaction paths on each supported OS.

The final keyring crate and platform feature flags must be selected and verified against the repository's pinned Rust toolchain and CI matrix before adding a dependency. No platform-specific adapter is enabled until its build and tests pass.

## cloudflared lifecycle contract

- Offer an explicit opt-in setup action; do not install or start a background daemon silently.
- Prefer a user-scoped, officially distributed `cloudflared` binary. Verify the downloaded binary using the publisher's documented signature/checksum process before execution.
- The tunnel configuration targets only the BrowserOS MCP loopback origin and uses a dedicated hostname protected by Cloudflare Access service authentication.
- The Worker holds the Access service token. The desktop tunnel client must not receive the Worker's Access service token.
- Start/stop the connector as a child process supervised by the native BrowserOS process. Track PID, exit code, health, and version without recording request bodies.
- Stop the connector on disconnect, app shutdown, user logout, or failed health checks. Do not configure system-wide startup or a persistent service without a separate explicit user action.
- Never use a quick tunnel or a publicly reachable unauthenticated origin for this integration.
- Do not create DNS, Access applications, tunnels, Workers, or paid resources as part of a local-only pairing setup.

## MCP and data safety requirements

- Keep the allowlist empty by default. Add tools only after reviewing their schemas and side effects.
- Deny cookie/password/credential/storage access, arbitrary code execution, form submission, uploads, payments, purchases, destructive actions, and sending/publishing actions by default.
- Do not forward browser `Cookie`, `Authorization`, `Proxy-Authorization`, or `Set-Cookie` headers. Forward only the Cloudflare Access service headers and required MCP transport headers.
- Treat MCP results as user data: page text and screenshots may be returned to ChatGPT. Do not promise that page content is never exposed.
- Support Streamable HTTP POST, GET/SSE, DELETE/session lifecycle, protocol version, session IDs, and resumability according to the MCP transport specification. Do not parse an SSE response as JSON or drop session headers.
- Validate JWT algorithm, issuer, audience, expiry, scope, and device revocation on every request. Add rate limits for registration, pairing, authorization, and token endpoints.
- Add CSRF protection to the approval form and strict redirect URI validation.

## Delivery gates

1. Implement and unit-test the Worker pairing protocol and Streamable HTTP proxy behavior.
2. Implement native secure-storage adapters and desktop pairing UI.
3. Add an opt-in tunnel supervisor and platform-specific installer/update checks.
4. Restore GitHub Actions runner execution; the previous gateway CI jobs failed before any runner step (runner ID 0), so they did not validate source code.
5. Run Rust, TypeScript, UI, security, and protocol integration tests.
6. Only then prepare a Cloudflare Free-tier Preview deployment plan. No DNS changes, Production deployment, paid upgrade, or merge is included in this work.

## Explicit non-goals

- No browser cookies, passwords, profile databases, or login tokens are uploaded.
- No public listener on the desktop.
- No silent tunnel installation or startup.
- No Production deployment, DNS mutation, paid resource, or PR merge.
