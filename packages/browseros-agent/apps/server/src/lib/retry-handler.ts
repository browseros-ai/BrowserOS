/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Production-quality HTTP retry handler with exponential backoff, jitter, and Retry-After support.
 * Follows HTTP specifications for rate limiting (RFC 7231, RFC 6585).
 */

import { HTTP_RETRY } from '@browseros/shared/constants/timeouts'
import { logger } from './logger'

export interface RetryOptions {
  /**
   * Maximum number of retries (not including the initial attempt).
   * @default HTTP_RETRY.MAX_RETRIES
   */
  maxRetries?: number
  /**
   * Base delay in milliseconds for exponential backoff.
   * @default HTTP_RETRY.BASE_DELAY_MS
   */
  baseDelayMs?: number
  /**
   * Maximum jitter in milliseconds to add to each retry delay.
   * @default HTTP_RETRY.JITTER_MAX_MS
   */
  jitterMaxMs?: number
  /**
   * Custom status codes to retry on. If omitted, uses HTTP_RETRY.RETRYABLE_STATUS_CODES
   */
  retryableStatusCodes?: readonly number[]
}

export interface RetryContext {
  url: string
  statusCode: number
  attempt: number
  totalAttempts: number
  retryAfterSeconds?: number
  reason: string
}

export type FetchLike = (
  url: RequestInfo | URL,
  options?: RequestInit,
) => Promise<Response>

/**
 * Represents an error that occurred during request execution.
 * Used internally to distinguish retry-eligible errors from fatal errors.
 */
export class RetryableError extends Error {
  constructor(
    message: string,
    public readonly statusCode: number,
    public readonly responseBody?: string,
    public readonly retryAfterSeconds?: number,
  ) {
    super(message)
    this.name = 'RetryableError'
  }
}

/**
 * Parses the Retry-After header per HTTP specifications.
 * Returns delay in seconds, or undefined if header is invalid.
 *
 * @param retryAfterHeader The value of the Retry-After header
 * @returns Delay in seconds, or undefined if invalid
 */
export function parseRetryAfter(
  retryAfterHeader: string | null | undefined,
): number | undefined {
  const trimmedHeader = retryAfterHeader?.trim()
  if (!trimmedHeader) return undefined

  // Attempt to parse as seconds (RFC 7231 Section 7.1.3)
  if (/^\d+$/.test(trimmedHeader)) {
    return Number(trimmedHeader)
  }

  // Attempt to parse as HTTP-date (RFC 7231 Section 7.1.1)
  // Example: "Fri, 31 Dec 1999 23:59:59 GMT"
  try {
    const retryDate = new Date(trimmedHeader)
    if (!Number.isNaN(retryDate.getTime())) {
      const delayMs = retryDate.getTime() - Date.now()
      if (delayMs > 0) {
        return Math.ceil(delayMs / 1000)
      }
    }
  } catch {
    // Invalid date format, will return undefined
  }

  return undefined
}

/**
 * Calculates exponential backoff delay with jitter.
 * Formula: baseDelay * 2^attempt + random(0, jitterMax)
 *
 * @param attempt Zero-indexed attempt number
 * @param baseDelayMs Base delay in milliseconds
 * @param jitterMaxMs Maximum jitter to add
 * @returns Delay in milliseconds
 */
export function calculateBackoffDelay(
  attempt: number,
  baseDelayMs: number,
  jitterMaxMs: number,
): number {
  const exponentialDelay = baseDelayMs * 2 ** attempt
  const jitter = Math.random() * jitterMaxMs
  return exponentialDelay + jitter
}

/**
 * Determines if a status code is retryable.
 * Non-retryable codes: 400, 401, 403, 404, and 429 with CREDITS_EXHAUSTED.
 *
 * @param statusCode HTTP status code
 * @param isCreditsExhausted Whether the 429 error is due to exhausted credits
 * @param retryableStatusCodes Custom list of retryable codes
 * @returns Whether the error should be retried
 */
export function isRetryableStatus(
  statusCode: number,
  isCreditsExhausted = false,
  retryableStatusCodes?: readonly number[],
): boolean {
  // 429 with CREDITS_EXHAUSTED is NOT retryable
  if (statusCode === 429 && isCreditsExhausted) return false

  // Use custom codes if provided, otherwise use default
  const codes = retryableStatusCodes ?? HTTP_RETRY.RETRYABLE_STATUS_CODES
  return codes.includes(statusCode)
}

/**
 * Handles a network error during fetch retry, either throwing (if max retries reached) or scheduling retry.
 * @returns Delay in milliseconds to wait, or throws if max retries exhausted
 */
function handleNetworkError(
  error: Error,
  attempt: number,
  maxRetries: number,
  baseDelayMs: number,
  jitterMaxMs: number,
  urlStr: string,
): number {
  if (attempt === maxRetries) {
    logger.error('HTTP request failed after max retries (network error)', {
      url: urlStr,
      error: error instanceof Error ? error.message : String(error),
      attempts: attempt + 1,
    })
    throw error
  }

  const backoffDelayMs = calculateBackoffDelay(
    attempt,
    baseDelayMs,
    jitterMaxMs,
  )
  logger.debug('HTTP request failed (network), will retry', {
    url: urlStr,
    error: error instanceof Error ? error.message : String(error),
    attempt: attempt + 1,
    nextRetryMs: backoffDelayMs,
  })

  return backoffDelayMs
}

/**
 * Cancels a response body so discarded retry responses do not keep streams open.
 */
