/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Handwritten BrowserClaw HTTP client for the app. OpenAPI owns the wire
 * contract and `@browseros/claw-api` owns its data shapes; this module owns
 * the endpoint-to-shape mapping used by UI hooks and the recorder.
 *
 * Base URL resolution order:
 *   1. BrowserOS `browseros.server.server_port` pref
 *   2. `?apiUrl=…` on window.location (dev launcher publishes this)
 *   3. sessionStorage cache of (2)
 *   4. VITE_BROWSEROS_CLAW_API_URL from the dev watcher
 *   5. standalone BrowserClaw port on 127.0.0.1
 *
 * The BrowserOS pref is callback-based, so `apiClient()` re-resolves on every
 * call and swaps the cached client when the managed port changes.
 */

import type {
  AppendRecordingEventsResponse,
  CancelSessionResponse,
  Connection,
  ConnectionList,
  Harness,
  HealthResponse,
  RecordingMetadata,
  SessionDetail,
  SessionList,
  SessionScreenshotList,
  SessionStatus,
  ShutdownResponse,
  SystemInfo,
  TelemetryState,
  UpdateTelemetryRequest,
} from '@browseros/claw-api'
import {
  apiBaseUrlSourcesFromWindow,
  resolveBrowserOSServerBaseUrl,
} from './browseros-ports'
import { resolveApiBaseUrlFromSources } from './client.helpers'

export type ApiFetcher = (
  input: Parameters<typeof globalThis.fetch>[0],
  init?: Parameters<typeof globalThis.fetch>[1],
) => ReturnType<typeof globalThis.fetch>

export interface ClawApiClientOptions {
  fetch?: ApiFetcher
  credentials?: RequestCredentials
}

export interface AppendRecordingEventsRequest {
  xRecordingTabId: number
  xRecordingDocumentId: string
  xRecordingBatchId: string
  body: string
  xRecordingHasGap?: boolean
}

export interface ListSessionsRequest {
  profileId?: string
  slug?: string
  status?: SessionStatus
  site?: string
  search?: string
  since?: number
  cursor?: number
  limit?: number
}

/** Non-success HTTP response; callers may parse its body as generated `ApiError`. */
export class ApiResponseError extends Error {
  override name = 'ApiResponseError'

  constructor(public readonly response: Response) {
    super(
      `BrowserClaw API request failed with status ${response.status.toString()}`,
    )
  }
}

export class ClawApiClient {
  private readonly baseUrl: string
  private readonly fetcher: ApiFetcher
  private readonly credentials: RequestCredentials

  constructor(baseUrl: string, options: ClawApiClientOptions = {}) {
    this.baseUrl = baseUrl.replace(/\/$/, '')
    this.fetcher =
      options.fetch ?? ((input, init) => globalThis.fetch(input, init))
    this.credentials = options.credentials ?? 'omit'
  }

  getHealth(): Promise<HealthResponse> {
    return this.json('/system/health')
  }

  shutdown(): Promise<ShutdownResponse> {
    return this.json('/system/shutdown', { method: 'POST' })
  }

  getSystemInfo(): Promise<SystemInfo> {
    return this.json('/api/v1/system')
  }

  getTelemetry(): Promise<TelemetryState> {
    return this.json('/api/v1/settings/telemetry')
  }

  updateTelemetry(request: {
    updateTelemetryRequest: UpdateTelemetryRequest
  }): Promise<TelemetryState> {
    return this.json('/api/v1/settings/telemetry', {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(request.updateTelemetryRequest),
    })
  }

  listSessions(request: ListSessionsRequest = {}): Promise<SessionList> {
    const query = new URLSearchParams()
    appendQuery(query, 'profileId', request.profileId)
    appendQuery(query, 'slug', request.slug)
    appendQuery(query, 'status', request.status)
    appendQuery(query, 'site', request.site)
    appendQuery(query, 'search', request.search)
    appendQuery(query, 'since', request.since)
    appendQuery(query, 'cursor', request.cursor)
    appendQuery(query, 'limit', request.limit)
    const suffix = query.size > 0 ? `?${query.toString()}` : ''
    return this.json(`/api/v1/sessions${suffix}`)
  }

