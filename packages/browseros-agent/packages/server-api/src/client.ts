/**
 * The extension's typed client for the BrowserOS server (`crates/browseros-server`). Every
 * method is one JSON operation of `contracts/server-api/openapi.json`, named by its
 * `operationId`, over `openapi-fetch` and the generated `./generated/openapi` types, so a
 * contract change in the Rust server surfaces here as a type error.
 *
 * Four operations stay out on purpose, and the extension calls them with its own `fetch`:
 * `startTurn`, `resumeStream` and `watchPanels` answer with server-sent events that its stream
 * readers consume, and `startSignIn` answers with a redirect the extension follows.
 */

import createClient, { type Client } from 'openapi-fetch'
import type { components, operations, paths } from './generated/openapi'

export type Schemas = components['schemas']
export type PathOf<Operation extends keyof operations> =
  operations[Operation]['parameters']['path']

// Every other operation must be a method of the same name: an operation added to the contract
// fails typecheck at `implements JsonOperations` until the client gains its method.
export type JsonOperations = Record<
  Exclude<
    keyof operations,
    'startTurn' | 'resumeStream' | 'watchPanels' | 'startSignIn'
  >,
  (...args: never[]) => Promise<unknown>
>

export interface ServerApiClientOptions {
  /** Defaults to `globalThis.fetch`. */
  fetch?: (input: Request) => Promise<Response>
}

/** A non-2xx answer. `apiMessage` is the server's one-sentence `{"error"}`, when it sent one. */
export class ServerApiError extends Error {
  override name = 'ServerApiError'
  readonly status: number
  readonly apiMessage?: string

  constructor(response: Response, body: unknown) {
    const apiMessage = isApiError(body) ? body.error : undefined
    super(
      `BrowserOS server answered ${response.status.toString()}${apiMessage ? `: ${apiMessage}` : ''}`,
    )
    this.status = response.status
    this.apiMessage = apiMessage
  }
}

type ApiResult<T> =
  | { data: T; error?: never; response: Response }
  | { data?: never; error: unknown; response: Response }

export class ServerApiClient implements JsonOperations {
  private readonly client: Client<paths>

  constructor(baseUrl: string, options: ServerApiClientOptions = {}) {
    this.client = createClient<paths>({
      baseUrl: baseUrl.replace(/\/+$/, ''),
      fetch: options.fetch,
    })
  }

  async getHealth(): Promise<Schemas['Health']> {
    return unwrap(await this.client.GET('/system/health'))
  }

  async getStatus(): Promise<Schemas['Status']> {
    return unwrap(await this.client.GET('/status'))
  }

  async getDiagnostics(): Promise<Schemas['Diagnostics']> {
    return unwrap(await this.client.GET('/system/diagnostics'))
  }

  async shutdown(): Promise<Schemas['ShutdownResult']> {
    return unwrap(await this.client.POST('/system/shutdown'))
  }

  async forgetPanel(path: PathOf<'forgetPanel'>): Promise<Schemas['Success']> {
    return unwrap(
      await this.client.DELETE('/chat/panels/{tabId}', { params: { path } }),
    )
  }

  async getRunState(
    path: PathOf<'getRunState'>,
    signal?: AbortSignal,
  ): Promise<Schemas['RunState']> {
    return unwrap(
      await this.client.GET('/chat/{conversationId}/state', {
        params: { path },
        cache: 'no-store',
        signal,
      }),
    )
  }

  async stopTurn(
    path: PathOf<'stopTurn'>,
    signal?: AbortSignal,
  ): Promise<Schemas['StopResult']> {
    return unwrap(
      await this.client.POST('/chat/{conversationId}/stop', {
        params: { path },
        signal,
      }),
    )
  }

  async releaseConversation(
    path: PathOf<'releaseConversation'>,
    signal?: AbortSignal,
  ): Promise<Schemas['Success']> {
    return unwrap(
      await this.client.DELETE('/chat/{conversationId}', {
        params: { path },
        // Conversation teardown can outlive the extension page that initiated it.
        keepalive: true,
        signal,
      }),
    )
  }

  async listConversations(): Promise<Schemas['ConversationList']> {
    return unwrap(await this.client.GET('/conversations'))
  }

  async getConversation(
    path: PathOf<'getConversation'>,
  ): Promise<Schemas['ConversationResponse']> {
    return unwrap(
      await this.client.GET('/conversations/{conversationId}', {
        params: { path },
      }),
    )
  }

  async deleteConversation(
    path: PathOf<'deleteConversation'>,
  ): Promise<Schemas['Success']> {
    return unwrap(
      await this.client.DELETE('/conversations/{conversationId}', {
        params: { path },
      }),
    )
  }

