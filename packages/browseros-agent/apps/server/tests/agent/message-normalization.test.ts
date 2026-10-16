import { describe, expect, test } from 'bun:test'
import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import type { ModelMessage, ToolResultPart } from 'ai'
import {
  getMessageNormalizationOptions,
  normalizeMessagesForModel,
  stripReasoningFromModelMessages,
} from '../../src/agent/message-normalization'
import { serializesReasoningContent } from '../../src/agent/provider-factory'
import type { ResolvedAgentConfig } from '../../src/agent/types'

type ToolResultContentPart = Extract<
  ToolResultPart['output'],
  { type: 'content' }
>['value'][number]

function cfg(
  overrides: Partial<ResolvedAgentConfig> = {},
): ResolvedAgentConfig {
  return {
    conversationId: 'c1',
    provider: LLM_PROVIDERS.OPENROUTER,
    model: 'deepseek/deepseek-v4.1-flash',
    ...overrides,
  }
}

const BASE64 = 'aW1hZ2UtYnl0ies'

// A screenshot tool result as the AI SDK MCP client actually emits it: the
// canonical v7 `file` content part with inline tagged data.
function screenshotToolMessage(part: ToolResultContentPart): ModelMessage {
  return {
    role: 'tool',
    content: [
      {
        type: 'tool-result',
        toolCallId: 't1',
        toolName: 'screenshot',
        output: { type: 'content', value: [part] },
      },
    ],
  }
}

const CUSTOM_PROVIDER_OPTS = {
  supportsImages: true,
  supportsMediaInToolResults: false,
}

describe('getMessageNormalizationOptions', () => {
  test('custom providers (OpenRouter) cannot carry media inside tool results', () => {
    expect(getMessageNormalizationOptions(cfg())).toEqual({
      supportsImages: true,
      supportsMediaInToolResults: false,
    })
  })

  test('providers with native tool-result media keep it inline', () => {
    expect(
      getMessageNormalizationOptions(
        cfg({ provider: LLM_PROVIDERS.ANTHROPIC }),
      ),
    ).toEqual({ supportsImages: true, supportsMediaInToolResults: true })
  })

  test('supportsImages only disables on an explicit false', () => {
    expect(
      getMessageNormalizationOptions(cfg({ supportsImages: false }))
        .supportsImages,
    ).toBe(false)
    expect(
      getMessageNormalizationOptions(cfg({ supportsImages: undefined }))
        .supportsImages,
    ).toBe(true)
  })
})

describe('normalizeMessagesForModel', () => {
  test('re-attaches a canonical `file` image (screenshot) as a user message (#2722)', () => {
    const messages = [
      screenshotToolMessage({
        type: 'file',
        data: { type: 'data', data: BASE64 },
        mediaType: 'image/jpeg',
      }),
    ]

    const out = normalizeMessagesForModel(messages, CUSTOM_PROVIDER_OPTS)

    expect(out).toHaveLength(2)
    // tool result keeps a text placeholder, no longer the raw image
    const toolResult = (out[0] as { content: Array<{ output: unknown }> })
      .content[0].output
    expect(toolResult).toEqual({ type: 'text', value: '[Image]' })
    // the image rides in a following user message so the model can see it
    const userMessage = out[1] as {
      role: string
      content: Array<Record<string, unknown>>
    }
    expect(userMessage.role).toBe('user')
    expect(userMessage.content).toContainEqual({
      type: 'image',
      image: BASE64,
      mediaType: 'image/jpeg',
    })
  })

  test('a non-image `file` part is re-attached as a file', () => {
    const messages = [
      screenshotToolMessage({
        type: 'file',
        data: { type: 'data', data: BASE64 },
        mediaType: 'application/pdf',
        filename: 'report.pdf',
      }),
    ]

    const out = normalizeMessagesForModel(messages, CUSTOM_PROVIDER_OPTS)

    const userMessage = out[1] as { content: Array<Record<string, unknown>> }
    expect(userMessage.content).toContainEqual({
      type: 'file',
      data: BASE64,
      mediaType: 'application/pdf',
      filename: 'report.pdf',
    })
  })

  test('the deprecated `image-data` shape still works', () => {
    const messages = [
      screenshotToolMessage({
        type: 'image-data',
        data: BASE64,
        mediaType: 'image/png',
      }),
    ]

    const out = normalizeMessagesForModel(messages, CUSTOM_PROVIDER_OPTS)

    expect(out).toHaveLength(2)
    const userMessage = out[1] as { content: Array<Record<string, unknown>> }
    expect(userMessage.content).toContainEqual({
      type: 'image',
      image: BASE64,
      mediaType: 'image/png',
    })
  })

  test('a non-inline `file` (url) is stripped but not re-attached', () => {
    const messages = [
      screenshotToolMessage({
        type: 'file',
        data: { type: 'url', url: new URL('https://example.com/x.png') },
        mediaType: 'image/png',
      }),
    ]

    const out = normalizeMessagesForModel(messages, CUSTOM_PROVIDER_OPTS)

    expect(out).toHaveLength(1)
    expect(out[0].role).toBe('tool')
  })

  test('providers with native tool-result media pass through untouched', () => {
    const messages = [
      screenshotToolMessage({
        type: 'file',
        data: { type: 'data', data: BASE64 },
        mediaType: 'image/jpeg',
      }),
    ]

    const out = normalizeMessagesForModel(messages, {
      supportsImages: true,
      supportsMediaInToolResults: true,
    })

    expect(out).toBe(messages)
  })

  test('when the model has no vision, media is stripped and not re-attached', () => {
    const messages = [
      screenshotToolMessage({
        type: 'file',
        data: { type: 'data', data: BASE64 },
        mediaType: 'image/jpeg',
      }),
    ]

    const out = normalizeMessagesForModel(messages, {
      supportsImages: false,
      supportsMediaInToolResults: false,
    })

    expect(out).toHaveLength(1)
    expect(out[0].role).toBe('tool')
  })
})