async function cancelResponseBody(response: Response): Promise<void> {
  if (!response.body) return

  try {
    await response.body.cancel()
  } catch {
    // Best effort only.
  }
}

/**
 * Helper to wait for a given number of milliseconds.
 * @param ms Milliseconds to wait
 */
async function wait(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

/**
 * Evaluates a response to determine if it should be retried.
 * Returns retry decision and delay (if retrying).
 */
async function evaluateResponseForRetry(
  response: Response,
  attempt: number,
  maxRetries: number,
  baseDelayMs: number,
  jitterMaxMs: number,
  retryableStatusCodes: readonly number[],
  urlStr: string,
): Promise<{ shouldRetry: boolean; delayMs?: number }> {
  // Check if this is a retryable error
  const isCreditsExhausted =
    response.status === 429 && (await isCreditsExhaustedError(response))

  if (
    !isRetryableStatus(
      response.status,
      isCreditsExhausted,
      retryableStatusCodes,
    )
  ) {
    // Not retryable - return the error response
    return { shouldRetry: false }
  }

  // If this is the last attempt, can't retry
  if (attempt === maxRetries) {
    logger.warn('HTTP request failed after max retries', {
      url: urlStr,
      statusCode: response.status,
      attempts: attempt + 1,
      maxRetries,
    })
    return { shouldRetry: false }
  }

  // Calculate delay
  const retryAfterHeader = response.headers.get('Retry-After')
  const retryAfterSeconds = parseRetryAfter(retryAfterHeader)
  const delayMs =
    retryAfterSeconds !== undefined
      ? retryAfterSeconds * 1000 // Retry-After takes precedence
      : calculateBackoffDelay(attempt, baseDelayMs, jitterMaxMs)

  logger.debug('HTTP request will be retried', {
    url: urlStr,
    statusCode: response.status,
    attempt: attempt + 1,
    nextRetryMs: delayMs,
    retryAfterHeader,
  })

  return { shouldRetry: true, delayMs }
}

type FetchWithPreconnect = FetchLike & {
  preconnect?: typeof globalThis.fetch.preconnect
}

function getBoundPreconnect(
  fetchLike: FetchLike | typeof globalThis.fetch,
): typeof globalThis.fetch.preconnect | undefined {
  const candidate = fetchLike as FetchWithPreconnect
  if (typeof candidate.preconnect !== 'function') {
    return undefined
  }

  return candidate.preconnect.bind(candidate)
}

/**
 * Wraps a fetch function with exponential backoff retry logic.
 * Respects Retry-After headers and includes jitter for thundering herd mitigation.
 *
 * @param fetch The fetch function to wrap
 * @param options Retry configuration
 * @returns A new fetch function with retry capability
 */
export function createRetryableFetch(
  fetch: FetchLike,
  options: RetryOptions = {},
): typeof globalThis.fetch {
  const maxRetries = options.maxRetries ?? HTTP_RETRY.MAX_RETRIES
  const baseDelayMs = options.baseDelayMs ?? HTTP_RETRY.BASE_DELAY_MS
  const jitterMaxMs = options.jitterMaxMs ?? HTTP_RETRY.JITTER_MAX_MS
  const retryableStatusCodes =
    options.retryableStatusCodes ?? HTTP_RETRY.RETRYABLE_STATUS_CODES
  const retryableFetch = async (
    url: RequestInfo | URL,
    options?: RequestInit,
  ): Promise<Response> => {
    const urlStr = typeof url === 'string' ? url : url.toString()
    let lastError: Error | RetryableError | undefined
    let lastResponse: Response | undefined

    for (let attempt = 0; attempt <= maxRetries; attempt++) {
      try {
        const response = await fetch(url, options)

        // If successful, return immediately
        if (response.ok) {
          return response
        }

        // Evaluate if we should retry this response
        const { shouldRetry, delayMs } = await evaluateResponseForRetry(
          response,
          attempt,
          maxRetries,
          baseDelayMs,
          jitterMaxMs,
          retryableStatusCodes,
          urlStr,
        )

        if (!shouldRetry) {
          return response
        }

        // Store response for potential retry and wait before next attempt
        lastResponse = response
        if (delayMs === undefined) {
          throw new Error('Expected retry delay to be defined')
        }
        await cancelResponseBody(response)
        await wait(delayMs)
      } catch (error) {
        const backoffDelayMs = handleNetworkError(
          error as Error,
          attempt,
          maxRetries,
          baseDelayMs,
          jitterMaxMs,
          urlStr,
        )
        lastError = error as Error
        await wait(backoffDelayMs)
      }
    }

    // Exhausted retries - return last response or throw last error
    if (lastResponse) {
      return lastResponse
    }
    throw lastError ?? new Error('Unknown error during retry loop')
  }

  const preconnect =
    getBoundPreconnect(fetch) ?? getBoundPreconnect(globalThis.fetch)

  return Object.assign(
    retryableFetch,
    preconnect ? { preconnect } : {},
  ) as typeof globalThis.fetch
}

/**
 * Checks if a 429 response is due to exhausted credits.
 * Reads and parses the response body, so be careful with streaming responses.
 *
 * @param response The HTTP response to check
 * @returns Whether the error is CREDITS_EXHAUSTED
 */
async function isCreditsExhaustedError(response: Response): Promise<boolean> {
  try {
    const cloned = response.clone()
    const text = await cloned.text()
    const parsed = JSON.parse(text)
    return parsed.error?.code === 'CREDITS_EXHAUSTED'
  } catch {
    return false
  }
}
