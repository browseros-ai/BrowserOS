/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Integration tests for BrowserOS fetch with retry and rate-limit handling.
 */

import { beforeEach, describe, expect, it, mock } from 'bun:test'
import { APICallError } from '@ai-sdk/provider'
import { createBrowserOSFetch } from '../../src/lib/browseros-fetch'

describe('BrowserOS Fetch Integration', () => {
  beforeEach(() => {
    // Reset fetch mock before each test
    globalThis.fetch = mock(async () => {
      return new Response('OK', { status: 200 })
    })
  })

  it('adds X-BrowserOS-ID header to requests', async () => {
    const browserosId = 'test-123'

    let capturedHeaders: Headers | undefined
    globalThis.fetch = mock(async (_url, options) => {
      capturedHeaders = options?.headers as Headers
      return new Response('OK', { status: 200 })
    })

    const fetch = createBrowserOSFetch(browserosId)
    await fetch('http://example.com')

    expect(capturedHeaders?.get('X-BrowserOS-ID')).toBe(browserosId)
  })

  it('throws APICallError with isRetryable=false for CREDITS_EXHAUSTED', async () => {
    globalThis.fetch = mock(async () => {
      return new Response(
        JSON.stringify({
          error: {
            code: 'CREDITS_EXHAUSTED',
            message: 'Daily credits exhausted',
          },
        }),
        {
          status: 429,
          headers: { 'Content-Type': 'application/json' },
        },
      )
    })

    const fetch = createBrowserOSFetch('test-123')

    try {
      await fetch('http://example.com')
      expect.unreachable()
    } catch (error) {
      expect(error instanceof APICallError).toBe(true)
      const apiError = error as APICallError
      expect(apiError.statusCode).toBe(429)
      expect(apiError.isRetryable).toBe(false)
      expect(apiError.message).toContain('Daily credits exhausted')
    }
  })

  it('retries on 429 without CREDITS_EXHAUSTED code', async () => {
    let attempts = 0
    globalThis.fetch = mock(async () => {
      attempts++
      if (attempts < 2) {
        return new Response(
          JSON.stringify({
            error: {
              code: 'RATE_LIMIT_EXCEEDED',
              message: 'Rate limit exceeded',
            },
          }),
          {
            status: 429,
            headers: {
              'Content-Type': 'application/json',
              'Retry-After': '0',
            },
          },
        )
      }
      return new Response('OK', { status: 200 })
    })

    const fetch = createBrowserOSFetch('test-123')
    const result = await fetch('http://example.com')

    expect(result.status).toBe(200)
    expect(attempts).toBe(2)
  })

  it('extracts error details from response body', async () => {
    globalThis.fetch = mock(async () => {
      return new Response(
        JSON.stringify({
          error: {
            code: 'SOME_ERROR',
            message: 'Something went wrong',
            metadata: {
              raw: { details: 'Additional context' },
            },
          },
        }),
        {
          status: 500,
          headers: {
            'Content-Type': 'application/json',
            'Retry-After': '0',
          },
        },
      )
    })

    const fetch = createBrowserOSFetch('test-123')

    try {
      await fetch('http://example.com')
      expect.unreachable()
    } catch (error) {
      const apiError = error as APICallError
      expect(apiError.message).toContain('SOME_ERROR')
      expect(apiError.message).toContain('Something went wrong')
    }
  })

  it('parses Retry-After header from 429 responses', async () => {
    globalThis.fetch = mock(async () => {
      return new Response(
        JSON.stringify({
          error: {
            code: 'RATE_LIMIT',
            message: 'Rate limited',
          },
        }),
        {
          status: 429,
          headers: {
            'Content-Type': 'application/json',
            'Retry-After': '0',
          },
        },
      )
    })

    const fetch = createBrowserOSFetch('test-123')

    try {
      await fetch('http://example.com')
    } catch (error) {
      const apiError = error as APICallError
      expect(apiError.responseHeaders?.['retry-after']).toBe('0')
    }
  })

  it('throws APICallError for non-retryable errors like 404', async () => {
    globalThis.fetch = mock(async () => {
      return new Response('Not Found', { status: 404 })
    })

    const fetch = createBrowserOSFetch('test-123')

    try {
      await fetch('http://example.com')
      expect.unreachable()
    } catch (error) {
      expect(error instanceof APICallError).toBe(true)
      const apiError = error as APICallError
      expect(apiError.statusCode).toBe(404)
      expect(apiError.isRetryable).toBe(false)
    }
  })

  it('logs credits remaining from response header', async () => {
    globalThis.fetch = mock(async () => {
      return new Response('OK', {
        status: 200,
        headers: { 'X-Credits-Remaining': '95' },
      })
    })

    const fetch = createBrowserOSFetch('test-123')
    const response = await fetch('http://example.com')

    expect(response.status).toBe(200)
    expect(response.headers.get('X-Credits-Remaining')).toBe('95')
  })
})
