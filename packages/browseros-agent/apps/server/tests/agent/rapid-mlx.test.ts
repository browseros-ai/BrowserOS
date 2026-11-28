import { afterEach, expect, it } from 'bun:test'
import { generateText, streamText } from 'ai'
import { createLanguageModel } from '../../src/agent/provider-factory'
import { createLLMProvider } from '../../src/lib/clients/llm/provider'

const originalFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = originalFetch
})

it('routes both Rapid-MLX clients through chat completions without credentials', async () => {
  const requests: string[] = []
  globalThis.fetch = (async (url, init) => {
    requests.push(String(url))
    const body = JSON.parse(String(init?.body))
    expect(body.model).toBe('local-model')
    expect(new Headers(init?.headers).has('authorization')).toBe(false)
    if (body.stream) {
      return new Response(
        'data: {"id":"test","choices":[{"index":0,"delta":{"content":"hello"},"finish_reason":null}]}\n\n' +
          'data: {"id":"test","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}\n\n' +
          'data: [DONE]\n\n',
        { headers: { 'content-type': 'text/event-stream' } },
      )
    }
    return Response.json({
      id: 'test',
      choices: [
        {
          index: 0,
          message: { role: 'assistant', content: 'hello' },
          finish_reason: 'stop',
        },
      ],
      usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
    })
  }) as typeof fetch
  const config = {
    provider: 'rapid-mlx' as const,
    model: 'local-model',
    baseUrl: 'http://localhost:8000/v1',
  }
  const { model } = await createLanguageModel({
    ...config,
    conversationId: 'test',
  })
  expect((await generateText({ model, prompt: 'hello' })).text).toBe('hello')
  const streamed = streamText({
    model: createLLMProvider(config),
    prompt: 'hello',
  })
  let text = ''
  for await (const chunk of streamed.textStream) text += chunk
  expect(text).toBe('hello')
  expect(requests).toEqual(
    Array(2).fill('http://localhost:8000/v1/chat/completions'),
  )
})
