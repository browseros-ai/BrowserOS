/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * hono-rpc client factory + lazy Proxy.
 *
 * AppType is imported as a type-only symbol from
 * @browseros/claw-server/server. That export field has
 * `"default": null` so a runtime import would fail at build time;
 * we only get the type at compile time, the runtime calls go over
 * HTTP loopback to whichever port the claw server bound to.
 *
 * Resolution order for the base URL:
 *   1. ?apiUrl=… on window.location (dev launcher publishes this)
 *   2. sessionStorage cache of (1)
 *   3. VITE_BROWSEROS_CLAW_API_URL from the dev watcher
 *   4. standalone BrowserClaw port on 127.0.0.1
 *
 * The lazy Proxy is what lets us re-resolve the base URL after the
 * dev launcher hot-swaps it without breaking hc's path chaining
 * (hc returns its own Proxy; ours just forwards each property read).
 */

import type { AppType } from '@browseros/claw-server/server'
import {
  CLAW_API_PORT_DEFAULT,
  COCKPIT_MOUNT_PREFIX,
} from '@browseros/claw-server/shared/port'
import { hc } from 'hono/client'
import {
  API_URL_STORAGE_KEY,
  isLoopbackCockpitUrl,
  resolveApiBaseUrlFromSources,
} from './client.helpers'

function resolveApiBaseUrl(): string {
  const fallback = `http://127.0.0.1:${CLAW_API_PORT_DEFAULT}${COCKPIT_MOUNT_PREFIX}`
  if (typeof window === 'undefined') return fallback

  const fromQuery = new URLSearchParams(window.location.search).get('apiUrl')
  if (isLoopbackCockpitUrl(fromQuery)) {
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
    return resolveApiBaseUrlFromSources({
      query: null,
      stored,
      launcher: import.meta.env.VITE_BROWSEROS_CLAW_API_URL,
      fallback,
    })
  } catch {
    return resolveApiBaseUrlFromSources({
      query: null,
      stored: null,
      launcher: import.meta.env.VITE_BROWSEROS_CLAW_API_URL,
      fallback,
    })
  }
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
