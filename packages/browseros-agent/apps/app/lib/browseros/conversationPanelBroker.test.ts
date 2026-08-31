import { describe, expect, it, mock } from 'bun:test'
import type { ConversationPresenceEvent } from '@browseros/shared/schemas/conversation-presence'
import {
  ConversationPanelBroker,
  type ConversationPanelBrokerDeps,
} from './conversationPanelBroker'

describe('ConversationPanelBroker', () => {
  it('opens every touched tab and retains its conversation mapping', async () => {
    const fixture = createFixture()

    await fixture.broker.accept(runStarted('conversation-1', 'run-1', [10, 11]))

    expect(fixture.opened).toEqual([
      { tabId: 10, windowId: 1 },
      { tabId: 11, windowId: 1 },
    ])
    expect(fixture.views['10']).toMatchObject({
      conversationId: 'conversation-1',
      runId: 'run-1',
      status: 'running',
    })
    expect(fixture.glow).toEqual([
      { tabId: 10, isActive: true, conversationId: 'conversation-1' },
      { tabId: 11, isActive: true, conversationId: 'conversation-1' },
    ])
  })

  it('does not let an older run overwrite a tab claimed by a newer run', async () => {
    const fixture = createFixture()
    await fixture.broker.accept(runStarted('older', 'old-run', [20]))
    await fixture.broker.accept(runStarted('newer', 'new-run', [20]))

    await fixture.broker.accept(runFinished('older', 'old-run', [20]))

    expect(fixture.views['20']).toMatchObject({
      conversationId: 'newer',
      runId: 'new-run',
      status: 'running',
    })
    expect(fixture.glow.at(-1)).toMatchObject({
      conversationId: 'newer',
      isActive: true,
    })
  })

  it('treats a reconnect snapshot as authoritative and reopens running tabs', async () => {
    const fixture = createFixture({
      '99': tab(99, 'stale', 'stale-run', 'running'),
    })
    const event: ConversationPresenceEvent = {
      type: 'snapshot',
      runs: [],
      tabs: [
        tab(30, 'active', 'active-run', 'running'),
        tab(31, 'done', 'done-run', 'completed'),
      ],
    }

    await fixture.broker.accept(event)

    expect(fixture.views['99']).toBeUndefined()
    expect(fixture.opened).toEqual([{ tabId: 30, windowId: 1 }])
    expect(fixture.views['31']?.conversationId).toBe('done')
    expect(fixture.glow[0]).toEqual({
      tabId: 99,
      conversationId: 'stale',
      isActive: false,
    })
  })

  it('deactivates a finished run and shows first-run confetti once', async () => {
    const fixture = createFixture()
    await fixture.broker.accept(runStarted('conversation-4', 'run-4', [40, 41]))

    await fixture.broker.accept(
      runFinished('conversation-4', 'run-4', [40, 41]),
    )

    expect(fixture.glow.slice(-2)).toEqual([
      {
        tabId: 40,
        conversationId: 'conversation-4',
        isActive: false,
        showConfetti: true,
      },
      {
        tabId: 41,
        conversationId: 'conversation-4',
        isActive: false,
        showConfetti: false,
      },
    ])
    expect(fixture.markConfettiShown).toHaveBeenCalledTimes(1)
  })
})

function createFixture(
  initialViews: Record<string, ReturnType<typeof tab>> = {},
) {
  const views = { ...initialViews }
  const opened: Array<{ tabId: number; windowId: number }> = []
  const glow: Array<{
    tabId: number
    conversationId: string
    isActive: boolean
    showConfetti?: boolean
  }> = []
  const markConfettiShown = mock(async () => {})
  const deps: ConversationPanelBrokerDeps = {
    resolveServerUrl: async () => 'http://127.0.0.1:9000',
    fetch: mock(async () => new Response()),
    getTab: async (tabId) => ({ id: tabId, windowId: 1 }),
    openPanel: async (target) => {
      opened.push(target)
    },
    readViews: async () => ({ ...views }),
    writeViews: async (next) => {
      for (const key of Object.keys(views)) delete views[key]
      Object.assign(views, next)
    },
    sendGlow: (tabId, message) => {
      glow.push({ tabId, ...message })
    },
    hasShownConfetti: async () => false,
    markConfettiShown,
  }
  return {
    broker: new ConversationPanelBroker(deps),
    glow,
    markConfettiShown,
    opened,
    views,
  }
}

function tab(
  tabId: number,
  conversationId: string,
  runId: string,
  status: 'running' | 'completed',
) {
  return { tabId, conversationId, runId, status, updatedAt: 1 }
}

function runStarted(
  conversationId: string,
  runId: string,
  tabIds: number[],
): ConversationPresenceEvent {
  return {
    type: 'run-started',
    run: { conversationId, runId, status: 'running', tabIds, updatedAt: 1 },
  }
}

function runFinished(
  conversationId: string,
  runId: string,
  tabIds: number[],
): ConversationPresenceEvent {
  return {
    type: 'run-finished',
    run: {
      conversationId,
      runId,
      status: 'completed',
      tabIds,
      updatedAt: 2,
    },
  }
}
