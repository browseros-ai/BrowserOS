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
 *   1. BrowserOS `browseros.server.server_port` pref
 *   2. ?apiUrl=… on window.location (dev launcher publishes this)
 *   3. sessionStorage cache of (2)
 *   4. VITE_BROWSEROS_CLAW_API_URL from the dev watcher
 *   5. standalone BrowserClaw port on 127.0.0.1
 *
 * The lazy Proxy awaits the base URL at the terminal `$get`/`$post`
 * call so BrowserOS's callback-based pref API can feed Hono without
 * changing call sites.
 */

import type { AppType } from '@browseros/claw-server/server'
import { hc } from 'hono/client'
import {
  apiBaseUrlSourcesFromWindow,
  resolveBrowserOSServerBaseUrl,
} from './browseros-ports'
import { resolveApiBaseUrlFromSources } from './client.helpers'

/**
 * Public helper for surfaces that need to embed the resolved base
 * URL directly (eg. an `<img src>` to a streamed screenshot route)
 * rather than going through the hc-proxied JSON client. Uses the
 * same resolution chain as the rpc client.
 */
export function apiBaseUrl(): string {
  return resolveApiBaseUrlFromSources(apiBaseUrlSourcesFromWindow())
}

export async function resolveApiBaseUrl(): Promise<string> {
  return resolveBrowserOSServerBaseUrl(apiBaseUrlSourcesFromWindow())
}

type ApiClient = ReturnType<typeof hc<AppType>>

let cachedBase: string | null = null
let cachedClient: ApiClient | null = null

async function getApiClient(): Promise<ApiClient> {
  const base = await resolveApiBaseUrl()
  if (base !== cachedBase || !cachedClient) {
    cachedBase = base
    cachedClient = hc<AppType>(base)
  }
  return cachedClient
}

/** Creates a Hono-compatible proxy that awaits BrowserOS port resolution at request time. */
function apiProxyFor(path: PropertyKey[]): unknown {
  return new Proxy(
    {},
    {
      get(_target, prop) {
        if (prop === 'then') return undefined
        if (typeof prop === 'string' && prop.startsWith('$')) {
          return async (...args: unknown[]) => {
            const client = await getApiClient()
            let node: unknown = client
            for (const key of path) {
              node = (node as Record<PropertyKey, unknown>)[key]
            }
            const method = (node as Record<PropertyKey, unknown>)[prop]
            if (typeof method !== 'function') {
              throw new Error(`Unknown API method: ${String(prop)}`)
            }
            return method(...args)
          }
        }
        return apiProxyFor([...path, prop])
      },
    },
  )
}

export const api = apiProxyFor([]) as ApiClient
