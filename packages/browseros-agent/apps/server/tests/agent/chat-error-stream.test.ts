/**
 * Proves a model failure survives to the wire. The AI SDK masks stream errors
 * unless an onError is supplied, so this asserts the emitted SSE frame carries
 * the classified envelope rather than "An error occurred."
 */

import { describe, expect, it } from 'bun:test'
import { APICallError } from '@ai-sdk/provider'
import { parseChatErrorEnvelope } from '@browseros/shared/schemas/chat-error'
import { createAgentUIStreamResponse, ToolLoopAgent } from 'ai'
import { MockLanguageModelV3 } from 'ai/test'
import { toChatErrorText } from '../../src/agent/chat-error'

function rateLimited(): APICallError {
  return new APICallError({
    message: 'Too many requests',
    url: 'https://openrouter.ai/api/v1/chat/completions',
    requestBodyValues: {},
    statusCode: 429,
    responseBody: JSON.stringify({
      error: { code: 'RATE_LIMITED', message: 'Too many requests' },
    }),
    isRetryable: false,
    data: { code: 'RATE_LIMITED' },
  })
}

async function readErrorFrame(response: Response): Promise<string | null> {
  const body = await response.text()
  for (const line of body.split('\n')) {
    if (!line.startsWith('data: ')) continue
    const payload = line.slice(6)
    if (payload === '[DONE]') continue
    const chunk = JSON.parse(payload)
    if (chunk.type === 'error') return chunk.errorText
  }
  return null
}

function agentThatFails(error: unknown): ToolLoopAgent {
  return new ToolLoopAgent({
    model: new MockLanguageModelV3({
      doStream: async () => {
        throw error
      },
    }),
  })
}

describe('chat stream error propagation', () => {
  it('emits the classified envelope instead of the SDK mask', async () => {
    const response = await createAgentUIStreamResponse({
      agent: agentThatFails(rateLimited()),
      uiMessages: [
        { id: 'u1', role: 'user', parts: [{ type: 'text', text: 'hi' }] },
      ],
      onError: (error: unknown) =>
        toChatErrorText(error, { provider: 'openrouter' }),
    })

    const errorText = await readErrorFrame(response)

    expect(errorText).not.toBeNull()
    expect(errorText).not.toBe('An error occurred.')

    const envelope = parseChatErrorEnvelope(errorText as string)
    expect(envelope?.code).toBe('rate_limited')
    expect(envelope?.message).toBe('Too many requests')
    expect(envelope?.retryable).toBe(true)
    expect(envelope?.statusCode).toBe(429)
    expect(envelope?.provider).toBe('openrouter')
  })

  it('masks the failure when no onError is supplied (the bug being fixed)', async () => {
    const response = await createAgentUIStreamResponse({
      agent: agentThatFails(rateLimited()),
      uiMessages: [
        { id: 'u1', role: 'user', parts: [{ type: 'text', text: 'hi' }] },
      ],
    })

    expect(await readErrorFrame(response)).toBe('An error occurred.')
  })
})
