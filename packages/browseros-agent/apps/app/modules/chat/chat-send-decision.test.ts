import { describe, expect, it } from 'bun:test'
import {
  type ChatSendDecisionInput,
  decideChatSend,
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
