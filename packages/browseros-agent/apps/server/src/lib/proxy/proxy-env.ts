/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Standard proxy environment variable parsing.
 *
 * Honors HTTP_PROXY/HTTPS_PROXY/NO_PROXY (either case). Bun's fetch already
 * honors these for direct fetch calls, but the assistant resolves them
 * explicitly so system-proxy settings and env vars share one deterministic
 * precedence chain (env wins) and one bypass matcher.
 */

export interface ProxyEnv {
  httpProxy: string | undefined
  httpsProxy: string | undefined
  noProxy: string[]
}

/** Trim, drop empties, and default bare host:port values to http://. */
export function normalizeProxyUrl(
  value: string | undefined,
): string | undefined {
  if (!value) return undefined
  const trimmed = value.trim()
  if (!trimmed) return undefined
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(trimmed)) return trimmed
  return `http://${trimmed}`
}

export function parseNoProxy(value: string | undefined): string[] {
  if (!value) return []
  return value
    .split(',')
    .map((entry) => entry.trim())
    .filter((entry) => entry.length > 0)
}

export function readProxyEnv(env: NodeJS.ProcessEnv = process.env): ProxyEnv {
  return {
    httpProxy: normalizeProxyUrl(env.HTTP_PROXY ?? env.http_proxy),
    httpsProxy: normalizeProxyUrl(env.HTTPS_PROXY ?? env.https_proxy),
    noProxy: parseNoProxy(env.NO_PROXY ?? env.no_proxy),
  }
}
