import { APICallError } from '@ai-sdk/provider'
import { HTTP_RETRY } from '@browseros/shared/constants/timeouts'
import { createRetryableFetch, isRetryableStatus } from './retry-handler'

/**
 * Creates a fetch function for OpenRouter-compatible APIs.
 * Features:
 * - Exponential backoff with jitter for rate-limited requests (RFC 7231)
 * - Retry-After header respect
 * - OpenRouter-style error detail extraction and message enhancement
 *
 * The Vercel AI SDK's retry mechanism is also applied by this fetch wrapper,
 * providing defense-in-depth retry handling. This wrapper explicitly classifies
 * retryability for the APICallError it throws, and the SDK also respects the
 * status code and Retry-After headers when it applies its own retry logic.
 *
 * Note: This is primarily for non-gateway APIs. For gateway requests, use createBrowserOSFetch.
 */
export function createOpenRouterCompatibleFetch(): typeof fetch {
  // Wrap with retry handler for exponential backoff
  const retryableFetch = createRetryableFetch(globalThis.fetch, {
    maxRetries: HTTP_RETRY.MAX_RETRIES,
    baseDelayMs: HTTP_RETRY.BASE_DELAY_MS,
    jitterMaxMs: HTTP_RETRY.JITTER_MAX_MS,
  })

  return (async (url: RequestInfo | URL, options?: RequestInit) => {
    const response = await retryableFetch(url, options)

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

      // Determine if retryable based on status code
      const isRetryable = isRetryableStatus(statusCode, false)
      const retryAfterHeader = response.headers.get('Retry-After')
      const responseHeaders = retryAfterHeader
        ? { 'retry-after': retryAfterHeader }
        : undefined

      throw new APICallError({
        message: errorMessage,
        url: typeof url === 'string' ? url : url.toString(),
        requestBodyValues: {},
        statusCode,
        responseBody,
        responseHeaders,
        isRetryable,
      })
    }

    return response
  }) as typeof fetch
}
