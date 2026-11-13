/**
 * @license
 * Copyright 2025 BrowserOS
 *
 * Message Validation — Test Suite
 *
 * Tests for sanitizeMessagesForToolset, which strips tool parts from
 * carried-over messages when a session is rebuilt with a different toolset
 * (e.g., workspace removed or MCP server disconnected mid-conversation).
 *
 * Without this sanitization, the AI SDK throws a validation error because
 * it finds tool parts in the message history that have no matching schema.
 */

import { describe, expect, it } from 'bun:test'
import {
  convertToModelMessages,
  createAgentUIStream,
  isToolUIPart,
  type ModelMessage,
  ToolLoopAgent,
  tool,
  type UIMessage,
} from 'ai'
import { MockLanguageModelV3 } from 'ai/test'
import { z } from 'zod'
import {
  closeDanglingToolCalls,
  filterValidMessages,
  hasMessageContent,
  sanitizeMessagesForToolset,
  stripReasoningParts,
} from '../../src/agent/message-validation'

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeUserMessage(text: string, id?: string): UIMessage {
  return {
    id: id ?? crypto.randomUUID(),
    role: 'user',
    parts: [{ type: 'text', text }],
  }
}

function makeAssistantMessage(
  parts: UIMessage['parts'],
  id?: string,
): UIMessage {
  return {
    id: id ?? crypto.randomUUID(),
    role: 'assistant',
    parts,
  }
}

// ---------------------------------------------------------------------------
// sanitizeMessagesForToolset
// ---------------------------------------------------------------------------