function assistantReasoningPartCount(message: ModelMessage): number {
  if (message.role !== 'assistant' || typeof message.content === 'string') {
    return 0
  }
  return message.content.filter((part) => part.type === 'reasoning').length
}

describe('stripReasoningFromModelMessages', () => {
  // The exact #2921 shape: a mid-run assistant turn carrying GLM reasoning plus
  // the tool call. Replaying the reasoning as `reasoning_content` is what strict
  // gateways (Mistral) reject with 422.
  test('drops the reasoning part but keeps the tool call', () => {
    const messages: ModelMessage[] = [
      { role: 'user', content: 'search for cats' },
      {
        role: 'assistant',
        content: [
          { type: 'reasoning', text: 'I should call the search tool' },
          {
            type: 'tool-call',
            toolCallId: 't1',
            toolName: 'search',
            input: { q: 'cats' },
          },
        ],
      },
    ]

    const out = stripReasoningFromModelMessages(messages)

    expect(out).toHaveLength(2)
    expect(assistantReasoningPartCount(out[1])).toBe(0)
    const assistant = out[1] as { content: Array<Record<string, unknown>> }
    expect(assistant.content).toEqual([
      {
        type: 'tool-call',
        toolCallId: 't1',
        toolName: 'search',
        input: { q: 'cats' },
      },
    ])
  })

  test('keeps assistant text when reasoning is stripped', () => {
    const messages: ModelMessage[] = [
      {
        role: 'assistant',
        content: [
          { type: 'reasoning', text: 'thinking out loud' },
          { type: 'text', text: 'Here is the answer.' },
        ],
      },
    ]

    const out = stripReasoningFromModelMessages(messages)

    const assistant = out[0] as { content: Array<Record<string, unknown>> }
    expect(assistant.content).toEqual([
      { type: 'text', text: 'Here is the answer.' },
    ])
  })

  test('drops a reasoning-only assistant turn entirely', () => {
    const messages: ModelMessage[] = [
      { role: 'user', content: 'hi' },
      {
        role: 'assistant',
        content: [{ type: 'reasoning', text: 'no visible output' }],
      },
    ]

    const out = stripReasoningFromModelMessages(messages)

    expect(out).toHaveLength(1)
    expect(out[0].role).toBe('user')
  })

  test('leaves user, tool, and string-content assistant messages untouched', () => {
    const messages: ModelMessage[] = [
      { role: 'system', content: 'be helpful' },
      { role: 'user', content: 'question' },
      { role: 'assistant', content: 'plain string answer' },
      {
        role: 'tool',
        content: [
          {
            type: 'tool-result',
            toolCallId: 't1',
            toolName: 'search',
            output: { type: 'text', value: 'done' },
          },
        ],
      },
    ]

    const out = stripReasoningFromModelMessages(messages)

    // No assistant message had a reasoning part, so nothing changed: same
    // reference back, like normalizeMessagesForModel.
    expect(out).toBe(messages)
  })

  test('returns the same array when there is no reasoning to strip', () => {
    const messages: ModelMessage[] = [
      {
        role: 'assistant',
        content: [
          {
            type: 'tool-call',
            toolCallId: 't1',
            toolName: 'search',
            input: { q: 'x' },
          },
        ],
      },
    ]

    expect(stripReasoningFromModelMessages(messages)).toBe(messages)
  })

  // Guards the #2921 regression end to end: this is the per-step pipeline
  // prepareStep applies on every step (normalize, then strip reasoning only when
  // the provider serializes reasoning_content). Step one returned reasoning plus
  // a tool call; step two must send the tool call WITHOUT reasoning to an
  // openai-compatible provider, but keep reasoning for a native provider that
  // replays it.
  test('step two drops replayed reasoning for openai-compatible but keeps it for a native provider (#2921)', () => {
    const stepTwoInbound: ModelMessage[] = [
      { role: 'user', content: 'search for cats' },
      {
        role: 'assistant',
        content: [
          { type: 'reasoning', text: 'call the search tool' },
          {
            type: 'tool-call',
            toolCallId: 't1',
            toolName: 'search',
            input: { q: 'cats' },
          },
        ],
      },
      {
        role: 'tool',
        content: [
          {
            type: 'tool-result',
            toolCallId: 't1',
            toolName: 'search',
            output: { type: 'text', value: 'found' },
          },
        ],
      },
    ]

    const perStep = (provider: string): ModelMessage[] => {
      const normalized = normalizeMessagesForModel(
        stepTwoInbound,
        getMessageNormalizationOptions(cfg({ provider })),
      )
      return serializesReasoningContent(provider)
        ? stripReasoningFromModelMessages(normalized)
        : normalized
    }

    const assistantPartTypes = (messages: ModelMessage[]): string[] => {
      const assistant = messages.find((m) => m.role === 'assistant')
      if (!assistant || typeof assistant.content === 'string') return []
      return assistant.content.map((part) => part.type)
    }

    expect(
      assistantPartTypes(perStep(LLM_PROVIDERS.OPENAI_COMPATIBLE)),
    ).toEqual(['tool-call'])
    expect(assistantPartTypes(perStep(LLM_PROVIDERS.ANTHROPIC))).toContain(
      'reasoning',
    )
  })
})
