/**
 * The app's HTTP seam for both server implementations. The generated HTTP
 * methods cover the entire contract; chat helpers preserve the raw Response so
 * either the AI SDK transport or readUiMessageStream can own consumption.
 */
import type { ChatRequest } from '@browseros/server-api'
import createClient, { type Client, type ClientOptions } from 'openapi-fetch'
import { ServerApiError } from './errors'
import type { paths } from './generated/openapi'

export class ServerApiClient {
  readonly http: Client<paths>

  constructor(baseUrl: string, options: Omit<ClientOptions, 'baseUrl'> = {}) {
    this.http = createClient<paths>({
      credentials: 'omit',
      ...options,
      baseUrl: baseUrl.replace(/\/+$/, ''),
    })
    // Throw before openapi-fetch parses the error, including text/plain OAuth
    // failures. Every JSON method and raw streaming request shares this path.
    this.http.use({
      async onResponse({ response }) {
        if (!response.ok) throw await ServerApiError.fromResponse(response)
      },
    })
  }

  async createChat(
    body: ChatRequest,
    options: { signal?: AbortSignal } = {},
  ): Promise<Response> {
    const { response } = await this.http.POST('/chat', {
      body,
      ...options,
      parseAs: 'stream',
    })
    return response
  }

  async reconnectChat(
    conversationId: string,
    options: { runId?: string; signal?: AbortSignal } = {},
  ): Promise<Response | null> {
    const { response } = await this.http.GET('/chat/{conversationId}/stream', {
      params: { path: { conversationId }, query: { runId: options.runId } },
      signal: options.signal,
      parseAs: 'stream',
    })
    return response.status === 204 ? null : response
  }
}