describe('sanitizeMessagesForToolset', () => {
  const allTools = new Set([
    'navigate_page',
    'click',
    'take_snapshot',
    'filesystem_read',
    'filesystem_write',
    'evaluate_script',
  ])

  const noFilesystemTools = new Set([
    'navigate_page',
    'click',
    'take_snapshot',
    'evaluate_script',
  ])

  it('preserves messages with no tool parts', () => {
    const messages: UIMessage[] = [
      makeUserMessage('Hello'),
      makeAssistantMessage([{ type: 'text', text: 'Hi there!' }]),
    ]

    const result = sanitizeMessagesForToolset(messages, noFilesystemTools)
    expect(result).toHaveLength(2)
    expect(result[0].parts).toHaveLength(1)
    expect(result[1].parts).toHaveLength(1)
  })

  it('preserves tool parts when tool is in the toolset', () => {
    const messages: UIMessage[] = [
      makeAssistantMessage([
        { type: 'text', text: 'Taking a snapshot...' },
        {
          type: 'tool-take_snapshot',
          toolCallId: 'call-1',
          toolName: 'take_snapshot',
          state: 'result',
          input: { page: 1 },
          output: { content: 'snapshot data' },
        } as unknown as UIMessage['parts'][number],
      ]),
    ]

    const result = sanitizeMessagesForToolset(messages, allTools)
    expect(result).toHaveLength(1)
    expect(result[0].parts).toHaveLength(2)
  })

  it('strips tool parts when tool is NOT in the toolset', () => {
    const messages: UIMessage[] = [
      makeAssistantMessage([
        { type: 'text', text: 'Reading file...' },
        {
          type: 'tool-filesystem_read',
          toolCallId: 'call-1',
          toolName: 'filesystem_read',
          state: 'result',
          input: { path: '/tmp/test.txt' },
          output: { content: 'file data' },
        } as unknown as UIMessage['parts'][number],
      ]),
    ]

    const result = sanitizeMessagesForToolset(messages, noFilesystemTools)
    expect(result).toHaveLength(1)
    // Only the text part should remain
    expect(result[0].parts).toHaveLength(1)
    expect(result[0].parts[0].type).toBe('text')
  })

  it('strips multiple removed tool parts from same message', () => {
    const messages: UIMessage[] = [
      makeAssistantMessage([
        { type: 'text', text: 'Working on files...' },
        {
          type: 'tool-filesystem_read',
          toolCallId: 'call-1',
          toolName: 'filesystem_read',
          state: 'result',
          input: { path: '/tmp/a.txt' },
          output: {},
        } as unknown as UIMessage['parts'][number],
        {
          type: 'tool-filesystem_write',
          toolCallId: 'call-2',
          toolName: 'filesystem_write',
          state: 'result',
          input: { path: '/tmp/b.txt', content: 'data' },
          output: {},
        } as unknown as UIMessage['parts'][number],
      ]),
    ]

    const result = sanitizeMessagesForToolset(messages, noFilesystemTools)
    expect(result).toHaveLength(1)
    expect(result[0].parts).toHaveLength(1)
    expect(result[0].parts[0].type).toBe('text')
  })

  it('keeps browser tool parts while removing filesystem tool parts', () => {
    const messages: UIMessage[] = [
      makeAssistantMessage([
        {
          type: 'tool-take_snapshot',
          toolCallId: 'call-1',
          toolName: 'take_snapshot',
          state: 'result',
          input: { page: 1 },
          output: {},
        } as unknown as UIMessage['parts'][number],
        {
          type: 'tool-filesystem_read',
          toolCallId: 'call-2',
          toolName: 'filesystem_read',
          state: 'result',
          input: { path: '/tmp/test.txt' },
          output: {},
        } as unknown as UIMessage['parts'][number],
      ]),
    ]

    const result = sanitizeMessagesForToolset(messages, noFilesystemTools)
    expect(result).toHaveLength(1)
    expect(result[0].parts).toHaveLength(1)
    expect((result[0].parts[0] as { type: string }).type).toBe(
      'tool-take_snapshot',
    )
  })

  it('removes messages that become empty after stripping', () => {
    const messages: UIMessage[] = [
      makeUserMessage('Read this file'),
      makeAssistantMessage([
        {
          type: 'tool-filesystem_read',
          toolCallId: 'call-1',
          toolName: 'filesystem_read',
          state: 'result',
          input: { path: '/tmp/test.txt' },
          output: {},
        } as unknown as UIMessage['parts'][number],
      ]),
    ]

    const result = sanitizeMessagesForToolset(messages, noFilesystemTools)
    // The assistant message had only a tool part — after stripping, it's empty
    // and should be filtered out by hasMessageContent
    expect(result).toHaveLength(1)
    expect(result[0].role).toBe('user')
  })

  it('preserves non-tool part types (reasoning, step-start, file)', () => {
    const messages: UIMessage[] = [
      makeAssistantMessage([
        { type: 'text', text: 'Let me think...' },
        {
          type: 'reasoning',
          reasoning: 'Analyzing the request',
        } as unknown as UIMessage['parts'][number],
        {
          type: 'step-start',
        } as unknown as UIMessage['parts'][number],
      ]),
    ]

    const result = sanitizeMessagesForToolset(messages, noFilesystemTools)
    expect(result).toHaveLength(1)
    expect(result[0].parts).toHaveLength(3)
  })

  it('returns same message references when no filtering needed', () => {
    const messages: UIMessage[] = [
      makeUserMessage('Hello'),
      makeAssistantMessage([{ type: 'text', text: 'Hi!' }]),
    ]

    const result = sanitizeMessagesForToolset(messages, noFilesystemTools)
    // Messages that don't need filtering should be the same reference
    expect(result[0]).toBe(messages[0])
    expect(result[1]).toBe(messages[1])
  })

  it('handles empty message array', () => {
    const result = sanitizeMessagesForToolset([], noFilesystemTools)
    expect(result).toHaveLength(0)
  })

  it('handles empty toolset (all tools removed)', () => {
    const messages: UIMessage[] = [
      makeAssistantMessage([
        { type: 'text', text: 'Working...' },
        {
          type: 'tool-navigate_page',
          toolCallId: 'call-1',
          toolName: 'navigate_page',
          state: 'result',
          input: {},
          output: {},
        } as unknown as UIMessage['parts'][number],
      ]),
    ]

    const result = sanitizeMessagesForToolset(messages, new Set())
    expect(result).toHaveLength(1)
    expect(result[0].parts).toHaveLength(1)
    expect(result[0].parts[0].type).toBe('text')
  })
})

// ---------------------------------------------------------------------------
// closeDanglingToolCalls
// ---------------------------------------------------------------------------

function danglingToolPart(
  state: string,
  extra: Record<string, unknown> = {},
): UIMessage['parts'][number] {
  return {
    type: 'tool-browse',
    toolCallId: 'call-1',
    toolName: 'browse',
    state,
    input: { url: 'https://example.com' },
    ...extra,
  } as unknown as UIMessage['parts'][number]
}

const TERMINAL_TOOL_STATES = new Set([
  'approval-responded',
  'output-available',
  'output-error',
  'output-denied',
])

