import type {
  AppendRecordingEventsResponse,
  AuditCleanupResult,
  AuditRetention,
  AuditStorageState,
  CancelSessionResponse,
  CockpitStats,
  Connection,
  ConnectionList,
  HealthResponse,
  RecordingMetadata,
  SessionDetail,
  SessionList,
  SessionScreenshotList,
  SetAuditRetentionRequest,
  ShutdownResponse,
  SystemInfo,
  TelemetryState,
} from '@browseros/claw-api'
import createClient, { type Client } from 'openapi-fetch'
import { ApiResponseError } from './errors'
import type { operations, paths } from './generated/openapi'
import type {
  GetSessionPreviewRequest,
  GetSessionScreenshotRequest,
} from './urls'

export type ApiFetcher = (
  input: Parameters<typeof globalThis.fetch>[0],
  init?: Parameters<typeof globalThis.fetch>[1],
) => ReturnType<typeof globalThis.fetch>

export interface ClawApiClientOptions {
  fetch?: ApiFetcher
  credentials?: RequestCredentials
}

export type ListSessionsRequest = NonNullable<
  operations['listSessions']['parameters']['query']
>

export interface UpdateTelemetryOperationRequest {
  updateTelemetryRequest: operations['updateTelemetry']['requestBody']['content']['application/json']
}

type RecordingHeaders =
  operations['appendRecordingEvents']['parameters']['header']

export interface AppendRecordingEventsRequest {
  xRecordingTabId: RecordingHeaders['X-Recording-Tab-Id']
  xRecordingDocumentId: RecordingHeaders['X-Recording-Document-Id']
  xRecordingBatchId: RecordingHeaders['X-Recording-Batch-Id']
  xRecordingHasGap?: RecordingHeaders['X-Recording-Has-Gap']
  body: operations['appendRecordingEvents']['requestBody']['content']['application/x-ndjson']
}

export type SessionRequest = operations['getSession']['parameters']['path']
export type HarnessRequest = operations['connectHarness']['parameters']['path']

type ApiResult<T> =
  | { data: T; response: Response }
  | { error: unknown; response: Response }

export class ClawApiClient {
  private readonly client: Client<paths>
  private readonly responseClones = new WeakMap<Response, Response>()

  constructor(baseUrl: string, options: ClawApiClientOptions = {}) {
    const fetcher = options.fetch ?? globalThis.fetch
    const credentials = options.credentials ?? 'omit'
    const receiverSafeFetch = async (
      input: Parameters<typeof globalThis.fetch>[0],
      init?: Parameters<typeof globalThis.fetch>[1],
    ): Promise<Response> => {
      const response = await fetcher(input, {
        ...init,
        credentials: init?.credentials ?? credentials,
      })
      if (!response.ok) this.responseClones.set(response, response.clone())
      return response
    }
    this.client = createClient<paths>({
      baseUrl: baseUrl.replace(/\/+$/, ''),
      credentials,
      fetch: receiverSafeFetch,
    })
  }

  async getHealth(): Promise<HealthResponse> {
    return this.unwrap(await this.client.GET('/system/health'))
  }

  async shutdown(): Promise<ShutdownResponse> {
    return this.unwrap(await this.client.POST('/system/shutdown'))
  }

  async getSystemInfo(): Promise<SystemInfo> {
    return this.unwrap(await this.client.GET('/api/v1/system'))
  }

  async getCockpitStats(): Promise<CockpitStats> {
    return this.unwrap(await this.client.GET('/api/v1/cockpit/stats'))
  }

  async getTelemetry(): Promise<TelemetryState> {
    return this.unwrap(await this.client.GET('/api/v1/settings/telemetry'))
  }

  async getAuditStorage(): Promise<AuditStorageState> {
    return this.unwrap(await this.client.GET('/api/v1/audit/storage'))
  }

  async setAuditRetention(
    body: SetAuditRetentionRequest,
  ): Promise<AuditRetention> {
    return this.unwrap(
      await this.client.PUT('/api/v1/audit/retention', { body }),
    )
  }

  async runAuditCleanup(): Promise<AuditCleanupResult> {
    return this.unwrap(await this.client.POST('/api/v1/audit/cleanup'))
  }

