import { describe, expect, it } from 'bun:test'
import { APICallError, LoadAPIKeyError } from '@ai-sdk/provider'
import { parseChatErrorEnvelope } from '@browseros/shared/schemas/chat-error'
import { RetryError } from 'ai'
import { toChatError, toChatErrorText } from '../../src/agent/chat-error'

function apiCallError(
  overrides: Partial<ConstructorParameters<typeof APICallError>[0]> = {},
): APICallError {
  return new APICallError({
    message: 'upstream failed',
    url: 'https://llm.browseros.com/v1/chat/completions',
    requestBodyValues: {},
    ...overrides,
  })
}

describe('toChatError', () => {
  it('classifies gateway credit exhaustion from the structured code', () => {
    const error = toChatError(
      apiCallError({
        message: 'Daily credits exhausted',
        statusCode: 429,
        isRetryable: false,
        data: { code: 'CREDITS_EXHAUSTED' },
      }),
      { provider: 'browseros' },
    )

    expect(error.code).toBe('credits_exhausted')
    expect(error.title).toBe('Daily limit reached')
    expect(error.message).toBe('Daily credits exhausted')
    expect(error.retryable).toBe(false)
    expect(error.statusCode).toBe(429)
    expect(error.provider).toBe('browseros')
  })

  it('recovers the gateway code from responseBody when data is absent', () => {
    const error = toChatError(
      apiCallError({
        message: 'quota gone',
        statusCode: 429,
        responseBody: JSON.stringify({
          error: { code: 'CREDITS_EXHAUSTED', message: 'quota gone' },
        }),
      }),
      { provider: 'browseros' },
    )

    expect(error.code).toBe('credits_exhausted')
    expect(error.retryable).toBe(false)
  })

  it('treats a plain 429 as retryable rate limiting, not credit exhaustion', () => {
    const error = toChatError(
      apiCallError({ message: 'slow down', statusCode: 429 }),
      { provider: 'anthropic' },
    )

    expect(error.code).toBe('rate_limited')
    expect(error.retryable).toBe(true)
  })

  it('reads Retry-After off the response headers', () => {
    const error = toChatError(
      apiCallError({
        statusCode: 429,
        responseHeaders: { 'retry-after': '30' },
      }),
    )

    expect(error.retryAfterSeconds).toBe(30)
  })

  it.each([401, 403])('classifies %i as auth failure', (statusCode) => {
    const error = toChatError(
      apiCallError({ message: 'invalid api key', statusCode }),
    )

    expect(error.code).toBe('auth_failed')
    expect(error.retryable).toBe(false)
  })

  it('classifies 5xx as a transient provider outage', () => {
    const error = toChatError(
      apiCallError({ message: 'overloaded', statusCode: 503 }),
    )

    expect(error.code).toBe('provider_unavailable')
    expect(error.retryable).toBe(true)
  })

  it('unwraps RetryError and classifies the last failure', () => {
    const last = apiCallError({ message: 'still limited', statusCode: 429 })
    const error = toChatError(
      new RetryError({
        message: 'maxRetriesExceeded',
        reason: 'maxRetriesExceeded',
        errors: [last, last],
      }),
      { provider: 'browseros' },
    )

    expect(error.code).toBe('rate_limited')
    expect(error.statusCode).toBe(429)
  })

  it('classifies provider construction failures as configuration errors', () => {
    const error = toChatError(new Error('Anthropic provider requires apiKey'), {
      provider: 'anthropic',
    })

    expect(error.code).toBe('provider_config')
    expect(error.message).toBe('Anthropic provider requires apiKey')
    expect(error.retryable).toBe(false)
  })

  it('classifies a missing key error from the SDK class', () => {
    const error = toChatError(
      new LoadAPIKeyError({ message: 'OpenAI API key is missing' }),
    )

    expect(error.code).toBe('provider_config')
    expect(error.retryable).toBe(false)
  })

  it('classifies local connection failures', () => {
    const error = toChatError(new TypeError('fetch failed'))

    expect(error.code).toBe('connection_failed')
    expect(error.retryable).toBe(true)
  })

  it('falls back to unknown while still carrying the real message', () => {
    const error = toChatError(new Error('something exotic broke'))

    expect(error.code).toBe('unknown')
    expect(error.message).toBe('something exotic broke')
  })

  it('redacts key-shaped tokens out of the user-facing message', () => {
    const error = toChatError(
      apiCallError({
        message: `Incorrect API key provided: sk-${'b'.repeat(40)}`,
        statusCode: 401,
      }),
    )

    expect(error.message).toContain('[REDACTED]')
    expect(error.message).not.toContain('bbbbbbbbbb')
  })

  it('redacts key-shaped tokens out of details', () => {
    const error = toChatError(
      apiCallError({
        message: `rejected key sk-${'a'.repeat(40)} for org`,
        statusCode: 401,
      }),
    )

    expect(error.details).toContain('[REDACTED]')
    expect(error.details).not.toContain('aaaaaaaaaa')
  })

  it('carries the full upstream response body as pretty-printed details', () => {
    const error = toChatError(
      apiCallError({
        message: 'Provider returned error',
        statusCode: 429,
        data: { code: 'CREDITS_EXHAUSTED' },
        responseBody: JSON.stringify({
          error: {
            code: 'CREDITS_EXHAUSTED',
            message: 'Provider returned error',
            metadata: { raw: 'quota 0 of 100 for today' },
          },
        }),
      }),
      { provider: 'browseros' },
    )

    // The real reason the generic message hid is preserved in details...
    expect(error.details).toContain('quota 0 of 100 for today')
    // ...pretty-printed onto its own lines.
    expect(error.details).toContain('\n')
  })

  it('falls back to structured data.raw when there is no response body', () => {
    const error = toChatError(
      apiCallError({
        message: 'Provider returned error',
        statusCode: 429,
        data: { code: 'CREDITS_EXHAUSTED', raw: { remaining: 0 } },
      }),
      { provider: 'browseros' },
    )

    expect(error.details).toContain('remaining')
    expect(error.details).toContain('0')
  })

  it('redacts secrets and bounds a pathological raw body', () => {
    const secret = `sk-${'c'.repeat(40)}`
    const error = toChatError(
      apiCallError({
        statusCode: 500,
        responseBody: JSON.stringify({
          error: { message: `boom ${secret}`, note: 'x'.repeat(30000) },
        }),
      }),
    )

    expect(error.details).toContain('[REDACTED]')
    expect(error.details).not.toContain('cccccccccc')
    // Bounded so a pathological body cannot flood the wire or the card.
    expect((error.details ?? '').length).toBeLessThanOrEqual(20001)
  })

  it('surfaces a plain error message as copyable details', () => {
    const error = toChatError(
      new Error('Anthropic session expired. Please re-login.'),
      { provider: 'anthropic' },
    )

    expect(error.code).toBe('unknown')
    expect(error.details).toContain('session expired')
  })

  it('redacts a JWT sitting inside a non-sensitive field', () => {
    // Assembled from segments so the full token is never a literal in source
    // (scanners match the whole token; the regex sees the runtime value).
    const jwt = [
      'eyJhbGciOiJIUzI1NiJ9',
      'eyJzdWIiOiIxMjM0NTY3ODkwIn0',
      'SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJVadQssw5c',
    ].join('.')
    const error = toChatError(
      apiCallError({
        statusCode: 500,
        responseBody: JSON.stringify({
          error: { message: `auth failed: ${jwt}` },
        }),
      }),
    )

    expect(error.details).toContain('[REDACTED]')
    expect(error.details).not.toContain('eyJhbGci')
  })

  it('redacts a sensitive JSON value by its key on the raw text', () => {
    const error = toChatError(
      apiCallError({
        statusCode: 401,
        responseBody: JSON.stringify({
          access_token: 'ATsecret-value-9f8e7d6c5b4a',
        }),
      }),
    )

    expect(error.details).toContain('[REDACTED]')
    expect(error.details).not.toContain('ATsecret-value')
  })

  it('redacts URL userinfo while keeping the host', () => {
    const password = 'hunter2'.repeat(2)
    const error = toChatError(
      new Error(
        `proxy failed at https://svc:${password}@proxy.internal:8080/v1`,
      ),
    )

    expect(error.details).toContain('[REDACTED]')
    expect(error.details).not.toContain(password)
    // Host stays so the error is still diagnosable.
    expect(error.details).toContain('proxy.internal')
  })

  it('redacts vendor token shapes (Google, Slack)', () => {
    // Prefixes split from bodies so no full token literal appears in source.
    const google = 'AIza'.concat('A'.repeat(35))
    const slack = 'xoxb-'.concat('1234567890-abcdefghijklmnop')
    const error = toChatError(
      apiCallError({
        statusCode: 403,
        responseBody: JSON.stringify({
          error: { message: `google ${google} slack ${slack}` },
        }),
      }),
    )

    expect(error.details).not.toContain(google)
    expect(error.details).not.toContain(slack)
  })
})

