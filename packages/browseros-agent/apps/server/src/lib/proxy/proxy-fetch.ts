/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Proxy-aware fetch for outbound LLM traffic.
 *
 * Wraps an inner fetch (default: globalThis.fetch) and injects Bun's native
 * `proxy` option resolved from env vars + OS system settings. Composes
 * underneath the existing provider wrappers (browseros-fetch,
 * openrouter-fetch, codex/copilot) — those spread init through, so the proxy
 * key flows to the real fetch untouched. Fails open to direct on any error.
 */

import { readProxyEnv } from './proxy-env'
import { resolveProxyForUrl } from './resolve-proxy'

/** RequestInit plus Bun's native per-request proxy option. */
type ProxiedRequestInit = RequestInit & { proxy?: string }

export interface ProxiedFetchOptions {
  resolve?: (targetUrl: string) => Promise<string | undefined>
  env?: NodeJS.ProcessEnv
}

function inputToUrl(input: RequestInfo | URL): string {
  if (typeof input === 'string') return input
  if (input instanceof URL) return input.toString()
  return input.url
}

function safeHostname(targetUrl: string): string | undefined {
  try {
    return new URL(targetUrl).hostname || undefined
  } catch {
    return undefined
  }
}

function envProxyConfigured(env: NodeJS.ProcessEnv): boolean {
  const { httpProxy, httpsProxy } = readProxyEnv(env)
  return Boolean(httpProxy ?? httpsProxy)
}

/**
 * Plant a DIRECT verdict into NO_PROXY so Bun's native layer honors it.
 *
 * Bun snapshots HTTP(S)_PROXY and keeps applying it to plain fetches even
 * when we omit the `proxy` option (unsetting the vars at runtime does not
 * take effect), while runtime NO_PROXY writes do. Only the concrete
 * hostname is seeded — never patterns. Note NO_PROXY also wins over a
 * future explicit `proxy` option, so seeded hosts stay direct; that matches
 * OS intent for localhost and ProxyOverride entries.
 */
function ensureEnvBypass(hostname: string): void {
  const current = process.env.NO_PROXY ?? process.env.no_proxy ?? ''
  const entries = current
    .split(',')
    .map((entry) => entry.trim())
    .filter((entry) => entry.length > 0)
  if (entries.some((entry) => entry.toLowerCase() === hostname.toLowerCase())) {
    return
  }
  entries.push(hostname)
  process.env.NO_PROXY = entries.join(',')
}

export function createProxiedFetch(
  inner: typeof fetch = globalThis.fetch,
  options: ProxiedFetchOptions = {},
): typeof fetch {
  const resolve = options.resolve ?? resolveProxyForUrl
  const env = options.env ?? process.env
  return (async (input: RequestInfo | URL, init?: RequestInit) => {
    const proxied = init as ProxiedRequestInit | undefined
    if (proxied?.proxy) return inner(input, init)
    const targetUrl = inputToUrl(input)
    let proxy: string | undefined
    try {
      proxy = await resolve(targetUrl)
    } catch {
      proxy = undefined
    }
    if (!proxy) {
      // Tests inject a fake env; only the real environment is mutated.
      if (env === process.env && envProxyConfigured(env)) {
        const hostname = safeHostname(targetUrl)
        if (hostname) ensureEnvBypass(hostname)
      }
      return inner(input, init)
    }
    return inner(input, { ...init, proxy } as RequestInit)
  }) as typeof fetch
}
