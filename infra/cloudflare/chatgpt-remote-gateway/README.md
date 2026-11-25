# BrowserOS ↔ ChatGPT Remote Gateway (Cloudflare)

This package is an opt-in remote MCP gateway. It is intentionally separate from the BrowserOS local MCP listener.

## Security model

- BrowserOS MCP remains bound to loopback on the user's computer.
- `cloudflared` creates an outbound-only tunnel to a Cloudflare hostname protected by Cloudflare Access.
- The Worker is the only public MCP resource. It validates OAuth access tokens and a paired device before forwarding an MCP request.
- The Worker sends only a small header allowlist to the tunnel. It never forwards browser `Cookie`, `Authorization`, `Proxy-Authorization`, `Set-Cookie`, or arbitrary request headers.
- The Worker does not log request/response bodies. Do not add body logging, tracing payloads, or analytics containing page content.
- BrowserOS tools that expose credentials, cookies, arbitrary scripts, form filling, or secret-bearing inputs must remain blocked. Tool calls are denied unless their names are explicitly listed in `ALLOWED_TOOL_NAMES`.
- A browser's own authenticated web session remains on the local BrowserOS machine. This gateway does not export browser profile databases, cookies, passwords, or login tokens.

**Important boundary:** MCP tool results can contain page text, screenshots, or other user data. The Worker must relay those results to ChatGPT for remote control to work. This design prevents forwarding browser credential stores and cookie headers; it cannot guarantee that arbitrary page content never contains sensitive information. Review the selected tool allowlist before enabling it.

## Components

1. `worker/`: OAuth 2.1 + PKCE authorization server, Ed25519 device proof, one-time pairing state, IP-scoped rate limits, MCP reverse proxy and strict request filtering.
2. Cloudflare Tunnel: publishes the local MCP origin through an Access-protected hostname. Do not use a quick tunnel or an unauthenticated public hostname.
3. BrowserOS desktop integration: must request a one-time pairing code and display it locally. The local MCP listener must remain on loopback.

## Free-first Cloudflare resources

- Workers Free: up to 100,000 requests/day and 10 ms CPU per invocation.
- Durable Objects SQLite storage is supported on the Free plan, subject to its daily request/duration limits.
- Cloudflare Tunnel is available on all plans.
- Do not enable Workers Paid, R2, paid logging, or other paid resources for this package.

Limits and pricing can change; check the current Cloudflare plan before deployment.

## Required configuration

Worker secrets (set through Cloudflare's secret manager, never in Git):

- `TOKEN_SIGNING_KEY`: at least 32 random bytes, base64url encoded.
- `CF_ACCESS_CLIENT_ID` and `CF_ACCESS_CLIENT_SECRET`: a Cloudflare Access service token for the tunnel hostname.

Worker variables:

- `GATEWAY_ISSUER`: final HTTPS Worker/custom hostname.
- `BROWSEROS_MCP_ORIGIN`: the Access-protected tunnel hostname, without a path.
- `ALLOWED_TOOL_NAMES`: JSON array of reviewed MCP tool names. Empty by default means no browser tool calls are admitted.

The Access service token is used only by the Worker to reach the protected tunnel. It is not a browser login token and must never be placed in BrowserOS settings or source control.

## Local machine requirements

The user must run BrowserOS neo and `cloudflared` on the same trusted computer. The tunnel ingress must target only `http://127.0.0.1:9200` (or the actual loopback MCP port), never `0.0.0.0`. The tunnel hostname must require Cloudflare Access service authentication.

A Cloudflare Tunnel does not install or run itself on the user's computer. The desktop-side pairing/tunnel launcher is a separate BrowserOS integration step and is not supplied by this Worker package alone.

## Deploy order

1. Create an Access-protected tunnel hostname to the loopback MCP endpoint.
2. Create a Cloudflare Access service token restricted to that hostname.
3. Deploy this Worker to a Free Workers account with the SQLite Durable Object migration.
4. Set Worker secrets and variables using the Cloudflare dashboard.
5. Add the final Worker URL to ChatGPT as a remote MCP server and complete OAuth.
6. Pair the BrowserOS desktop instance using the one-time code shown by its local pairing UI.
7. Start with an empty tool allowlist; inspect the upstream `tools/list` response and add only reviewed tools. Keep credential, cookie, arbitrary-script, and secret-bearing tools excluded.

## Current implementation boundary

The Worker pairing API now accepts a 32-byte Ed25519 public key at `/pair/start`, returns a five-minute one-time code and transaction challenge, and requires signed device proof for `/pair/status` and `/pair/revoke`. Signatures use the UTF-8 canonical messages `browseros-pair-v1\\nstatus\\n{transaction_id}\\n{challenge}\\n{unix_timestamp}` and `browseros-pair-v1\\nrevoke\\n{device_id}\\n{nonce}\\n{unix_timestamp}`. The device private key must remain in native OS secure storage; this Worker change does not implement that desktop storage or UI. Rate limits are stored transactionally in the Durable Object and keyed by the Cloudflare client IP.

The Worker package is an isolated Preview candidate. It does not create Cloudflare resources, DNS records, Access policies, or a tunnel automatically. It does not modify BrowserOS Production or install software on the user's computer. Those steps require selecting the intended Cloudflare account/domain and a desktop-side BrowserOS pairing client.
