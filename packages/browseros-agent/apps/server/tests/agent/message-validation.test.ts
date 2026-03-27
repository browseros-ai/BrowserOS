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
import type { UIMessage } from 'ai'
import {
  hasMessageContent,
  sanitizeMessagesForToolset,
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
    'memory_search',
  ])

  const noFilesystemTools = new Set([
    'navigate_page',
    'click',
    'take_snapshot',
    'memory_search',
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
