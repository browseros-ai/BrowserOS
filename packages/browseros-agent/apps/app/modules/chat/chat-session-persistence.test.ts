import { describe, expect, it } from 'bun:test'
import type { ChatStatus, UIMessage } from 'ai'
import {
  didStreamingTurnFinish,
  getPersistableMessages,
} from './chat-session-persistence'

describe('chat session persistence transitions', () => {
  it('saves exactly once when streaming ends in an error with partial content', () => {
    const messages = [
      userMessage('Fix the active tab'),
      assistantMessage('Partial response'),
    ]
    const saves = collectSaves(
      ['ready', 'streaming', 'error', 'ready'],
      messages,
    )

    expect(saves).toHaveLength(1)
    expect(saves[0]).toEqual(messages)
  })

  it('keeps the existing streaming to ready save behavior', () => {
    const messages = [userMessage('Hello'), assistantMessage('Done')]
    const saves = collectSaves(
      ['ready', 'submitted', 'streaming', 'ready'],
      messages,
    )

    expect(saves).toHaveLength(1)
    expect(saves[0]).toEqual(messages)
  })

  it('does not save when the previous status was not streaming', () => {
    const messages = [userMessage('Hello')]
    const saves = collectSaves(['ready', 'error', 'ready'], messages)

    expect(saves).toHaveLength(0)
  })

  it('filters empty assistant messages but keeps partial assistant content', () => {
    const user = userMessage('Hello')
    const partialAssistant = assistantMessage('Partial response')
    const emptyAssistant: UIMessage = {
      id: 'assistant-empty',
      role: 'assistant',
      parts: [],
    }

    expect(
      getPersistableMessages([user, emptyAssistant, partialAssistant]),
    ).toEqual([user, partialAssistant])
  })
})

function collectSaves(statuses: ChatStatus[], messages: UIMessage[]) {
  const saves: UIMessage[][] = []
  let previousStatus = statuses[0]

  for (const status of statuses.slice(1)) {
    if (didStreamingTurnFinish(previousStatus, status)) {
      saves.push(getPersistableMessages(messages))
    }
    previousStatus = status
  }

  return saves
}

function userMessage(text: string): UIMessage {
  return {
    id: `user-${text}`,
    role: 'user',
    parts: [{ type: 'text', text }],
  }
}

function assistantMessage(text: string): UIMessage {
  return {
    id: `assistant-${text}`,
    role: 'assistant',
    parts: [{ type: 'text', text }],
  }
}
