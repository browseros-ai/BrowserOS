import { describe, expect, it } from 'bun:test'
import {
  type ChatSendDecisionInput,
  decideChatSend,
  drainPendingSends,
} from './chat-send-decision'

function input(
  overrides: Partial<ChatSendDecisionInput> = {},
): ChatSendDecisionInput {
  return {
    isRestoring: false,
    hasRestoreError: false,
    isSettled: true,
    hasAnyTarget: true,
    isIntegrationsSynced: true,
    hasAgentUrl: true,
    ...overrides,
  }
}

describe('decideChatSend', () => {
  it('sends when everything is ready', () => {
    expect(decideChatSend(input())).toBe('send')
  })

  it('refuses once the lists have settled on nothing connected', () => {
    expect(decideChatSend(input({ hasAnyTarget: false }))).toBe('refuse')
  })

  it('queues rather than refusing while the lists are still loading', () => {
    // The home composer hands off on mount, before either list has resolved.
    // Refusing there discarded the task: the query parameters carrying it are
    // consumed and cleared before the send is attempted, so nothing can retry.
    expect(
      decideChatSend(input({ isSettled: false, hasAnyTarget: false })),
    ).toBe('queue')
  })

  it('queues while the agent server is still starting', () => {
    expect(decideChatSend(input({ hasAgentUrl: false }))).toBe('queue')
  })

  it('queues until integrations have synced', () => {
    expect(decideChatSend(input({ isIntegrationsSynced: false }))).toBe('queue')
  })

  it('drops a send made during conversation restore', () => {
    expect(decideChatSend(input({ isRestoring: true }))).toBe('drop')
    expect(decideChatSend(input({ hasRestoreError: true }))).toBe('drop')
  })

  it('treats restore as the first question, ahead of anything else', () => {
    expect(
      decideChatSend(
        input({ isRestoring: true, isSettled: true, hasAnyTarget: false }),
      ),
    ).toBe('drop')
  })
})

describe('drainPendingSends', () => {
  it('sends every message that was held, in the order it was written', async () => {
    // The regression this guards: the queue was one slot, so a second send
    // during the same wait replaced the first and the first never went. The
    // composer had already cleared for both.
    const sent: string[] = []

    await drainPendingSends(['first', 'second', 'third'], async (text) => {
      sent.push(text)
    })

    expect(sent).toEqual(['first', 'second', 'third'])
  })

  it('waits for each turn before starting the next', async () => {
    const events: string[] = []
    const send = (text: string) =>
      new Promise<void>((resolve) => {
        events.push(`start:${text}`)
        setTimeout(() => {
          events.push(`end:${text}`)
          resolve()
        }, 0)
      })

    await drainPendingSends(['a', 'b'], send)

    expect(events).toEqual(['start:a', 'end:a', 'start:b', 'end:b'])
  })

  it('does nothing when nothing was held', async () => {
    let calls = 0

    await drainPendingSends([], async () => {
      calls += 1
    })

    expect(calls).toBe(0)
  })
})
