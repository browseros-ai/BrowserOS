/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Custom fetch for BrowserOS gateway requests.
 * Adds X-BrowserOS-ID header for credit tracking,
 * handles CREDITS_EXHAUSTED (429), and extracts OpenRouter-style error details.
 */

import { APICallError } from '@ai-sdk/provider'
import { logger } from './logger'

function resolveUrl(url: RequestInfo | URL): string {
  return typeof url === 'string' ? url : url.toString()
}

/** Retry-After and rate-limit headers only reach the SDK via responseHeaders. */
function headersToRecord(headers: Headers): Record<string, string> {
  const record: Record<string, string> = {}
  headers.forEach((value, key) => {
    record[key] = value
  })
  return record
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
  return (async (url: RequestInfo | URL, options?: RequestInit) => {
    const headers = new Headers(options?.headers)
    headers.set('X-BrowserOS-ID', browserosId)

    const response = await globalThis.fetch(url, { ...options, headers })

    const creditsRemaining = response.headers.get('X-Credits-Remaining')
    if (creditsRemaining !== null) {
      logger.debug('Credits remaining', { creditsRemaining })
    }

    if (!response.ok) {
      const statusCode = response.status
      const responseBody = await response.text()
      const error = parseErrorBody(responseBody)

      // `data` keeps the gateway's code structured so the chat error
      // classifier branches on a field instead of searching the message text.
      const data = error?.code
        ? { code: error.code, raw: error.metadata?.raw }
        : undefined

      if (statusCode === 429 && error?.code === 'CREDITS_EXHAUSTED') {
        throw new APICallError({
          message: error.message ?? 'Daily credits exhausted',
          url: resolveUrl(url),
          requestBodyValues: {},
          statusCode,
          responseBody,
          responseHeaders: headersToRecord(response.headers),
          isRetryable: false,
          data,
        })
      }

      throw new APICallError({
        message: error
          ? buildErrorMessage(statusCode, response.statusText, error)
          : `HTTP ${statusCode}: ${response.statusText}`,
        url: resolveUrl(url),
        requestBodyValues: {},
        statusCode,
        responseBody,
        responseHeaders: headersToRecord(response.headers),
        data,
      })
    }

    return response
  }) as typeof fetch
}
