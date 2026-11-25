interface Env {
  GATEWAY_STATE: DurableObjectNamespace;
  GATEWAY_ISSUER: string;
  BROWSEROS_MCP_ORIGIN: string;
  ALLOWED_TOOL_NAMES: string;
  TOKEN_SIGNING_KEY: string;
  CF_ACCESS_CLIENT_ID: string;
  CF_ACCESS_CLIENT_SECRET: string;
}

type Pairing = { deviceId: string; transactionId: string; publicKey: string; challenge: string; label: string; createdAt: number; expiresAt: number; paired: boolean };
type PairTransaction = { transactionId: string; deviceId: string; publicKey: string; challenge: string; state: "pending" | "paired" | "revoked"; createdAt: number; expiresAt: number };
type Client = { clientId: string; redirectUris: string[]; createdAt: number };
type AuthRequest = { clientId: string; redirectUri: string; state: string; challenge: string; scope: string; expiresAt: number };
type AuthCode = { clientId: string; redirectUri: string; challenge: string; deviceId: string; scope: string; expiresAt: number };
type AccessClaims = { iss: string; aud: string; sub: string; device_id: string; scope: string; iat: number; exp: number };

const encoder = new TextEncoder();
const json = (data: unknown, status = 200) => new Response(JSON.stringify(data), {
  status, headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" }
});
const randomId = (bytes = 24) => {
  const value = crypto.getRandomValues(new Uint8Array(bytes));
  return btoa(String.fromCharCode(...value)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
};
const b64url = (value: ArrayBuffer | ArrayBufferView) => {
  const bytes = value instanceof ArrayBuffer
    ? new Uint8Array(value)
    : new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  return btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
};
const sha256 = async (value: string) => b64url(await crypto.subtle.digest("SHA-256", encoder.encode(value)));
function fromB64url(value: string): Uint8Array | null {
  if (!/^[A-Za-z0-9_-]+$/.test(value)) return null;
  try {
    const normalized = value.replace(/-/g, "+").replace(/_/g, "/");
    const binary = atob(normalized + "=".repeat((4 - normalized.length % 4) % 4));
    return Uint8Array.from(binary, (char) => char.charCodeAt(0));
  } catch { return null; }
}
async function verifyEd25519(publicKey: string, signature: string, message: string): Promise<boolean> {
  const keyBytes = fromB64url(publicKey);
  const signatureBytes = fromB64url(signature);
  if (!keyBytes || keyBytes.length !== 32 || !signatureBytes || signatureBytes.length !== 64) return false;
  try {
    const key = await crypto.subtle.importKey("raw", keyBytes, { name: "Ed25519" }, false, ["verify"]);
    return await crypto.subtle.verify({ name: "Ed25519" }, key, signatureBytes, encoder.encode(message));
  } catch { return false; }
}
const hmac = async (secret: string, value: string) => {
  const key = await crypto.subtle.importKey("raw", encoder.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  return b64url(await crypto.subtle.sign("HMAC", key, encoder.encode(value)));
};
const safeEqual = (a: string, b: string) => {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
};
const html = (value: string) => value.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
const noStoreHtml = (body: string) => new Response(body, {
  headers: {
    "content-type": "text/html; charset=utf-8", "cache-control": "no-store",
    "x-content-type-options": "nosniff",
    "content-security-policy": "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'"
  }
});

function allowedTools(env: Env): Set<string> {
  try {
    const parsed = JSON.parse(env.ALLOWED_TOOL_NAMES || "[]");
    return new Set(Array.isArray(parsed) ? parsed.filter((x) => typeof x === "string") : []);
  } catch { return new Set(); }
}
function hasSensitiveInput(value: unknown, depth = 0): boolean {
  if (depth > 12) return true;
  if (Array.isArray(value)) return value.some((v) => hasSensitiveInput(v, depth + 1));
  if (!value || typeof value !== "object") return false;
  for (const [key, child] of Object.entries(value as Record<string, unknown>)) {
    if (/(cookie|password|passwd|credential|authorization|access.?token|refresh.?token|secret|credit.?card|cvv|cvc|one.?time.?code|otp)/i.test(key)) return true;
    if (/(^|_)(code|script)(_|$)/i.test(key)) return true;
    if (hasSensitiveInput(child, depth + 1)) return true;
  }
  return false;
}
function forbiddenTool(name: string): boolean {
  return /(cookie|password|credential|secret|storage|local.?storage|session.?storage|evaluate.?in.?page|(^|_)run($|_)|fill|type|submit|upload|payment|checkout|purchase|delete|remove|publish|send)/i.test(name);
}
function hasConsequentialInput(value: unknown, depth = 0): boolean {
  if (depth > 12) return true;
  if (Array.isArray(value)) return value.some((v) => hasConsequentialInput(v, depth + 1));
  if (!value || typeof value !== "object") return false;
  for (const [key, child] of Object.entries(value as Record<string, unknown>)) {
    if (typeof child === "string" && /(buy now|pay now|purchase|checkout|submit|send message|send email|publish|delete|remove account|confirm payment)/i.test(child)) return true;
    if (hasConsequentialInput(child, depth + 1)) return true;
  }
  return false;
}
async function signAccessToken(env: Env, claims: AccessClaims): Promise<string> {
  const header = b64url(encoder.encode(JSON.stringify({ alg: "HS256", typ: "JWT" })));
  const payload = b64url(encoder.encode(JSON.stringify(claims)));
  const input = header + "." + payload;
  return input + "." + await hmac(env.TOKEN_SIGNING_KEY, input);
}
async function verifyAccessToken(env: Env, token: string): Promise<AccessClaims | null> {
  const parts = token.split(".");
  if (parts.length !== 3 || !env.TOKEN_SIGNING_KEY) return null;
  if (!env.TOKEN_SIGNING_KEY) return null;
  try {
    const headerBytes = fromB64url(parts[0]);
    const payloadBytes = fromB64url(parts[1]);
    if (!headerBytes || !payloadBytes) return null;
    const header = JSON.parse(new TextDecoder().decode(headerBytes));
    if (header?.alg !== "HS256" || header?.typ !== "JWT") return null;
    const expected = await hmac(env.TOKEN_SIGNING_KEY, parts[0] + "." + parts[1]);
    if (!safeEqual(expected, parts[2])) return null;
    const claims = JSON.parse(new TextDecoder().decode(payloadBytes)) as AccessClaims;
    const now = Math.floor(Date.now() / 1000);
    if (claims.iss !== env.GATEWAY_ISSUER || claims.aud !== env.GATEWAY_ISSUER + "/mcp" || !Number.isSafeInteger(claims.iat) || !Number.isSafeInteger(claims.exp) || claims.iat > now + 60 || claims.exp <= now || claims.exp - claims.iat > 3600) return null;
    if (!claims.device_id || !claims.scope || !claims.sub) return null;
    return claims;
  } catch { return null; }
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const id = env.GATEWAY_STATE.idFromName("browseros-chatgpt-gateway");
    return env.GATEWAY_STATE.get(id).fetch(request);
  }
};

export class GatewayState {
  private ctx: DurableObjectState;
  private env: Env;
  constructor(ctx: DurableObjectState, env: Env) { this.ctx = ctx; this.env = env; }

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    if (request.method === "GET" && url.pathname === "/health") {
      return json({ service: "browseros-chatgpt-remote-gateway", status: "ok", pairing: "required", toolCalls: "allowlist-only" });
    }
    if (request.method === "GET" && (url.pathname === "/.well-known/oauth-authorization-server" || url.pathname === "/.well-known/oauth-protected-resource" || url.pathname === "/mcp/.well-known/oauth-protected-resource")) {
      const issuer = this.env.GATEWAY_ISSUER;
      if (url.pathname.endsWith("oauth-authorization-server")) {
        return json({
          issuer, authorization_endpoint: issuer + "/authorize", token_endpoint: issuer + "/token",
          registration_endpoint: issuer + "/register", response_types_supported: ["code"],
          grant_types_supported: ["authorization_code"], code_challenge_methods_supported: ["S256"],
          token_endpoint_auth_methods_supported: ["none"], scopes_supported: ["browser:read", "browser:control"]
        });
      }
      return json({ resource: issuer + "/mcp", authorization_servers: [issuer], bearer_methods_supported: ["header"], scopes_supported: ["browser:read", "browser:control"] });
    }
    if (request.method === "POST" && url.pathname === "/register") return this.registerClient(request);
    if (request.method === "POST" && url.pathname === "/pair/start") return this.startPairing(request);
    if (request.method === "POST" && url.pathname === "/pair/status") return this.pairingStatus(request);
    if (request.method === "POST" && url.pathname === "/pair/revoke") return this.revokePairing(request);
    if (request.method === "GET" && url.pathname === "/authorize") {
      const limited = await this.rateLimit(request, "authorize-page", 30, 3600000);
      if (limited) return limited;
      return this.authorizePage(url);
    }
    if (request.method === "POST" && url.pathname === "/authorize") return this.authorize(request);
    if (request.method === "POST" && url.pathname === "/token") return this.token(request);
    if (request.method === "POST" && url.pathname === "/mcp") return this.mcp(request);
    return json({ error: "not_found" }, 404);
  }

  private async registerClient(request: Request): Promise<Response> {
    const limited = await this.rateLimit(request, "register", 30, 3600000);
    if (limited) return limited;
    const body = await request.json().catch(() => null) as any;
    const uris = body?.redirect_uris;
    if (!Array.isArray(uris) || uris.length < 1 || uris.length > 10 || !uris.every((u: unknown) => typeof u === "string" && this.validRedirect(u))) {
      return json({ error: "invalid_redirect_uri" }, 400);
    }
    const clientId = randomId(24);
    await this.ctx.storage.put("client:" + clientId, { clientId, redirectUris: uris, createdAt: Date.now() } satisfies Client);
    return json({ client_id: clientId, redirect_uris: uris, token_endpoint_auth_method: "none", grant_types: ["authorization_code"], response_types: ["code"] }, 201);
  }

  private validRedirect(value: string): boolean {
    try {
      const u = new URL(value);
      return u.protocol === "https:" || ((u.hostname === "localhost" || u.hostname === "127.0.0.1" || u.hostname === "[::1]") && (u.protocol === "http:" || u.protocol === "https:"));
    } catch { return false; }
  }

  private async rateLimit(request: Request, action: string, limit: number, windowMs: number): Promise<Response | null> {
    const ip = request.headers.get("cf-connecting-ip") || "unknown";
    const now = Date.now();
    const key = "rate:" + action + ":" + await sha256(ip);
    const retryAfter = await this.ctx.storage.transaction(async (txn) => {
      const previous = await txn.get<{ count: number; resetAt: number }>(key);
      const current = !previous || previous.resetAt <= now ? { count: 0, resetAt: now + windowMs } : previous;
      if (current.count >= limit) return Math.max(1, Math.ceil((current.resetAt - now) / 1000));
      await txn.put(key, { count: current.count + 1, resetAt: current.resetAt });
      return 0;
    });
    if (retryAfter > 0) return new Response(JSON.stringify({ error: "rate_limited", retry_after: retryAfter }), { status: 429, headers: { "content-type": "application/json", "cache-control": "no-store", "retry-after": String(retryAfter) } });
    return null;
  }

  private async startPairing(request: Request): Promise<Response> {
    const limited = await this.rateLimit(request, "pair-start", 8, 3600000);
    if (limited) return limited;
    const body = await request.json().catch(() => null) as any;
    const publicKey = typeof body?.public_key === "string" ? body.public_key : "";
    const keyBytes = fromB64url(publicKey);
    if (!keyBytes || keyBytes.length !== 32) return json({ error: "invalid_public_key" }, 400);
    const label = typeof body.label === "string" ? body.label.trim().slice(0, 48) : "BrowserOS device";
    const deviceId = randomId(18);
    const transactionId = randomId(24);
    const code = randomId(9).replace(/[-_]/g, "A").toUpperCase();
    const challenge = randomId(32);
    const createdAt = Date.now();
    const expiresAt = createdAt + 300000;
    const pairing: Pairing = { deviceId, transactionId, publicKey, challenge, label, createdAt, expiresAt, paired: false };
    const transaction: PairTransaction = { transactionId, deviceId, publicKey, challenge, state: "pending", createdAt, expiresAt };
    await this.ctx.storage.put("pair:" + await sha256(code), pairing);
    await this.ctx.storage.put("pairtx:" + transactionId, transaction);
    return json({ device_id: deviceId, transaction_id: transactionId, pairing_code: code, challenge, expires_in: 300, signature_format: "browseros-pair-v1\\nstatus\\n{transaction_id}\\n{challenge}\\n{unix_timestamp}" }, 201);
  }

  private async pairingStatus(request: Request): Promise<Response> {
    const limited = await this.rateLimit(request, "pair-status", 60, 60000);
    if (limited) return limited;
    const body = await request.json().catch(() => null) as any;
    const transactionId = typeof body?.transaction_id === "string" ? body.transaction_id : "";
    const timestamp = Number(body?.timestamp);
    const signature = typeof body?.signature === "string" ? body.signature : "";
    if (!transactionId || !Number.isSafeInteger(timestamp) || Math.abs(Date.now() - timestamp * 1000) > 300000) return json({ error: "invalid_request" }, 400);
    const transaction = await this.ctx.storage.get<PairTransaction>("pairtx:" + transactionId);
    if (!transaction || transaction.expiresAt < Date.now()) return json({ error: "pairing_expired" }, 410);
    const message = ["browseros-pair-v1", "status", transactionId, transaction.challenge, String(timestamp)].join("\n");
    if (!await verifyEd25519(transaction.publicKey, signature, message)) return json({ error: "invalid_device_proof" }, 401);
    if (transaction.state === "pending") return json({ state: "pending", expires_at: transaction.expiresAt });
    if (transaction.state === "revoked") return json({ state: "revoked" }, 403);
    const device = await this.ctx.storage.get<any>("device:" + transaction.deviceId);
    if (!device || device.revoked) return json({ state: "revoked" }, 403);
    return json({ state: "paired", device_id: transaction.deviceId, paired_at: device.pairedAt });
  }

  private async revokePairing(request: Request): Promise<Response> {
    const limited = await this.rateLimit(request, "pair-revoke", 12, 3600000);
    if (limited) return limited;
    const body = await request.json().catch(() => null) as any;
    const deviceId = typeof body?.device_id === "string" ? body.device_id : "";
    const timestamp = Number(body?.timestamp);
    const nonce = typeof body?.nonce === "string" ? body.nonce : "";
    const signature = typeof body?.signature === "string" ? body.signature : "";
    if (!deviceId || !/^[A-Za-z0-9_-]{16,64}$/.test(nonce) || !Number.isSafeInteger(timestamp) || Math.abs(Date.now() - timestamp * 1000) > 300000) return json({ error: "invalid_request" }, 400);
    const key = "device:" + deviceId;
    const device = await this.ctx.storage.get<any>(key);
    if (!device) return json({ error: "device_not_found" }, 404);
    if (device.revoked) return json({ revoked: true });
    const message = ["browseros-pair-v1", "revoke", deviceId, nonce, String(timestamp)].join("\n");
    if (!await verifyEd25519(device.publicKey, signature, message)) return json({ error: "invalid_device_proof" }, 401);
    const nonceKey = "revokenonce:" + deviceId + ":" + nonce;
    if (await this.ctx.storage.get<boolean>(nonceKey)) return json({ error: "replayed_request" }, 409);
    await this.ctx.storage.put(nonceKey, true, { expirationTtl: 600 });
    device.revoked = true;
    device.revokedAt = Date.now();
    await this.ctx.storage.put(key, device);
    const tx = await this.ctx.storage.get<PairTransaction>("pairtx:" + device.transactionId);
    if (tx) { tx.state = "revoked"; await this.ctx.storage.put("pairtx:" + tx.transactionId, tx); }
    return json({ revoked: true });
  }

  private async authorizePage(url: URL): Promise<Response> {
    const clientId = url.searchParams.get("client_id") || "";
    const redirectUri = url.searchParams.get("redirect_uri") || "";
    const state = url.searchParams.get("state") || "";
    const challenge = url.searchParams.get("code_challenge") || "";
    const method = url.searchParams.get("code_challenge_method") || "";
    const scope = url.searchParams.get("scope") || "browser:read";
    const client = await this.ctx.storage.get<Client>("client:" + clientId);
    if (!client || !client.redirectUris.includes(redirectUri) || !this.validRedirect(redirectUri) || !state || !/^[A-Za-z0-9_-]{43}$/.test(challenge) || method !== "S256") {
      return json({ error: "invalid_request" }, 400);
    }
    const safeScope = scope.split(/\s+/).filter((s) => ["browser:read", "browser:control"].includes(s));
    if (!safeScope.includes("browser:read")) safeScope.push("browser:read");
    const transaction = randomId(24);
    await this.ctx.storage.put("authreq:" + await sha256(transaction), {
      clientId, redirectUri, state, challenge, scope: safeScope.join(" "), expiresAt: Date.now() + 10 * 60 * 1000
    } satisfies AuthRequest);
    const hidden = '<input type="hidden" name="transaction" value="' + html(transaction) + '">';
    const scopeLabel = safeScope.join(", ");
    const page = '<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>Pair BrowserOS</title><style>body{font:16px system-ui;max-width:520px;margin:12vh auto;padding:24px;background:#090909;color:#f5f5f5}form{display:grid;gap:14px}input,button{font:inherit;padding:12px;border:1px solid #555;border-radius:0;background:#111;color:#fff}button{background:#fff;color:#000;font-weight:700}small{color:#aaa}</style></head><body><h1>Connect ChatGPT to BrowserOS</h1><p>Enter the one-time pairing code displayed by your local BrowserOS app. The code proves control of that local device.</p><form method="post" action="/authorize">' + hidden + '<label>BrowserOS pairing code<input name="pairing_code" autocomplete="off" required minlength="8" maxlength="24"></label><label><input type="checkbox" name="approve" value="yes" required> I approve this ChatGPT connection.</label><p>Requested permissions: ' + html(scopeLabel) + '</p><p><small>Browser cookies, passwords and browser login tokens are not requested or transferred by this gateway. MCP page content may still be visible to ChatGPT.</small></p><button type="submit">Approve connection</button></form></body></html>';
    return noStoreHtml(page);
  }

  private async authorize(request: Request): Promise<Response> {
    const limited = await this.rateLimit(request, "authorize", 20, 3600000);
    if (limited) return limited;
    const form = await request.formData();
    const transaction = String(form.get("transaction") || "");
    const txKey = "authreq:" + await sha256(transaction);
    const authRequest = await this.ctx.storage.get<AuthRequest>(txKey);
    if (!authRequest || authRequest.expiresAt < Date.now()) return json({ error: "invalid_request" }, 400);
    await this.ctx.storage.delete(txKey);
    const codeInput = String(form.get("pairing_code") || "").trim().toUpperCase();
    if (form.get("approve") !== "yes" || !codeInput) return json({ error: "access_denied" }, 400);
    const client = await this.ctx.storage.get<Client>("client:" + authRequest.clientId);
    if (!client || !client.redirectUris.includes(authRequest.redirectUri)) return json({ error: "invalid_client" }, 400);
    const pairKey = "pair:" + await sha256(codeInput);
    const pairing = await this.ctx.storage.get<Pairing>(pairKey);
    if (!pairing || pairing.expiresAt < Date.now() || pairing.paired) return json({ error: "invalid_pairing_code" }, 400);
    pairing.paired = true;
    const pairedAt = Date.now();
    await this.ctx.storage.put("device:" + pairing.deviceId, { deviceId: pairing.deviceId, transactionId: pairing.transactionId, publicKey: pairing.publicKey, label: pairing.label, pairedAt, revoked: false });
    const pairTransaction = await this.ctx.storage.get<PairTransaction>("pairtx:" + pairing.transactionId);
    if (pairTransaction) { pairTransaction.state = "paired"; await this.ctx.storage.put("pairtx:" + pairing.transactionId, pairTransaction); }
    await this.ctx.storage.delete(pairKey);
    const authCode = randomId(32);
    await this.ctx.storage.put("code:" + await sha256(authCode), {
      clientId: authRequest.clientId, redirectUri: authRequest.redirectUri, challenge: authRequest.challenge,
      deviceId: pairing.deviceId, scope: authRequest.scope, expiresAt: Date.now() + 5 * 60 * 1000
    } satisfies AuthCode);
    const target = new URL(authRequest.redirectUri);
    target.searchParams.set("code", authCode);
    target.searchParams.set("state", authRequest.state);
    return Response.redirect(target.toString(), 302);
  }

  private async token(request: Request): Promise<Response> {
    const limited = await this.rateLimit(request, "token", 60, 3600000);
    if (limited) return limited;
    if (!this.env.TOKEN_SIGNING_KEY || this.env.TOKEN_SIGNING_KEY.length < 43) return json({ error: "gateway_not_configured" }, 503);
    const form = await request.formData().catch(() => null);
    if (!form) return json({ error: "invalid_request" }, 400);
    const grantType = String(form.get("grant_type") || "");
    const clientId = String(form.get("client_id") || "");
    const code = String(form.get("code") || "");
    const redirectUri = String(form.get("redirect_uri") || "");
    const verifier = String(form.get("code_verifier") || "");
    if (grantType !== "authorization_code" || verifier.length < 43 || verifier.length > 128) return json({ error: "unsupported_grant_type" }, 400);
    const recordKey = "code:" + await sha256(code);
    const record = await this.ctx.storage.get<AuthCode>(recordKey);
    if (!record || record.expiresAt < Date.now() || record.clientId !== clientId || record.redirectUri !== redirectUri || !safeEqual(await sha256(verifier), record.challenge)) {
      return json({ error: "invalid_grant" }, 400);
    }
    await this.ctx.storage.delete(recordKey);
    const device = await this.ctx.storage.get<any>("device:" + record.deviceId);
    if (!device || device.revoked) return json({ error: "invalid_grant" }, 400);
    const now = Math.floor(Date.now() / 1000);
    const claims: AccessClaims = { iss: this.env.GATEWAY_ISSUER, aud: this.env.GATEWAY_ISSUER + "/mcp", sub: clientId, device_id: record.deviceId, scope: record.scope, iat: now, exp: now + 3600 };
    const accessToken = await signAccessToken(this.env, claims);
    return json({ access_token: accessToken, token_type: "Bearer", expires_in: 3600, scope: record.scope });
  }

  private async mcp(request: Request): Promise<Response> {
    const auth = request.headers.get("authorization") || "";
    const token = auth.startsWith("Bearer ") ? auth.slice(7) : "";
    const claims = token ? await verifyAccessToken(this.env, token) : null;
    if (!claims || !claims.scope.split(/\s+/).includes("browser:read")) return json({ error: "unauthorized" }, 401);
    const device = await this.ctx.storage.get<any>("device:" + claims.device_id);
    if (!device || device.revoked) return json({ error: "device_not_paired" }, 403);
    if (!this.env.BROWSEROS_MCP_ORIGIN || !this.env.CF_ACCESS_CLIENT_ID || !this.env.CF_ACCESS_CLIENT_SECRET) return json({ error: "gateway_not_configured" }, 503);
    const raw = await request.text();
    if (raw.length > 1_000_000) return json({ error: "request_too_large" }, 413);
    let message: any;
    try { message = JSON.parse(raw); } catch { return json({ error: "invalid_json" }, 400); }
    const method = message?.method;
    if (!["initialize", "notifications/initialized", "ping", "tools/list", "tools/call"].includes(method)) return json({ error: "method_not_allowed" }, 403);
    const tools = allowedTools(this.env);
    if (method === "tools/call") {
      if (!claims.scope.split(/\s+/).includes("browser:control")) return json({ error: "insufficient_scope" }, 403);
      const name = message?.params?.name;
      if (typeof name !== "string" || !tools.has(name) || forbiddenTool(name) || hasSensitiveInput(message?.params?.arguments) || hasConsequentialInput(message?.params?.arguments)) {
        return json({ error: "tool_not_allowed", message: "Tool is not allowlisted or its arguments contain a blocked sensitive field." }, 403);
      }
    }
    const upstream = new URL("/mcp", this.env.BROWSEROS_MCP_ORIGIN);
    const headers = new Headers({
      "content-type": "application/json",
      "accept": request.headers.get("accept") || "application/json, text/event-stream",
      "cf-access-client-id": this.env.CF_ACCESS_CLIENT_ID,
      "cf-access-client-secret": this.env.CF_ACCESS_CLIENT_SECRET
    });
    for (const key of ["mcp-protocol-version", "mcp-session-id", "last-event-id"]) {
      const value = request.headers.get(key);
      if (value) headers.set(key, value);
    }
    const upstreamResponse = await fetch(upstream.toString(), { method: "POST", headers, body: raw, redirect: "manual" }).catch(() => null);
    if (!upstreamResponse) return json({ error: "browseros_unavailable" }, 502);
    if (upstreamResponse.status >= 300 && upstreamResponse.status < 400) return json({ error: "upstream_redirect_blocked" }, 502);
    const responseHeaders = new Headers();
    for (const key of ["content-type", "mcp-session-id", "mcp-protocol-version", "cache-control"]) {
      const value = upstreamResponse.headers.get(key);
      if (value) responseHeaders.set(key, value);
    }
    responseHeaders.set("cache-control", "no-store");
    responseHeaders.set("x-content-type-options", "nosniff");
    if (method === "tools/list" && upstreamResponse.ok) {
      const contentType = upstreamResponse.headers.get("content-type") || "";
      if (contentType.includes("application/json")) {
        const payload: any = await upstreamResponse.json();
        if (payload?.result?.tools && Array.isArray(payload.result.tools)) {
          payload.result.tools = payload.result.tools.filter((tool: any) => typeof tool?.name === "string" && tools.has(tool.name) && !forbiddenTool(tool.name));
          return new Response(JSON.stringify(payload), { status: upstreamResponse.status, headers: responseHeaders });
        }
      }
      return json({ error: "unexpected_upstream_response" }, 502);
    }
    return new Response(upstreamResponse.body, { status: upstreamResponse.status, headers: responseHeaders });
  }
}