describe('toChatErrorText', () => {
  it('round-trips through the shared envelope parser', () => {
    const text = toChatErrorText(
      apiCallError({
        message: 'Daily credits exhausted',
        statusCode: 429,
        isRetryable: false,
        data: { code: 'CREDITS_EXHAUSTED' },
      }),
      { provider: 'browseros' },
    )

    const parsed = parseChatErrorEnvelope(text)

    expect(parsed).not.toBeNull()
    expect(parsed?.code).toBe('credits_exhausted')
    expect(parsed?.retryable).toBe(false)
    expect(parsed?.provider).toBe('browseros')
  })

  it('produces a string that is safe to put in errorText', () => {
    const text = toChatErrorText(new Error('boom'))

    expect(typeof text).toBe('string')
    expect(() => JSON.parse(text)).not.toThrow()
  })
})

describe('parseChatErrorEnvelope', () => {
  it('rejects prose', () => {
    expect(parseChatErrorEnvelope('An error occurred.')).toBeNull()
  })

  it('rejects JSON that is not an envelope', () => {
    expect(
      parseChatErrorEnvelope(JSON.stringify({ error: { message: 'hi' } })),
    ).toBeNull()
  })

  it('rejects an unknown code so older clients fall back cleanly', () => {
    expect(
      parseChatErrorEnvelope(
        JSON.stringify({
          error: { code: 'invented_code', title: 'x', message: 'y' },
        }),
      ),
    ).toBeNull()
  })
})
