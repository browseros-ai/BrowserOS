/**
 * Typed HTTP driver for cross-server conformance. It deliberately owns its
 * route mapping so the suite checks both servers against the wire contract
 * without sharing claw-app transport code.
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
  SessionStatus,
  ShutdownResponse,
  SystemInfo,
  TelemetryState,
  UpdateTelemetryRequest,
} from '../../../packages/claw-api/src'

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

export class ContractHttpError extends Error {
  override name = 'ContractHttpError'

  constructor(public readonly response: Response) {
    super(`contract request failed with status ${response.status.toString()}`)
  }
}

export class ContractHttpClient {
  constructor(private readonly baseUrl: string) {}

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

  getSessionBrowserTabPreview(request: {
    sessionId: string
    browserTabId: number
  }): Promise<Blob> {
    return this.blob(
      `/api/v1/sessions/${pathPart(request.sessionId)}/browser-tabs/${request.browserTabId.toString()}/preview`,
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

  private async blob(path: string, init?: RequestInit): Promise<Blob> {
    return (await this.request(path, init)).blob()
  }

  private async request(
    path: string,
    init: RequestInit = {},
  ): Promise<Response> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      ...init,
      method: init.method ?? 'GET',
      credentials: 'omit',
      headers: new Headers(init.headers),
    })
    if (!response.ok) throw new ContractHttpError(response)
    return response
  }
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
