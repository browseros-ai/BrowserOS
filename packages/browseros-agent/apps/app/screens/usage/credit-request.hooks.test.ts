import { afterEach, beforeAll, describe, expect, it, mock } from 'bun:test'
import type { CreditRequestInput } from './credit-request.hooks'

mock.module('@/lib/sentry/sentry', () => ({
  sentry: { captureException: () => {} },
}))

// A sentinel origin, so the URL assertion proves the request is built from the
// constant rather than passing on a hardcoded copy of the real host.
const GATEWAY_URL = 'https://gateway.test'

mock.module('@/lib/constants/gatewayUrl', () => ({
  BROWSEROS_GATEWAY_URL: GATEWAY_URL,
}))

let submitCreditRequest: (input: CreditRequestInput) => Promise<void>
let rateLimitedMessage: string
let failedMessage: string

const originalFetch = globalThis.fetch
const input: CreditRequestInput = {
  browserosId: 'c481ffe7-e00a-4412-8070-696a45f444a1',
  discordHandle: 'someuser',
}

beforeAll(async () => {
  const module = await import('./credit-request.hooks')
  submitCreditRequest = module.submitCreditRequest
  rateLimitedMessage = module.CREDIT_REQUEST_RATE_LIMITED_MESSAGE
  failedMessage = module.CREDIT_REQUEST_FAILED_MESSAGE
})

afterEach(() => {
  globalThis.fetch = originalFetch
})

function stubFetch(respond: () => Response | Promise<Response>) {
  const calls: { url: string; init?: RequestInit }[] = []
  globalThis.fetch = (async (url: string, init?: RequestInit) => {
    calls.push({ url, init })
    return respond()
  }) as typeof fetch
  return calls
}

describe('submitCreditRequest', () => {
  it('posts the request to the gateway and resolves on success', async () => {
    const calls = stubFetch(() => new Response(null, { status: 200 }))

    await submitCreditRequest(input)

    expect(calls).toHaveLength(1)
    expect(calls[0]?.url).toBe(`${GATEWAY_URL}/credit-request`)
    expect(calls[0]?.init?.method).toBe('POST')
    expect(JSON.parse(String(calls[0]?.init?.body))).toEqual(input)
  })

  it('resolves on any ok status, so a suppressed duplicate still reads as sent', async () => {
    stubFetch(() => new Response(null, { status: 204 }))

    await submitCreditRequest(input)
  })

  it('reports rate limiting separately from other failures', async () => {
    stubFetch(() => new Response(null, { status: 429 }))

    await expect(submitCreditRequest(input)).rejects.toThrow(rateLimitedMessage)
  })

  it('reports invalid input as a generic failure', async () => {
    stubFetch(() => new Response(null, { status: 400 }))

    await expect(submitCreditRequest(input)).rejects.toThrow(failedMessage)
  })

  it('reports a network error as a generic failure', async () => {
    stubFetch(() => {
      throw new TypeError('Failed to fetch')
    })

    await expect(submitCreditRequest(input)).rejects.toThrow(failedMessage)
  })
})
