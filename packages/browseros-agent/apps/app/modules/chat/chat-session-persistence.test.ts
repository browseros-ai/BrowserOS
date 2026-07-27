import { describe, expect, it } from 'bun:test'
import type { ChatStatus, UIMessage } from 'ai'
import {
  didStreamingTurnFinish,
  getPersistableMessages,
  pickRicherMessages,
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

describe('pickRicherMessages (resume reconcile)', () => {
  const local = [userMessage('a'), assistantMessage('b')]
  const remoteAhead = [
    userMessage('a'),
    assistantMessage('b'),
    userMessage('c'),
  ]

  it('prefers the cloud copy when it has more messages', () => {
    expect(pickRicherMessages(local, remoteAhead)).toBe(remoteAhead)
  })

  it('prefers the cloud copy when both are equal length (cloud is authoritative)', () => {
    const remoteSame = [userMessage('a'), assistantMessage('b')]
    expect(pickRicherMessages(local, remoteSame)).toBe(remoteSame)
  })

  it('keeps the local copy when it has more messages (never regress)', () => {
    const remoteBehind = [userMessage('a')]
    expect(pickRicherMessages(local, remoteBehind)).toBe(local)
  })

  it('falls back to local when the cloud has no copy', () => {
    expect(pickRicherMessages(local, undefined)).toBe(local)
  })

  it('uses the cloud copy when there is no local copy', () => {
    expect(pickRicherMessages(undefined, remoteAhead)).toBe(remoteAhead)
  })

  it('returns undefined when neither source has the conversation', () => {
    expect(pickRicherMessages(undefined, undefined)).toBeUndefined()
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
