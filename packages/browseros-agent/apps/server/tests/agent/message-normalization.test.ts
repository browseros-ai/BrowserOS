import { describe, expect, test } from 'bun:test'
import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import type { ModelMessage, ToolResultPart } from 'ai'
import {
  getMessageNormalizationOptions,
  normalizeMessagesForModel,
} from '../../src/agent/message-normalization'
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
