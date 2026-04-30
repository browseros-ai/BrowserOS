/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Custom fetch for BrowserOS gateway requests.
 * Features:
 * - X-BrowserOS-ID header for credit tracking
 * - Exponential backoff with jitter for rate-limited requests
 * - Retry-After header respect (RFC 7231)
 * - CREDITS_EXHAUSTED (429) detection (marked as non-retryable)
 * - OpenRouter-style error detail extraction
 */

import { APICallError } from '@ai-sdk/provider'
import { HTTP_RETRY } from '@browseros/shared/constants/timeouts'
import { logger } from './logger'
import {
  createRetryableFetch,
  type FetchLike,
  isRetryableStatus,
} from './retry-handler'

function resolveUrl(url: RequestInfo | URL): string {
  return typeof url === 'string' ? url : url.toString()
}

function parseErrorBody(
  body: string,
): { message?: string; code?: string; metadata?: { raw?: unknown } } | null {
  try {
    const parsed = JSON.parse(body)
    return parsed.error ?? null
  } catch {
    return null
  }
}

function buildErrorMessage(
  statusCode: number,
  statusText: string,
  error: NonNullable<ReturnType<typeof parseErrorBody>>,
): string {
  if (!error.message) return `HTTP ${statusCode}: ${statusText}`
  let msg = error.message
  if (error.code) msg = `[${error.code}] ${msg}`
  if (error.metadata?.raw) msg += ` (${JSON.stringify(error.metadata.raw)})`
  return msg
}

export function createBrowserOSFetch(browserosId: string): typeof fetch {
  // Create a fetch wrapper that adds BrowserOS ID header
  const fetchWithHeader: FetchLike = async (
    url: RequestInfo | URL,
    options?: RequestInit,
  ): Promise<Response> => {
    const headers = new Headers(options?.headers)
    headers.set('X-BrowserOS-ID', browserosId)
    return globalThis.fetch(url, { ...options, headers })
  }

  // Wrap with retry handler for automatic exponential backoff
  const retryableFetch = createRetryableFetch(fetchWithHeader, {
    maxRetries: HTTP_RETRY.MAX_RETRIES,
    baseDelayMs: HTTP_RETRY.BASE_DELAY_MS,
    jitterMaxMs: HTTP_RETRY.JITTER_MAX_MS,
  })

  return (async (url: RequestInfo | URL, options?: RequestInit) => {
    const response = await retryableFetch(url, options)

    const creditsRemaining = response.headers.get('X-Credits-Remaining')
    if (creditsRemaining !== null) {
      logger.debug('Credits remaining', { creditsRemaining })
    }

    if (!response.ok) {
      const statusCode = response.status
      const responseBody = await response.text()
      const error = parseErrorBody(responseBody)
      const isCreditsExhausted = error?.code === 'CREDITS_EXHAUSTED'
      const retryAfterHeader = response.headers.get('Retry-After')
      const responseHeaders = retryAfterHeader
        ? { 'retry-after': retryAfterHeader }
        : undefined

      // CREDITS_EXHAUSTED is fatal and should not be retried
      if (statusCode === 429 && isCreditsExhausted) {
        throw new APICallError({
          message: error?.message ?? 'Daily credits exhausted',
          url: resolveUrl(url),
          requestBodyValues: {},
          statusCode,
          responseBody,
          responseHeaders,
          isRetryable: false,
        })
      }

      // For other errors, calculate whether they're retryable
      const isRetryable = isRetryableStatus(statusCode, false)

      throw new APICallError({
        message: error
          ? buildErrorMessage(statusCode, response.statusText, error)
          : `HTTP ${statusCode}: ${response.statusText}`,
        url: resolveUrl(url),
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
