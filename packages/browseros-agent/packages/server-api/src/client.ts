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

type Schemas = components['schemas']
type PathOf<Operation extends keyof operations> =
  operations[Operation]['parameters']['path']

type NotInClient = 'startTurn' | 'resumeStream' | 'watchPanels' | 'startSignIn'

// Every other operation must be a method of the same name: an operation added to the contract
// fails typecheck at `implements JsonOperations` until the client gains its method.
type JsonOperations = Record<
  Exclude<keyof operations, NotInClient>,
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

  async getRunState(path: PathOf<'getRunState'>): Promise<Schemas['RunState']> {
    return unwrap(
      await this.client.GET('/chat/{conversationId}/state', {
        params: { path },
      }),
    )
  }

  async stopTurn(path: PathOf<'stopTurn'>): Promise<Schemas['StopResult']> {
    return unwrap(
      await this.client.POST('/chat/{conversationId}/stop', {
        params: { path },
      }),
    )
  }

  async releaseConversation(
    path: PathOf<'releaseConversation'>,
  ): Promise<Schemas['Success']> {
    return unwrap(
      await this.client.DELETE('/chat/{conversationId}', { params: { path } }),
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
    if (result.error && 'success' in result.error) return result.error
    return unwrap(result)
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
    return unwrap(await this.client.POST('/acpx/probe', { body }))
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
    typeof (body as { error?: unknown }).error === 'string'
  )
}