  async listProviders(): Promise<Schemas['ProviderList']> {
    return unwrap(await this.client.GET('/providers'))
  }

  async getDefaultProvider(): Promise<Schemas['DefaultProvider']> {
    return unwrap(await this.client.GET('/providers/default'))
  }

  async setDefaultProvider(
    body: Schemas['SetDefaultProvider'],
  ): Promise<Schemas['ProviderResponse']> {
    return unwrap(await this.client.PUT('/providers/default', { body }))
  }

  async saveProvider(
    path: PathOf<'saveProvider'>,
    body: Schemas['ProviderInput'],
  ): Promise<Schemas['ProviderResponse']> {
    return unwrap(
      await this.client.PUT('/providers/{providerId}', {
        params: { path },
        body,
      }),
    )
  }

  async deleteProvider(
    path: PathOf<'deleteProvider'>,
  ): Promise<Schemas['Success']> {
    return unwrap(
      await this.client.DELETE('/providers/{providerId}', { params: { path } }),
    )
  }

  /** A failed test is an answer, not an error: `400` carries the same result shape as `200`. */
  async testProvider(
    body: Schemas['TestProviderRequest'],
  ): Promise<Schemas['TestProviderResult']> {
    const result = await this.client.POST('/test-provider', { body })
    const bodyValue: unknown =
      result.response.status === 400 ? result.error : unwrap(result)
    if (isTestProviderResult(bodyValue)) return bodyValue
    if (result.response.ok) {
      throw new Error(
        'BrowserOS server returned an invalid provider test result',
      )
    }
    throw new ServerApiError(result.response, bodyValue)
  }

  async listAgents(): Promise<Schemas['AgentList']> {
    return unwrap(await this.client.GET('/agents'))
  }

  async createAgent(
    body: Schemas['AgentInput'],
  ): Promise<Schemas['AgentResponse']> {
    return unwrap(await this.client.POST('/agents', { body }))
  }

  async updateAgent(
    path: PathOf<'updateAgent'>,
    body: Schemas['AgentPatch'],
  ): Promise<Schemas['AgentResponse']> {
    return unwrap(
      await this.client.PUT('/agents/{agentId}', { params: { path }, body }),
    )
  }

  async deleteAgent(path: PathOf<'deleteAgent'>): Promise<Schemas['Success']> {
    return unwrap(
      await this.client.DELETE('/agents/{agentId}', { params: { path } }),
    )
  }

  /** A probe that could not run is still `200`, with `error` set. */
  async probeAgent(
    body: Schemas['ProbeRequest'],
  ): Promise<Schemas['AgentProbe']> {
    const result = await this.client.POST('/acpx/probe', { body })
    const error: unknown = result.error
    // The TypeScript server reports probe failures as a 500 with a nested
    // message. The Rust server returns the failure in its success body.
    if (
      result.response.status === 500 &&
      typeof error === 'object' &&
      error !== null &&
      'error' in error &&
      typeof error.error === 'object' &&
      error.error !== null &&
      'message' in error.error &&
      typeof error.error.message === 'string'
    ) {
      throw new ServerApiError(result.response, { error: error.error.message })
    }
    return unwrap(result)
  }

  async getSignInStatus(
    path: PathOf<'getSignInStatus'>,
  ): Promise<Schemas['SignInStatus']> {
    return unwrap(
      await this.client.GET('/oauth/{provider}/status', { params: { path } }),
    )
  }

  async signOut(path: PathOf<'signOut'>): Promise<Schemas['Success']> {
    return unwrap(
      await this.client.DELETE('/oauth/{provider}', { params: { path } }),
    )
  }
}

function unwrap<T>(result: ApiResult<T | undefined>): T {
  if ('error' in result) throw new ServerApiError(result.response, result.error)
  // Every operation in this client answers 2xx with a JSON body.
  if (result.data === undefined) {
    throw new Error('BrowserOS server returned an empty success response')
  }
  return result.data
}

function isApiError(body: unknown): body is Schemas['ApiError'] {
  return (
    typeof body === 'object' &&
    body !== null &&
    'error' in body &&
    typeof body.error === 'string'
  )
}

function isTestProviderResult(
  body: unknown,
): body is Schemas['TestProviderResult'] {
  return (
    typeof body === 'object' &&
    body !== null &&
    'success' in body &&
    typeof body.success === 'boolean' &&
    'message' in body &&
    typeof body.message === 'string' &&
    (!('responseTime' in body) ||
      body.responseTime === null ||
      typeof body.responseTime === 'number')
  )
}
