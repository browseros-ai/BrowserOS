import { afterEach, describe, expect, it } from 'bun:test'
import { LLMConfigSchema } from '@browseros/shared/schemas/llm'
import { generateText, streamText } from 'ai'
import { createLanguageModel } from '../../src/agent/provider-factory'
import { createLLMProvider } from '../../src/lib/clients/llm/provider'

const originalFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = originalFetch
})

describe('provider headers', () => {
  it('keeps header templates in the request schema', () => {
    const headers = { 'x-opencode-session': '{{conversationId}}' }
    expect(
      LLMConfigSchema.parse({ provider: 'openai-compatible', headers }).headers,
    ).toEqual(headers)
  })

  it.each([
    { 'bad name': 'value' },
    { 'x-test\n': 'value' },
    { 'x-test': 'value\n' },
    { 'x-test': 'value\r' },
    { 'x-test': 'value\u0000' },
    { 'x-test': 'injected\r\nAuthorization: secret' },
    { 'x-test': 'a', 'X-Test': 'b' },
    { 'x-test': 123 },
  ])('rejects invalid headers: %j', (headers) => {
    expect(
      LLMConfigSchema.safeParse({ provider: 'openai-compatible', headers })
        .success,
    ).toBe(false)
  })

  it('sends stable conversation headers on streaming, follow-up, and retry requests', async () => {
    const requests: Headers[] = []
    globalThis.fetch = (async (_url, init) => {
      requests.push(new Headers(init?.headers))
      if (requests.length === 1) return new Response('retry', { status: 503 })
      return new Response(
        'data: {"id":"test","object":"chat.completion.chunk","created":1,"model":"test-model","choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":null}]}\n\n' +
          'data: {"id":"test","object":"chat.completion.chunk","created":1,"model":"test-model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}\n\n' +
          'data: [DONE]\n\n',
        { headers: { 'Content-Type': 'text/event-stream' } },
      )
    }) as typeof fetch
    const headers = {
      'x-opencode-session': '{{conversationId}}',
      'x-custom': 'literal',
    }
    for (const conversationId of [
      'conversation-a',
      'conversation-a',
      'conversation-b',
    ]) {
      const { model } = await createLanguageModel({
        conversationId,
        provider: 'openai-compatible',
        model: 'test-model',
        apiKey: 'local-test-key',
        baseUrl: 'http://127.0.0.1:1234/v1',
        headers,
      })
      expect(
        await streamText({ model, prompt: 'hello', maxRetries: 1 }).text,
      ).toBe('ok')
    }
    expect(
      requests.map((request) => request.get('x-opencode-session')),
    ).toEqual([
      'conversation-a',
      'conversation-a',
      'conversation-a',
      'conversation-b',
    ])
    for (const request of requests) {
      expect(request.get('x-custom')).toBe('literal')
      expect(request.get('authorization')).toBe('Bearer local-test-key')
    }
    expect(headers['x-opencode-session']).toBe('{{conversationId}}')
  })

  it('gives non-chat operations a unique session that stays stable across requests', async () => {
    const sessions: (string | null)[] = []
    globalThis.fetch = (async (_url, init) => {
      sessions.push(new Headers(init?.headers).get('x-opencode-session'))
      return Response.json({
        id: 'test',
        object: 'chat.completion',
        created: 1,
        model: 'test-model',
        choices: [
          {
            index: 0,
            message: { role: 'assistant', content: 'ok' },
            finish_reason: 'stop',
          },
        ],
      })
    }) as typeof fetch
    const config = {
      provider: 'openai-compatible' as const,
      model: 'test-model',
      baseUrl: 'http://127.0.0.1:1234/v1',
      headers: { 'x-opencode-session': '{{conversationId}}' },
    }
    const model = createLLMProvider(config)
    await generateText({ model, prompt: 'test' })
    await generateText({ model, prompt: 'retry' })
    await generateText({ model: createLLMProvider(config), prompt: 'new test' })
    expect(sessions[0]).toMatch(/^[0-9a-f-]{36}$/)
    expect(sessions[1]).toBe(sessions[0])
    expect(sessions[2]).not.toBe(sessions[0])
  })
})
