/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { cors } from 'hono/cors'

type CorsOptions = Parameters<typeof cors>[0]

/**
 * Default CORS configuration for the HTTP server.
 *
 * The agent server binds to a localhost port and is reachable
 * from any tab running in the user's browser. With a wildcard
 * `Access-Control-Allow-Origin`, every page on the open
 * internet can issue cross-origin fetches and read responses.
 * Restrict to an explicit allowlist composed of:
 *
 *   1. The published BrowserOS extension origin (always
 *      allowed).
 *   2. Additional origins supplied via the
 *      `BROWSEROS_TRUSTED_ORIGINS` env var, comma-separated —
 *      used by dev (the WXT dev extension origin), unsigned
 *      builds, and internal alpha extensions.
 *
 * Reflecting any unknown origin is intentionally not done.
 * Callers without an `Origin` header (CLI tools, internal
 * Node clients) are unaffected — `cors()` only acts when an
 * `Origin` header is present, and the `requireTrustedOrigin`
 * middleware mirrors that behaviour at the reject side.
 */

/**
 * Static, hard-coded allowlist. Empty by default — every
 * deployment supplies its trusted origins via env. This keeps
 * the source agnostic of any specific build/extension id.
 */
const STATIC_ALLOWED_ORIGINS = new Set<string>()

let cachedAllowedOrigins: Set<string> | null = null

function buildAllowedOrigins(): Set<string> {
  const fromEnv = (process.env.BROWSEROS_TRUSTED_ORIGINS ?? '')
    .split(',')
    .map((value) => value.trim())
    .filter((value) => value.length > 0)
  return new Set([...STATIC_ALLOWED_ORIGINS, ...fromEnv])
}

function getAllowedOrigins(): Set<string> {
  if (!cachedAllowedOrigins) {
    cachedAllowedOrigins = buildAllowedOrigins()
  }
  return cachedAllowedOrigins
}

/** Test-only: drop the cached set so tests can re-read env. */
export function resetAllowedOriginsForTesting(): void {
  cachedAllowedOrigins = null
}

/**
 * Returns true when `origin` is in the allowlist. Case-
 * sensitive, exact match — origins are normalised by the
 * browser before they reach us, and `chrome-extension://` is
 * lowercase by spec.
 */
export function isAllowedOrigin(origin: string): boolean {
  return getAllowedOrigins().has(origin)
}

export const defaultCorsConfig: CorsOptions = {
  origin: (origin: string | undefined) => {
    if (origin && isAllowedOrigin(origin)) return origin
    return null
  },
  allowMethods: ['GET', 'POST', 'DELETE', 'OPTIONS'],
  allowHeaders: ['Content-Type', 'Authorization', 'Accept'],
  credentials: true,
}