describe('closeDanglingToolCalls', () => {
  it.each(['input-streaming', 'input-available', 'approval-requested'])(
    'closes a %s tool part with an error result',
    (state) => {
      const [message] = closeDanglingToolCalls([
        makeAssistantMessage([danglingToolPart(state)]),
      ])

      const part = message.parts[0] as { state: string; errorText?: string }
      expect(part.state).toBe('output-error')
      expect(part.errorText).toBeTruthy()
    },
  )

  it('closes an approved call that never produced a result', () => {
    const approved = danglingToolPart('approval-responded', {
      approval: { id: 'approval-1', approved: true },
    })

    const [message] = closeDanglingToolCalls([makeAssistantMessage([approved])])

    const part = message.parts[0] as {
      state: string
      approval?: { approved: boolean }
    }
    expect(part.state).toBe('output-error')
    expect(part.approval?.approved).toBe(true)
  })

  it('leaves a denied approval untouched', () => {
    const denied = danglingToolPart('approval-responded', {
      approval: { id: 'approval-1', approved: false },
    })

    const [message] = closeDanglingToolCalls([makeAssistantMessage([denied])])

    expect(message.parts[0]).toBe(denied)
  })

  it('returns the same message references when nothing is dangling', () => {
    const messages: UIMessage[] = [
      makeUserMessage('hi'),
      makeAssistantMessage([toolPart('browse', 'c1')]),
    ]

    const result = closeDanglingToolCalls(messages)

    expect(result[0]).toBe(messages[0])
    expect(result[1]).toBe(messages[1])
  })

  // Regression: a provider stream that errors after emitting a complete tool
  // call left the part at `input-available`; the persisted history then threw
  // MissingToolResultsError inside the next turn's prompt conversion.
  it('keeps the stored history convertible when a turn dies mid tool call', async () => {
    const browse = tool({
      description: 'browse a page',
      inputSchema: z.object({ url: z.string() }),
      execute: async () => 'page contents',
    })

    const dyingModel = new MockLanguageModelV3({
      doStream: async () => ({
        stream: new ReadableStream({
          start(ctrl) {
            ctrl.enqueue({
              type: 'tool-call',
              toolCallId: 'call_1',
              toolName: 'browse',
              input: '{"url":"https://example.com"}',
              delta: '{"url":"https://example.com"}',
            })
            ctrl.enqueue({ type: 'error', error: new Error('upstream reset') })
            ctrl.close()
          },
        }),
      }),
    })

    let finished: UIMessage[] = []
    const stream1 = await createAgentUIStream({
      agent: new ToolLoopAgent({ model: dyingModel, tools: { browse } }),
      uiMessages: [makeUserMessage('go')],
      onError: () => 'err',
      onFinish: ({ messages }: { messages: UIMessage[] }) => {
        finished = messages
      },
    })
    for await (const _ of stream1) {
      // drain so onFinish runs
    }

    // The line the chat service's onFinish runs before storing history.
    const stored = filterValidMessages(closeDanglingToolCalls(finished))
    expect(
      stored.every((message) =>
        message.parts.every(
          (part) => !isToolUIPart(part) || TERMINAL_TOOL_STATES.has(part.state),
        ),
      ),
    ).toBe(true)
    stored.push(makeUserMessage('now do something else'))

    // Turn 2 must reach the model instead of throwing in prompt conversion.
    let prompted = false
    const healthyModel = new MockLanguageModelV3({
      doStream: async () => {
        prompted = true
        return {
          stream: new ReadableStream({
            start(ctrl) {
              ctrl.enqueue({ type: 'text-start', id: 't1' })
              ctrl.enqueue({ type: 'text-delta', id: 't1', delta: 'ok' })
              ctrl.enqueue({ type: 'text-end', id: 't1' })
              ctrl.enqueue({
                type: 'finish',
                finishReason: { unified: 'stop', raw: 'stop' },
                usage: {
                  inputTokens: { total: 10, noCache: 10 },
                  outputTokens: { total: 5, text: 5 },
                },
              })
              ctrl.close()
            },
          }),
        }
      },
    })

    const stream2 = await createAgentUIStream({
      agent: new ToolLoopAgent({ model: healthyModel, tools: { browse } }),
      uiMessages: stored,
      onError: () => 'err',
    })
    for await (const _ of stream2) {
      // drain
    }

    expect(prompted).toBe(true)
  })
})

// ---------------------------------------------------------------------------
// hasMessageContent (existing function, verify edge cases)
// ---------------------------------------------------------------------------

describe('hasMessageContent', () => {
  it('rejects messages with empty parts array', () => {
    const msg: UIMessage = {
      id: '1',
      role: 'assistant',
      parts: [],
    }
    expect(hasMessageContent(msg)).toBe(false)
  })

  it('rejects messages with only whitespace text', () => {
    const msg: UIMessage = {
      id: '1',
      role: 'assistant',
      parts: [{ type: 'text', text: '   \n  ' }],
    }
    expect(hasMessageContent(msg)).toBe(false)
  })

  it('accepts messages with non-text parts', () => {
    const msg: UIMessage = {
      id: '1',
      role: 'assistant',
      parts: [
        {
          type: 'tool-click',
          toolCallId: 'call-1',
          toolName: 'click',
          state: 'result',
          input: {},
          output: {},
        } as unknown as UIMessage['parts'][number],
      ],
    }
    expect(hasMessageContent(msg)).toBe(true)
  })
})

