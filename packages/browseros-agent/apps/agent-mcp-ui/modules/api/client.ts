/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * hono-rpc client factory + lazy Proxy.
 *
 * AppType is imported as a type-only symbol from
 * @browseros/agent-mcp-interface/server. That export field has
 * `"default": null` so a runtime import would fail at build time;
 * we only get the type at compile time, the runtime calls go over
 * HTTP loopback to whichever port the interface server bound to.
 *
 * Resolution order for the base URL:
 *   1. ?apiUrl=… on window.location (dev launcher publishes this)
 *   2. sessionStorage cache of (1)
 *   3. PROD_API_PORT constant on 127.0.0.1
 *
 * The lazy Proxy is what lets us re-resolve the base URL after the
 * dev launcher hot-swaps it without breaking hc's path chaining
 * (hc returns its own Proxy; ours just forwards each property read).
 */

import type { AppType } from '@browseros/agent-mcp-interface/server'
import {
  COCKPIT_MOUNT_PREFIX,
  PROD_API_PORT,
} from '@browseros/agent-mcp-interface/shared/port'
import { hc } from 'hono/client'

const API_URL_STORAGE_KEY = 'browseros.agent-mcp-ui.apiUrl'

function isLoopbackUrl(value: string | null | undefined): value is string {
  return !!value && value.startsWith('http://127.0.0.1:')
}

function resolveApiBaseUrl(): string {
  // The cockpit is mounted under `/cockpit` inside apps/server's
  // runtime; every route handler lives under that prefix. The dev
  // launcher's `?apiUrl=` query and the sessionStorage cache carry
  // the full base URL (port + prefix), so they're passed through
  // verbatim.
  const fallback = `http://127.0.0.1:${PROD_API_PORT}${COCKPIT_MOUNT_PREFIX}`
  if (typeof window === 'undefined') return fallback

  const fromQuery = new URLSearchParams(window.location.search).get('apiUrl')
  if (isLoopbackUrl(fromQuery)) {
    try {
      window.sessionStorage.setItem(API_URL_STORAGE_KEY, fromQuery)
    } catch {
      // sessionStorage can refuse writes in sandboxed contexts; the
      // resolved URL still serves this session.
    }
    return fromQuery
  }

  try {
    const stored = window.sessionStorage.getItem(API_URL_STORAGE_KEY)
    if (isLoopbackUrl(stored)) return stored
  } catch {
    // see above
  }

  return fallback
}

type ApiClient = ReturnType<typeof hc<AppType>>

let cachedBase: string | null = null
let cachedClient: ApiClient | null = null

function getApiClient(): ApiClient {
  const base = resolveApiBaseUrl()
  if (base !== cachedBase || !cachedClient) {
    cachedBase = base
    cachedClient = hc<AppType>(base)
  }
  return cachedClient
}

// Lazy Proxy: every property access (`api.system.health.$get`) goes
// through the freshly resolved baseUrl rather than a snapshot
// captured at module load. hc itself returns a Proxy, so we forward
// to it without a receiver override (passing an empty target would
// break hc's path chaining).
export const api = new Proxy({} as ApiClient, {
  get(_target, prop) {
    const client = getApiClient() as unknown as Record<PropertyKey, unknown>
    return client[prop]
  },
})
