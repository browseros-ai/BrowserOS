import { describe, expect, it } from 'bun:test'
import type { AgentConversationTurn } from '@/lib/agent-conversations/types'
import {
  type AgentHistoryPageResponse,
  type BrowserOSChatHistoryItem,
  buildChatHistoryFromClawMessages,
  filterTurnsPersistedInHistory,
  flattenHistoryPages,
  mapHistoryItemToClawMessage,
} from './claw-chat-types'

function historyItem(
  overrides: Partial<BrowserOSChatHistoryItem>,
): BrowserOSChatHistoryItem {
  return {
    id: 'session-1:0',
    role: 'user',
    text: 'Hello',
    timestamp: 1000,
    messageSeq: 0,
    sessionKey: 'session-1',
    source: 'user-chat',
    ...overrides,
  }
}

function page(items: BrowserOSChatHistoryItem[]): AgentHistoryPageResponse {
  return {
    agentId: 'main',
    sessionKey: 'session-1',
    session: null,
    items,
    page: {
      hasMore: false,
      limit: 50,
    },
  }
}

describe('claw-chat-types', () => {
  it('maps backend history items into text-first ClawChat messages', () => {
    const message = mapHistoryItemToClawMessage(
      historyItem({
        id: 'session-1:1',
        role: 'assistant',
        text: 'Hi there',
        messageSeq: 1,
      }),
    )

    expect(message).toEqual({
      id: 'session-1:1',
      role: 'assistant',
      sessionKey: 'session-1',
      timestamp: 1000,
      source: 'user-chat',
      messageSeq: 1,
      status: 'historical',
      parts: [{ type: 'text', text: 'Hi there' }],
    })
  })

  it('flattens paginated history into oldest-to-newest render order', () => {
    const messages = flattenHistoryPages([
      page([
        historyItem({
          id: 'session-1:2',
          role: 'user',
          text: 'newer',
          timestamp: 3000,
          messageSeq: 2,
        }),
      ]),
      page([
        historyItem({
          id: 'session-1:0',
          role: 'user',
          text: 'older',
          timestamp: 1000,
          messageSeq: 0,
        }),
        historyItem({
          id: 'session-1:1',
          role: 'assistant',
          text: 'middle',
          timestamp: 2000,
          messageSeq: 1,
        }),
      ]),
    ])

    expect(messages.map((message) => message.id)).toEqual([
      'session-1:0',
      'session-1:1',
      'session-1:2',
    ])
  })

  it('builds OpenClaw chat history from text message parts only', () => {
    const history = buildChatHistoryFromClawMessages([
      {
        id: 'user-1',
        role: 'user',
        sessionKey: 'session-1',
        parts: [{ type: 'text', text: '  User request  ' }],
      },
      {
        id: 'assistant-1',
        role: 'assistant',
        sessionKey: 'session-1',
        parts: [
          { type: 'reasoning', text: 'private reasoning' },
          { type: 'text', text: 'Assistant answer' },
        ],
      },
    ])

    expect(history).toEqual([
      { role: 'user', content: 'User request' },
      { role: 'assistant', content: 'Assistant answer' },
    ])
  })

  it('hides completed live turns once harness history contains the same turn', () => {
    const turn: AgentConversationTurn = {
      id: 'live-turn',
      userText: 'hello',
      parts: [{ kind: 'text', text: 'hi there' }],
      done: true,
      timestamp: 1_000,
    }

    const visible = filterTurnsPersistedInHistory(
      [turn],
      [
        {
          id: 'history-user',
          role: 'user',
          sessionKey: 'main',
          timestamp: 1_050,
          status: 'historical',
          parts: [{ type: 'text', text: 'hello' }],
        },
        {
          id: 'history-assistant',
          role: 'assistant',
          sessionKey: 'main',
          timestamp: 1_100,
          status: 'historical',
          parts: [{ type: 'text', text: 'hi there' }],
        },
      ],
    )

    expect(visible).toEqual([])
  })

  it('keeps completed live turns until matching assistant history arrives', () => {
    const turn: AgentConversationTurn = {
      id: 'live-turn',
      userText: 'hello',
      parts: [{ kind: 'text', text: 'hi there' }],
      done: true,
      timestamp: 1_000,
    }

    const visible = filterTurnsPersistedInHistory(
      [turn],
      [
        {
          id: 'history-user',
          role: 'user',
          sessionKey: 'main',
          timestamp: 1_050,
          status: 'historical',
          parts: [{ type: 'text', text: 'hello' }],
        },
      ],
    )

    expect(visible).toEqual([turn])
  })
})
