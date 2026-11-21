import { afterEach, describe, expect, it } from 'bun:test'
import { generateText, jsonSchema, streamText } from 'ai'
import { createLanguageModel } from '../../src/agent/provider-factory'
import { buildAgentReasoningConfig } from '../../src/agent/reasoning-config'
import type { ResolvedAgentConfig } from '../../src/agent/types'
import { hydrateChatProvider } from '../../src/api/services/chat-provider-config'
import type { BrowserOsChatRequest } from '../../src/api/types'
import { createLLMProvider } from '../../src/lib/clients/llm/provider'
import type { ResolvedLLMConfig } from '../../src/lib/clients/llm/types'
import type { ProviderRow } from '../../src/lib/db/schema'

const flash = 'minimax/minimax-m3.1-flash:free'
const conversationId = '00000000-0000-4000-8000-000000000001'
interface CapturedBody {
  model?: string
  reasoning?: unknown
  messages: Array<{ content: unknown }>
  tools: Array<{ function: { name: string } }>
}
const requests: Array<{ url: string; body: CapturedBody }> = []
const originalFetch = globalThis.fetch

function interceptRequests() {
  requests.length = 0
  globalThis.fetch = (async (input, init) => {
    const url = String(input)
    const body = JSON.parse(String(init?.body))
    requests.push({ url, body })
    if (url.includes('bedrock-runtime.')) {
      return Response.json({
        output: { message: { role: 'assistant', content: [{ text: 'ok' }] } },
        stopReason: 'end_turn',
        usage: { inputTokens: 1, outputTokens: 1, totalTokens: 2 },
        metrics: { latencyMs: 1 },
      })
    }
    if (body.stream) {
      const chunk = {
        id: 'offline-completion',
        object: 'chat.completion.chunk',
        created: 1,
        model: body.model,
        choices: [
          { index: 0, delta: { content: 'ok' }, finish_reason: 'stop' },
        ],
      }
      return new Response(
        `data: ${JSON.stringify(chunk)}\n\ndata: [DONE]\n\n`,
        {
          headers: { 'Content-Type': 'text/event-stream' },
        },
      )
    }
    return Response.json({
      id: 'offline-completion',
      object: 'chat.completion',
      created: 1,
      model: body.model,
      choices: [
        {
          index: 0,
          message: { role: 'assistant', content: 'ok' },
          finish_reason: 'stop',
        },
      ],
      usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
    })
  }) as typeof fetch
}

afterEach(() => {
  globalThis.fetch = originalFetch
})

const pipelines = {
  chat: async (config: ResolvedLLMConfig) =>
    (await createLanguageModel({ ...config, conversationId })).model,
  'test-provider': async (config: ResolvedLLMConfig) =>
    createLLMProvider(config),
}

async function send(
  pipeline: keyof typeof pipelines,
  config: ResolvedLLMConfig,
) {
  const model = await pipelines[pipeline](config)
  const reasoning = buildAgentReasoningConfig({
    ...config,
    conversationId,
  } as ResolvedAgentConfig)
  const settings = {
    model,
    prompt: 'hello',
    maxOutputTokens: 64,
    maxRetries: 0,
    ...reasoning,
  }
  if (pipeline === 'chat' && config.provider === 'openrouter') {
    const text = await streamText(settings).text
    expect(text).toBe('ok')
    return text
  }
  return generateText(settings)
}

