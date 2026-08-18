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

  it('redacts key-shaped tokens and caps details', () => {
    const error = toChatError(
      apiCallError({
        message: `rejected key sk-${'a'.repeat(40)} for org`,
        statusCode: 401,
      }),
    )

    expect(error.details).toContain('[REDACTED]')
    expect(error.details).not.toContain('aaaaaaaaaa')
    expect((error.details ?? '').length).toBeLessThanOrEqual(501)
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
