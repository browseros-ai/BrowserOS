/**
 * Resolves the current server for both React and background callers. Only a
 * successfully discovered URL is cached; port changes replace the client and a
 * startup discovery failure cannot poison subsequent requests.
 */
import { ServerApiClient, ServerApiError } from '@browseros/server-api-client'
import { resolveAgentServerUrl } from '@/modules/browseros/agent-server-url.helpers'

const errorResponses = new WeakMap<ServerApiError, Response>()

let cached: { baseUrl: string; client: ServerApiClient } | undefined

export function serverApi(baseUrl: string): ServerApiClient {
  if (cached?.baseUrl !== baseUrl) {
    cached = { baseUrl, client: createServerApi(baseUrl) }
  }
  return cached.client
}

export async function getServerApi(): Promise<ServerApiClient> {
  return serverApi(await resolveAgentServerUrl())
}

/** Preserve each surface's existing HTTP error wording after shared decoding. */
export async function serverData<T>(
  request: Promise<{ data?: T }>,
  message?: (error: ServerApiError, response?: Response) => string,
): Promise<T> {
  try {
    const { data } = await request
    if (data === undefined) throw new Error('Server returned an empty response')
    return data
  } catch (error) {
    if (message && error instanceof ServerApiError) {
      const response = errorResponses.get(error)
      const mapped = new ServerApiError({
        status: error.status,
        code: error.code,
        message: message(error, response),
      })
      if (response) errorResponses.set(mapped, response)
      throw mapped
    }
    throw error
  }
}

/** Idempotent deletion and missing-history reads share the same 404 rule. */
export async function nullOnNotFound<T>(
  request: Promise<T>,
): Promise<T | null> {
  try {
    return await request
  } catch (error) {
    if (error instanceof ServerApiError && error.status === 404) return null
    throw error
  }
}

/** Preserve result-shaped HTTP failures without retrying a mutation. */
export async function serverResult<T>(
  baseUrl: string,
  request: (client: ServerApiClient) => Promise<{ data?: T }>,
  isResult: (value: unknown) => value is T,
  message?: (error: ServerApiError, response?: Response) => string,
): Promise<T> {
  try {
    return await serverData(request(serverApi(baseUrl)), message)
  } catch (error) {
    const response =
      error instanceof ServerApiError ? errorResponses.get(error) : undefined
    if (response) {
      const value: unknown = await response
        .clone()
        .json()
        .catch(() => null)
      if (isResult(value)) return value
    }
    throw error
  }
}

/**
 * C1's shared error decoder owns all dialects. This app-only middleware keeps
 * the untouched response alongside that same error, because a few legacy UIs
 * render statusText or return a typed failure result (including server timing).
 * Weak ownership avoids retaining response bodies after callers discard errors.
 */
function createServerApi(baseUrl: string): ServerApiClient {
  const client = new ServerApiClient(baseUrl, {
    fetch: (request) => globalThis.fetch(request),
  })
  client.http.use({
    async onResponse({ response }) {
      if (response.ok) return
      const error = await ServerApiError.fromResponse(response)
      errorResponses.set(error, response)
      throw error
    },
  })
  return client
}