  async updateTelemetry(
    request: UpdateTelemetryOperationRequest,
  ): Promise<TelemetryState> {
    return this.unwrap(
      await this.client.PUT('/api/v1/settings/telemetry', {
        body: request.updateTelemetryRequest,
      }),
    )
  }

  async listSessions(request: ListSessionsRequest = {}): Promise<SessionList> {
    return this.unwrap(
      await this.client.GET('/api/v1/sessions', {
        params: { query: request },
      }),
    )
  }

  async getSession(request: SessionRequest): Promise<SessionDetail> {
    return this.unwrap(
      await this.client.GET('/api/v1/sessions/{sessionId}', {
        params: { path: request },
      }),
    )
  }

  async cancelSession(request: SessionRequest): Promise<CancelSessionResponse> {
    return this.unwrap(
      await this.client.POST('/api/v1/sessions/{sessionId}/cancel', {
        params: { path: request },
      }),
    )
  }

  async getRecording(request: SessionRequest): Promise<RecordingMetadata> {
    return this.unwrap(
      await this.client.GET('/api/v1/sessions/{sessionId}/recording', {
        params: { path: request },
      }),
    )
  }

  async downloadRecordingEvents(request: SessionRequest): Promise<string> {
    return this.unwrap(
      await this.client.GET('/api/v1/sessions/{sessionId}/recording/events', {
        params: { path: request },
        parseAs: 'text',
      }),
    )
  }

  async appendRecordingEvents(
    request: AppendRecordingEventsRequest,
  ): Promise<AppendRecordingEventsResponse> {
    return this.unwrap(
      await this.client.POST('/api/v1/recordings/events', {
        params: {
          header: {
            'X-Recording-Tab-Id': request.xRecordingTabId,
            'X-Recording-Document-Id': request.xRecordingDocumentId,
            'X-Recording-Batch-Id': request.xRecordingBatchId,
            'X-Recording-Has-Gap': request.xRecordingHasGap,
          },
        },
        body: request.body,
        bodySerializer: (body) => body,
        headers: { 'content-type': 'application/x-ndjson' },
      }),
    )
  }

  async getSessionPreview(request: GetSessionPreviewRequest): Promise<Blob> {
    return this.unwrap(
      await this.client.GET('/api/v1/sessions/{sessionId}/preview', {
        params: {
          path: { sessionId: request.sessionId },
          query:
            request.refresh === undefined ? {} : { refresh: request.refresh },
        },
        parseAs: 'blob',
      }),
    )
  }

  async listSessionScreenshots(
    request: SessionRequest,
  ): Promise<SessionScreenshotList> {
    return this.unwrap(
      await this.client.GET('/api/v1/sessions/{sessionId}/screenshots', {
        params: { path: request },
      }),
    )
  }

  async getSessionScreenshot(
    request: GetSessionScreenshotRequest,
  ): Promise<Blob> {
    return this.unwrap(
      await this.client.GET(
        '/api/v1/sessions/{sessionId}/screenshots/{screenshotId}',
        {
          params: { path: request },
          parseAs: 'blob',
        },
      ),
    )
  }

  async listConnections(): Promise<ConnectionList> {
    return this.unwrap(await this.client.GET('/api/v1/connections'))
  }

  async connectHarness(request: HarnessRequest): Promise<Connection> {
    return this.unwrap(
      await this.client.PUT('/api/v1/connections/{harness}', {
        params: { path: request },
      }),
    )
  }

  async disconnectHarness(request: HarnessRequest): Promise<Connection> {
    return this.unwrap(
      await this.client.DELETE('/api/v1/connections/{harness}', {
        params: { path: request },
      }),
    )
  }

  private unwrap<T>(result: ApiResult<T | undefined>): T {
    const errorResponse = this.responseClones.get(result.response)
    this.responseClones.delete(result.response)
    if ('error' in result) {
      throw new ApiResponseError(errorResponse ?? result.response)
    }
    if (result.data === undefined) {
      throw new Error('BrowserClaw API returned an empty success response')
    }
    return result.data
  }
}
