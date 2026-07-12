import { APICallError } from '@ai-sdk/provider'

/**
 * Creates a fetch function that extracts detailed error messages from Requesty.
 *
 * Requesty is an OpenAI-compatible router (like OpenRouter). It can wrap
 * upstream provider errors in a generic message, with the actual details
 * nested under metadata.raw. This fetch intercepts HTTP errors and extracts
 * the real error message so callers see the upstream failure.
 *
 * IMPORTANT: Throws APICallError (not plain Error) so the Vercel AI SDK's retry
 * mechanism works correctly. The SDK's APICallError automatically calculates
 * `isRetryable` from the statusCode (408, 409, 429, 500+ are retryable) - we
 * don't override this default. Requesty's router occasionally returns transient
 * 502s, so retryable handling matters here.
 */
export function createRequestyCompatibleFetch(): typeof fetch {
  return (async (url: RequestInfo | URL, options?: RequestInit) => {
    const response = await globalThis.fetch(url, options)

    if (!response.ok) {
      const statusCode = response.status
      let errorMessage = `HTTP ${statusCode}: ${response.statusText}`
      let responseBody: string | undefined

      try {
        responseBody = await response.clone().text()
        const parsed = JSON.parse(responseBody)
        if (parsed.error?.message) {
          errorMessage = parsed.error.message
          if (parsed.error.code) {
            errorMessage = `[${parsed.error.code}] ${errorMessage}`
          }
          if (parsed.error.metadata?.raw) {
            errorMessage += ` (${JSON.stringify(parsed.error.metadata.raw)})`
          }
        }
      } catch {
        // Keep default error message if parsing fails
      }

      throw new APICallError({
        message: errorMessage,
        url: typeof url === 'string' ? url : url.toString(),
        requestBodyValues: {},
        statusCode,
        responseBody,
      })
    }

    return response
  }) as typeof fetch
}
