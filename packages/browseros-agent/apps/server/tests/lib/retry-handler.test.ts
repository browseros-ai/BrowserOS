/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Tests for HTTP retry handler with exponential backoff and Retry-After support.
 */

import { beforeEach, describe, expect, it, mock } from 'bun:test'
import {
  calculateBackoffDelay,
  createRetryableFetch,
  isRetryableStatus,
  parseRetryAfter,
} from '../../src/lib/retry-handler'

describe('Retry Handler', () => {
  describe('parseRetryAfter', () => {
    it('parses numeric retry-after as seconds', () => {
      expect(parseRetryAfter('60')).toBe(60)
      expect(parseRetryAfter('120')).toBe(120)
    })

    it('parses HTTP-date retry-after', () => {
      // Create a date 10 seconds in the future
      const futureDate = new Date(Date.now() + 10_000)
      const httpDate = futureDate.toUTCString()
      const parsed = parseRetryAfter(httpDate)

      if (parsed === undefined) {
        throw new Error('Expected Retry-After date to parse')
      }

      expect(parsed).toBeGreaterThan(8) // Allow some variance
      expect(parsed).toBeLessThan(12)
    })

    it('returns undefined for invalid retry-after', () => {
      expect(parseRetryAfter('invalid')).toBeUndefined()
      expect(parseRetryAfter('')).toBeUndefined()
      expect(parseRetryAfter(null)).toBeUndefined()
      expect(parseRetryAfter(undefined)).toBeUndefined()
    })

    it('returns undefined for past dates', () => {
      const pastDate = new Date(Date.now() - 10_000)
      const httpDate = pastDate.toUTCString()
      expect(parseRetryAfter(httpDate)).toBeUndefined()
    })
  })

  describe('calculateBackoffDelay', () => {
    it('calculates exponential backoff without exceeding max jitter', () => {
      const baseDelay = 100
      const jitterMax = 50

      for (let attempt = 0; attempt < 5; attempt++) {
        const delay = calculateBackoffDelay(attempt, baseDelay, jitterMax)
        const minDelay = baseDelay * 2 ** attempt
        const maxDelay = minDelay + jitterMax

        expect(delay).toBeGreaterThanOrEqual(minDelay)
        expect(delay).toBeLessThanOrEqual(maxDelay)
      }
    })

    it('increases delay exponentially with attempt number', () => {
      const baseDelay = 100
      const jitterMax = 0 // No jitter for consistent testing

      const delay0 = calculateBackoffDelay(0, baseDelay, jitterMax)
      const delay1 = calculateBackoffDelay(1, baseDelay, jitterMax)
      const delay2 = calculateBackoffDelay(2, baseDelay, jitterMax)

      expect(delay1).toBe(delay0 * 2)
      expect(delay2).toBe(delay1 * 2)
    })
  })

  describe('isRetryableStatus', () => {
    it('retries on 429 (rate limit)', () => {
      expect(isRetryableStatus(429)).toBe(true)
    })

    it('does not retry on 429 with CREDITS_EXHAUSTED', () => {
      expect(isRetryableStatus(429, true)).toBe(false)
    })

    it('retries on server errors (5xx)', () => {
      expect(isRetryableStatus(500)).toBe(true)
      expect(isRetryableStatus(502)).toBe(true)
      expect(isRetryableStatus(503)).toBe(true)
      expect(isRetryableStatus(504)).toBe(true)
    })

    it('retries on 408 (timeout)', () => {
      expect(isRetryableStatus(408)).toBe(true)
    })

    it('does not retry on client errors (4xx except 429, 408)', () => {
      expect(isRetryableStatus(400)).toBe(false)
      expect(isRetryableStatus(401)).toBe(false)
      expect(isRetryableStatus(403)).toBe(false)
      expect(isRetryableStatus(404)).toBe(false)
    })

    it('supports custom retryable status codes', () => {
      const customCodes = [418, 429, 500] as const
      expect(isRetryableStatus(418, false, customCodes)).toBe(true)
      expect(isRetryableStatus(403, false, customCodes)).toBe(false)
    })
  })

  describe('createRetryableFetch', () => {
    let fetchMock: ReturnType<typeof mock>

    beforeEach(() => {
      fetchMock = mock(async () => {
        throw new Error('Network error')
      })
    })

    it('returns successful response immediately', async () => {
      const response = new Response('OK', { status: 200 })
      fetchMock = mock(async () => response)

      const retryableFetch = createRetryableFetch(fetchMock as typeof fetch)
      const result = await retryableFetch('http://example.com')

      expect(result).toBe(response)
      expect(fetchMock.mock.calls.length).toBe(1)
    })

    it('retries on network errors', async () => {
      let attempts = 0
      fetchMock = mock(async () => {
        attempts++
        if (attempts < 3) {
          throw new Error('Network error')
        }
        return new Response('OK', { status: 200 })
      })

      const retryableFetch = createRetryableFetch(fetchMock as typeof fetch, {
        maxRetries: 3,
        baseDelayMs: 1, // Minimal delay for testing
        jitterMaxMs: 0,
      })

      const result = await retryableFetch('http://example.com')
      expect(result.status).toBe(200)
      expect(attempts).toBe(3)
    })

    it('respects max retries limit', async () => {
      fetchMock = mock(async () => {
        throw new Error('Network error')
      })

      const retryableFetch = createRetryableFetch(fetchMock as typeof fetch, {
        maxRetries: 2,
        baseDelayMs: 1,
        jitterMaxMs: 0,
      })

      try {
        await retryableFetch('http://example.com')
      } catch (error) {
        expect(error instanceof Error).toBe(true)
      }

      // Original attempt + 2 retries = 3 total
      expect(fetchMock.mock.calls.length).toBe(3)
    })

    it('retries on 429 rate limit errors', async () => {
      let attempts = 0
      fetchMock = mock(async () => {
        attempts++
        if (attempts < 2) {
          return new Response('Too Many Requests', {
            status: 429,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        return new Response('OK', { status: 200 })
      })

      const retryableFetch = createRetryableFetch(fetchMock as typeof fetch, {
        maxRetries: 3,
        baseDelayMs: 1,
        jitterMaxMs: 0,
      })

      const result = await retryableFetch('http://example.com')
      expect(result.status).toBe(200)
      expect(attempts).toBe(2)
    })

    it('respects Retry-After header in seconds', async () => {
      let attempts = 0
      const startTime = Date.now()
      fetchMock = mock(async () => {
        attempts++
        if (attempts < 2) {
          return new Response('Too Many Requests', {
            status: 429,
            headers: {
              'Retry-After': '1',
              'Content-Type': 'application/json',
            },
          })
        }
        return new Response('OK', { status: 200 })
      })

      const retryableFetch = createRetryableFetch(fetchMock as typeof fetch, {
        maxRetries: 3,
        baseDelayMs: 100,
        jitterMaxMs: 0,
      })

      const result = await retryableFetch('http://example.com')
      const elapsedMs = Date.now() - startTime

      expect(result.status).toBe(200)
      expect(attempts).toBe(2)
      // Should wait at least 1000ms due to Retry-After header
      expect(elapsedMs).toBeGreaterThanOrEqual(900) // Allow 100ms variance
    })

    it('does not retry on non-retryable 4xx errors', async () => {
      fetchMock = mock(async () => {
        return new Response('Not Found', {
          status: 404,
          headers: { 'Content-Type': 'application/json' },
        })
      })

      const retryableFetch = createRetryableFetch(fetchMock as typeof fetch, {
        maxRetries: 3,
        baseDelayMs: 1,
        jitterMaxMs: 0,
      })

      const result = await retryableFetch('http://example.com')
      expect(result.status).toBe(404)
      expect(fetchMock.mock.calls.length).toBe(1) // Only one attempt
    })

    it('retries on 500+ server errors', async () => {
      let attempts = 0
      fetchMock = mock(async () => {
        attempts++
        if (attempts < 2) {
          return new Response('Internal Server Error', { status: 500 })
        }
        return new Response('OK', { status: 200 })
      })

      const retryableFetch = createRetryableFetch(fetchMock as typeof fetch, {
        maxRetries: 3,
        baseDelayMs: 1,
        jitterMaxMs: 0,
      })

      const result = await retryableFetch('http://example.com')
      expect(result.status).toBe(200)
      expect(attempts).toBe(2)
    })

    it('applies exponential backoff between retries', async () => {
      let attempts = 0
      const attemptTimes: number[] = []
      fetchMock = mock(async () => {
        attemptTimes.push(Date.now())
        attempts++
        if (attempts < 3) {
          return new Response('Too Many Requests', { status: 429 })
        }
        return new Response('OK', { status: 200 })
      })

      const baseDelay = 50
      const retryableFetch = createRetryableFetch(fetchMock as typeof fetch, {
        maxRetries: 3,
        baseDelayMs: baseDelay,
        jitterMaxMs: 0,
      })

      await retryableFetch('http://example.com')

      // Check that delays increase exponentially
      const delay1 = attemptTimes[1] - attemptTimes[0]
      const delay2 = attemptTimes[2] - attemptTimes[1]

      expect(delay1).toBeGreaterThanOrEqual(baseDelay)
      expect(delay2).toBeGreaterThanOrEqual(baseDelay * 2)
    })

    it('handles custom retryable status codes', async () => {
      let attempts = 0
      fetchMock = mock(async () => {
        attempts++
        if (attempts < 2) {
          return new Response('I am a teapot', { status: 418 })
        }
        return new Response('OK', { status: 200 })
      })

      const customRetryableCodes = [418, 429, 500, 502, 503, 504] as const
      const retryableFetch = createRetryableFetch(fetchMock as typeof fetch, {
        maxRetries: 3,
        baseDelayMs: 1,
        jitterMaxMs: 0,
        retryableStatusCodes: customRetryableCodes,
      })

      const result = await retryableFetch('http://example.com')
      expect(result.status).toBe(200)
      expect(attempts).toBe(2)
    })
  })
})
