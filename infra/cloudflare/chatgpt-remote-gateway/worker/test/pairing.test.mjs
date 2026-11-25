import test from "node:test";
import assert from "node:assert/strict";
import { GatewayState } from "../src/index.ts";

const encoder = new TextEncoder();
const b64url = (bytes) => Buffer.from(bytes).toString("base64url");
const headers = { "cf-connecting-ip": "192.0.2.10" };

class MemoryStorage {
  values = new Map();
  async get(key) {
    const value = this.values.get(key);
    return value === undefined ? undefined : structuredClone(value);
  }
  async put(key, value) {
    this.values.set(key, structuredClone(value));
  }
  async delete(key) {
    return this.values.delete(key);
  }
  async transaction(callback) {
    return callback({
      get: async (key) => this.get(key),
      put: async (key, value, _options) => this.put(key, value),
      delete: async (key) => this.delete(key)
    });
  }
}

function gateway() {
  const storage = new MemoryStorage();
  const env = {
    GATEWAY_ISSUER: "https://gateway.example",
    BROWSEROS_MCP_ORIGIN: "",
    ALLOWED_TOOL_NAMES: "[]",
    TOKEN_SIGNING_KEY: "test-signing-key-that-is-long-enough-for-local-tests",
    CF_ACCESS_CLIENT_ID: "",
    CF_ACCESS_CLIENT_SECRET: ""
  };
  return { app: new GatewayState({ storage }, env), storage };
}

async function ed25519Key() {
  const keys = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
  const publicKey = b64url(new Uint8Array(await crypto.subtle.exportKey("raw", keys.publicKey)));
  return { ...keys, publicKey };
}

async function signature(privateKey, message) {
  return b64url(new Uint8Array(await crypto.subtle.sign("Ed25519", privateKey, encoder.encode(message))));
}

async function startPairing(app, publicKey) {
  const response = await app.fetch(new Request("https://gateway.example/pair/start", {
    method: "POST", headers: { ...headers, "content-type": "application/json" },
    body: JSON.stringify({ public_key: publicKey, label: "test desktop" })
  }));
  return { response, payload: await response.json() };
}

test("pair/start rejects malformed Ed25519 public keys", async () => {
  const { app } = gateway();
  const { response, payload } = await startPairing(app, "not-a-public-key");
  assert.equal(response.status, 400);
  assert.equal(payload.error, "invalid_public_key");
});

test("pairing requires signed status proof and a one-time approval code", async () => {
  const { app } = gateway();
  const keys = await ed25519Key();
  const { response: startResponse, payload } = await startPairing(app, keys.publicKey);
  assert.equal(startResponse.status, 201);
  assert.equal(payload.expires_in, 300);
  assert.equal(typeof payload.pairing_code, "string");
  assert.match(payload.pairing_code, /^[0-9A-HJKMNP-TV-Z]{12}$/);

  const timestamp = Math.floor(Date.now() / 1000);
  const message = ["browseros-pair-v1", "status", payload.transaction_id, payload.challenge, String(timestamp)].join("\n");
  const invalid = await app.fetch(new Request("https://gateway.example/pair/status", {
    method: "POST", headers: { ...headers, "content-type": "application/json" },
    body: JSON.stringify({ transaction_id: payload.transaction_id, timestamp, signature: b64url(new Uint8Array(64)) })
  }));
  assert.equal(invalid.status, 401);

  const validSignature = await signature(keys.privateKey, message);
  const pending = await app.fetch(new Request("https://gateway.example/pair/status", {
    method: "POST", headers: { ...headers, "content-type": "application/json" },
    body: JSON.stringify({ transaction_id: payload.transaction_id, timestamp, signature: validSignature })
  }));
  assert.equal(pending.status, 200);
  assert.equal((await pending.json()).state, "pending");

  const verifier = b64url(crypto.getRandomValues(new Uint8Array(32)));
  const challenge = b64url(new Uint8Array(await crypto.subtle.digest("SHA-256", encoder.encode(verifier))));
  const register = await app.fetch(new Request("https://gateway.example/register", {
    method: "POST", headers: { ...headers, "content-type": "application/json" },
    body: JSON.stringify({ redirect_uris: ["https://client.example/callback"] })
  }));
  const client = await register.json();
  const authorizeUrl = new URL("https://gateway.example/authorize");
  authorizeUrl.searchParams.set("client_id", client.client_id);
  authorizeUrl.searchParams.set("redirect_uri", "https://client.example/callback");
  authorizeUrl.searchParams.set("state", "test-state");
  authorizeUrl.searchParams.set("code_challenge", challenge);
  authorizeUrl.searchParams.set("code_challenge_method", "S256");
  const page = await app.fetch(new Request(authorizeUrl, { headers }));
  const html = await page.text();
  const transaction = html.match(/name="transaction" value="([^"]+)"/)?.[1];
  assert.ok(transaction, "authorization transaction should be present");

  const approvalForm = () => {
    const form = new FormData();
    form.set("transaction", transaction);
    form.set("pairing_code", payload.pairing_code);
    form.set("approve", "yes");
    return form;
  };
  const approved = await app.fetch(new Request("https://gateway.example/authorize", {
    method: "POST", headers, body: approvalForm()
  }));
  assert.equal(approved.status, 302);

  const replayedApproval = await app.fetch(new Request("https://gateway.example/authorize", {
    method: "POST", headers, body: approvalForm()
  }));
  assert.equal(replayedApproval.status, 400);

  const pairedTimestamp = Math.floor(Date.now() / 1000);
  const pairedMessage = ["browseros-pair-v1", "status", payload.transaction_id, payload.challenge, String(pairedTimestamp)].join("\n");
  const pairedSignature = await signature(keys.privateKey, pairedMessage);
  const paired = await app.fetch(new Request("https://gateway.example/pair/status", {
    method: "POST", headers: { ...headers, "content-type": "application/json" },
    body: JSON.stringify({ transaction_id: payload.transaction_id, timestamp: pairedTimestamp, signature: pairedSignature })
  }));
  assert.equal((await paired.json()).state, "paired");

  const nonce = b64url(crypto.getRandomValues(new Uint8Array(18)));
  const revokeTimestamp = Math.floor(Date.now() / 1000);
  const revokeMessage = ["browseros-pair-v1", "revoke", payload.device_id, nonce, String(revokeTimestamp)].join("\n");
  const revokeSignature = await signature(keys.privateKey, revokeMessage);
  const revokeBody = JSON.stringify({ device_id: payload.device_id, nonce, timestamp: revokeTimestamp, signature: revokeSignature });
  const revoked = await app.fetch(new Request("https://gateway.example/pair/revoke", {
    method: "POST", headers: { ...headers, "content-type": "application/json" }, body: revokeBody
  }));
  assert.equal(revoked.status, 200);

  const replayedRevoke = await app.fetch(new Request("https://gateway.example/pair/revoke", {
    method: "POST", headers: { ...headers, "content-type": "application/json" }, body: revokeBody
  }));
  assert.equal(replayedRevoke.status, 409);
});