describe('MiniMax gateway HTTP serialization (real SDKs, offline fetch)', () => {
  for (const pipeline of ['chat', 'test-provider'] as const) {
    for (const effort of [
      undefined,
      'low',
      'medium',
      'high',
      'xhigh',
      'max',
    ] as const) {
      it(`${pipeline}: sends Flash ${effort ?? 'default'} effort to OpenRouter`, async () => {
        interceptRequests()
        await send(pipeline, {
          provider: 'openrouter',
          model: flash,
          apiKey: 'offline-test',
          reasoningEffort: effort,
        })
        expect(requests).toHaveLength(1)
        expect(requests[0]?.url).toBe(
          'https://openrouter.ai/api/v1/chat/completions',
        )
        expect(requests[0]?.body).toMatchObject({
          model: flash,
          reasoning: { effort: effort ?? 'max' },
        })
        expect(requests[0]?.body).not.toHaveProperty('reasoning_effort')
        expect(requests[0]?.body).not.toHaveProperty('thinking')
        expect(requests[0]?.body).not.toHaveProperty('service_tier')
      })
    }

    it(`${pipeline}: rejects disabled Flash reasoning before HTTP`, async () => {
      interceptRequests()
      await expect(
        send(pipeline, {
          provider: 'openrouter',
          model: flash,
          apiKey: 'offline-test',
          reasoningEffort: 'none',
        }),
      ).rejects.toThrow('requires low')
      expect(requests).toHaveLength(0)
    })

    for (const model of [
      'minimax/minimax-m3',
      'minimax/minimax-m2.7',
      'private/custom-model',
      'openai/gpt-4o',
    ]) {
      it(`${pipeline}: preserves ${model} and its custom endpoint without Flash options`, async () => {
        interceptRequests()
        await send(pipeline, {
          provider: 'openrouter',
          model,
          apiKey: 'offline-test',
          baseUrl: 'https://gateway.example/v1',
          reasoningEffort: 'high',
        })
        expect(requests[0]?.url).toBe(
          'https://gateway.example/v1/chat/completions',
        )
        expect(requests[0]?.body.model).toBe(model)
        expect(requests[0]?.body.reasoning ?? {}).toEqual({})
      })
    }

    for (const region of ['us-east-1', 'eu-west-1']) {
      it(`${pipeline}: preserves a saved Bedrock legacy ID in ${region}`, async () => {
        interceptRequests()
        await send(pipeline, {
          provider: 'bedrock',
          model: 'minimax.minimax-m2.5',
          region,
          accessKeyId: 'OFFLINEACCESSKEY',
          secretAccessKey: 'offline-secret',
          reasoningEffort: 'high',
        })
        expect(requests[0]?.url).toBe(
          `https://bedrock-runtime.${region}.amazonaws.com/model/minimax.minimax-m2.5/converse`,
        )
        expect(requests[0]?.body).not.toHaveProperty('reasoning')
        expect(requests[0]?.body).not.toHaveProperty(
          'additionalModelRequestFields',
        )
      })
    }
  }

  it('keeps image input and tool definitions in the OpenRouter payload', async () => {
    interceptRequests()
    const model = await pipelines.chat({
      provider: 'openrouter',
      model: flash,
      apiKey: 'offline-test',
    })
    await generateText({
      model,
      messages: [
        {
          role: 'user',
          content: [
            { type: 'text', text: 'Describe' },
            {
              type: 'file',
              data: new URL('https://example.com/image.png'),
              mediaType: 'image/png',
            },
          ],
        },
      ],
      tools: {
        inspect: {
          description: 'Inspect',
          inputSchema: jsonSchema({ type: 'object', properties: {} }),
        },
      },
      maxRetries: 0,
    })
    expect(requests[0]?.body.messages[0].content).toContainEqual({
      type: 'image_url',
      image_url: { url: 'https://example.com/image.png' },
    })
    expect(requests[0]?.body.tools[0].function.name).toBe('inspect')
  })

  for (const modelId of [
    flash,
    'minimax/minimax-m2.7',
    'private/custom-model',
  ]) {
    it(`hydrates the saved default ${modelId} through the chat request boundary`, async () => {
      interceptRequests()
      const saved: ProviderRow = {
        id: 'saved',
        kind: 'llm',
        type: 'openrouter',
        name: 'Saved',
        modelId,
        isDefault: true,
        createdAt: 1,
        updatedAt: 1,
        baseUrl: 'https://saved.example/v1',
        apiKey: 'offline-test',
        headers: null,
        reasoningEffort: null,
        reasoningSummary: null,
        supportsImages: true,
        contextWindow: 1000000,
        temperature: null,
        accessKeyId: null,
        secretAccessKey: null,
        sessionToken: null,
        region: null,
        resourceName: null,
        workingDirectory: null,
        customConfig: null,
      }
      const hydrated = await hydrateChatProvider(
        {
          conversationId,
          message: 'hello',
          target: { type: 'browseros' },
        } as BrowserOsChatRequest,
        { get: async () => saved, getDefault: async () => saved },
      )
      expect(hydrated.ok).toBe(true)
      if (!hydrated.ok) throw new Error(hydrated.error)
      if (!hydrated.request.model)
        throw new Error('Saved model was not hydrated')
      await send('chat', {
        ...hydrated.request,
        provider: 'openrouter',
        model: hydrated.request.model,
      })
      expect(requests[0]?.url).toBe('https://saved.example/v1/chat/completions')
      expect(requests[0]?.body.model).toBe(modelId)
      expect(requests[0]?.body.reasoning ?? {}).toEqual(
        modelId === flash ? { effort: 'max' } : {},
      )
    })
  }
})