  getSession(request: { sessionId: string }): Promise<SessionDetail> {
    return this.json(`/api/v1/sessions/${pathPart(request.sessionId)}`)
  }

  cancelSession(request: {
    sessionId: string
  }): Promise<CancelSessionResponse> {
    return this.json(`/api/v1/sessions/${pathPart(request.sessionId)}/cancel`, {
      method: 'POST',
    })
  }

  getRecording(request: { sessionId: string }): Promise<RecordingMetadata> {
    return this.json(
      `/api/v1/sessions/${pathPart(request.sessionId)}/recording`,
    )
  }

  downloadRecordingEvents(request: { sessionId: string }): Promise<string> {
    return this.text(
      `/api/v1/sessions/${pathPart(request.sessionId)}/recording/events`,
    )
  }

  appendRecordingEvents(
    request: AppendRecordingEventsRequest,
  ): Promise<AppendRecordingEventsResponse> {
    const headers = new Headers({
      'content-type': 'application/x-ndjson',
      'x-recording-tab-id': request.xRecordingTabId.toString(),
      'x-recording-document-id': request.xRecordingDocumentId,
      'x-recording-batch-id': request.xRecordingBatchId,
    })
    if (request.xRecordingHasGap !== undefined) {
      headers.set('x-recording-has-gap', request.xRecordingHasGap.toString())
    }
    return this.json('/api/v1/recordings/events', {
      method: 'POST',
      headers,
      body: request.body,
    })
  }

  getSessionPreview(request: { sessionId: string }): Promise<Blob> {
    return this.blob(`/api/v1/sessions/${pathPart(request.sessionId)}/preview`)
  }

  listSessionScreenshots(request: {
    sessionId: string
  }): Promise<SessionScreenshotList> {
    return this.json(
      `/api/v1/sessions/${pathPart(request.sessionId)}/screenshots`,
    )
  }

  getSessionScreenshot(request: {
    sessionId: string
    screenshotId: number
  }): Promise<Blob> {
    return this.blob(
      `/api/v1/sessions/${pathPart(request.sessionId)}/screenshots/${request.screenshotId.toString()}`,
    )
  }

  listConnections(): Promise<ConnectionList> {
    return this.json('/api/v1/connections')
  }

  connectHarness(request: { harness: Harness }): Promise<Connection> {
    return this.json(`/api/v1/connections/${pathPart(request.harness)}`, {
      method: 'PUT',
    })
  }

  disconnectHarness(request: { harness: Harness }): Promise<Connection> {
    return this.json(`/api/v1/connections/${pathPart(request.harness)}`, {
      method: 'DELETE',
    })
  }

  private async json<T>(path: string, init?: RequestInit): Promise<T> {
    return (await (await this.request(path, init)).json()) as T
  }

  private async text(path: string, init?: RequestInit): Promise<string> {
    return (await this.request(path, init)).text()
  }

  private async blob(path: string, init?: RequestInit): Promise<Blob> {
    return (await this.request(path, init)).blob()
  }

  private async request(
    path: string,
    init: RequestInit = {},
  ): Promise<Response> {
    const response = await this.fetcher(`${this.baseUrl}${path}`, {
      ...init,
      method: init.method ?? 'GET',
      credentials: init.credentials ?? this.credentials,
      headers: new Headers(init.headers),
    })
    if (!response.ok) throw new ApiResponseError(response)
    return response
  }
}

/** Synchronous resolution for binary URLs that are embedded directly in DOM attributes. */
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

function appendQuery(
  query: URLSearchParams,
  name: string,
  value: string | number | undefined,
): void {
  if (value !== undefined) query.set(name, value.toString())
}

function pathPart(value: string): string {
  return encodeURIComponent(value)
}