// ---------------------------------------------------------------------------
// stripReasoningParts
// ---------------------------------------------------------------------------

function reasoningPart(text: string): UIMessage['parts'][number] {
  return {
    type: 'reasoning',
    text,
    state: 'done',
  } as unknown as UIMessage['parts'][number]
}

function toolPart(name: string, callId: string): UIMessage['parts'][number] {
  return {
    type: `tool-${name}`,
    toolCallId: callId,
    state: 'output-available',
    input: {},
    output: { ok: true },
  } as unknown as UIMessage['parts'][number]
}

const stepStart = {
  type: 'step-start',
} as unknown as UIMessage['parts'][number]

// An assistant message with no text and no tool call is invalid on the
// OpenAI-compatible wire format (content: null, no tool_calls) and strict
// providers reject it with "The content field is a required field".
function invalidAssistantCount(model: ModelMessage[]): number {
  return model.filter((m) => {
    if (m.role !== 'assistant') return false
    const content = m.content
    if (typeof content === 'string') return content.trim().length === 0
    if (Array.isArray(content)) {
      const hasText = content.some(
        (p) => p.type === 'text' && p.text.trim().length > 0,
      )
      const hasToolCall = content.some((p) => p.type === 'tool-call')
      return !hasText && !hasToolCall
    }
    return true
  }).length
}

describe('stripReasoningParts', () => {
  it('removes reasoning parts while keeping text and tool parts', () => {
    const messages: UIMessage[] = [
      makeAssistantMessage([
        stepStart,
        reasoningPart('thinking'),
        toolPart('tabs', 'c1'),
        { type: 'text', text: 'done' },
      ]),
    ]

    const [result] = stripReasoningParts(messages)
    expect(result.parts.map((p) => p.type)).toEqual([
      'step-start',
      'tool-tabs',
      'text',
    ])
  })

  it('drops a message whose only content was reasoning', () => {
    const messages: UIMessage[] = [
      makeUserMessage('hi'),
      makeAssistantMessage([reasoningPart('private thought')]),
    ]

    const result = stripReasoningParts(messages)
    expect(result).toHaveLength(1)
    expect(result[0].role).toBe('user')
  })

  it('returns the same reference when there is no reasoning to strip', () => {
    const messages: UIMessage[] = [
      makeUserMessage('hi'),
      makeAssistantMessage([{ type: 'text', text: 'hello' }]),
    ]

    const result = stripReasoningParts(messages)
    expect(result[0]).toBe(messages[0])
    expect(result[1]).toBe(messages[1])
  })

  it('handles an empty array', () => {
    expect(stripReasoningParts([])).toHaveLength(0)
  })

  // Regression guard for the real bug: a persisted assistant turn with
  // reasoning replayed to a strict provider produced a reasoning-only assistant
  // model message and a 400 "content field is required".
  it('makes replayed history convert to a valid provider request', async () => {
    const history: UIMessage[] = [
      makeUserMessage('weather in singapore'),
      makeAssistantMessage([
        stepStart,
        reasoningPart('look up the weather'),
        toolPart('tabs', 'c1'),
        { type: 'text', text: 'It is sunny in Singapore.' },
      ]),
      makeUserMessage('and in malaysia?'),
      // A reasoning-only step converts to an assistant model message with no
      // text and no tool call: the exact shape the provider rejects.
      makeAssistantMessage([reasoningPart('the user asked about malaysia')]),
    ]

    // Without the fix, the reasoning-only step yields an invalid assistant
    // message ("content field is required").
    expect(
      invalidAssistantCount(await convertToModelMessages(history)),
    ).toBeGreaterThan(0)

    // With the fix, nothing invalid reaches the provider...
    const fixed = await convertToModelMessages(stripReasoningParts(history))
    expect(invalidAssistantCount(fixed)).toBe(0)
    // ...and the prior tool call and text still survive (higher fidelity than
    // the cloud lane's text-only projection).
    expect(fixed.some((m) => m.role === 'tool')).toBe(true)
    expect(
      fixed.some(
        (m) =>
          m.role === 'assistant' &&
          Array.isArray(m.content) &&
          m.content.some((p) => p.type === 'text'),
      ),
    ).toBe(true)
  })
})
