/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * BrowserOS base-URL resolution and client caching for the claw-app.
 *
 * Resolution order:
 *   1. BrowserOS `browseros.server.server_port` pref
 *   2. `?apiUrl=…` on window.location (dev launcher publishes this)
 *   3. sessionStorage cache of (2)
 *   4. VITE_BROWSEROS_CLAW_API_URL from the dev watcher
 *   5. standalone BrowserClaw port on 127.0.0.1
 *
 * The BrowserOS pref is callback-based, so `apiClient()` re-resolves on every
 * call and swaps the cached client when the managed port changes.
 */

import { ClawApiClient } from '@browseros/claw-api-client'
import {
  apiBaseUrlSourcesFromWindow,
  resolveBrowserOSServerBaseUrl,
} from './browseros-ports'
import { resolveApiBaseUrlFromSources } from './client.helpers'

export type {
  ApiFetcher,
  AppendRecordingEventsRequest,
  ClawApiClientOptions,
  ListSessionsRequest,
} from '@browseros/claw-api-client'
export { ApiResponseError, ClawApiClient } from '@browseros/claw-api-client'

/** Synchronous resolution for binary URLs embedded directly in DOM attributes. */
export function apiBaseUrl(): string {
  return resolveApiBaseUrlFromSources(apiBaseUrlSourcesFromWindow())
}

export async function resolveApiBaseUrl(): Promise<string> {
  return resolveBrowserOSServerBaseUrl(apiBaseUrlSourcesFromWindow())
}

let cachedBase: string | null = null
let cachedClient: ClawApiClient | null = null

export function apiClientForBaseUrl(baseUrl: string): ClawApiClient {
  if (baseUrl !== cachedBase || !cachedClient) {
    cachedBase = baseUrl
    cachedClient = new ClawApiClient(baseUrl)
  }
  return cachedClient
}

export async function apiClient(): Promise<ClawApiClient> {
  return apiClientForBaseUrl(await resolveApiBaseUrl())
}
