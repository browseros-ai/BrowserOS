import { describe, expect, it, mock } from 'bun:test'
import type { ConversationPanelAssignments } from '@browseros/shared/schemas/conversation-panels'
import {
  ConversationPanelBroker,
  type ConversationPanelBrokerDeps,
  type ConversationPanelBrokerErrorContext,
} from './conversationPanelBroker'
import type { ConversationPanelViews } from './conversationPanelStorage'

describe('ConversationPanelBroker', () => {
  it('opens every touched tab and retains its conversation mapping', async () => {
    const fixture = createFixture()

    await fixture.broker.reconcile(
      assignments([
        assignment(10, 'conversation-1', 'run-1', 'running'),
        assignment(11, 'conversation-1', 'run-1', 'running'),
      ]),
    )

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
    await fixture.broker.reconcile(
      assignments([assignment(20, 'older', 'old-run', 'running')]),
    )
    const newer = assignments([assignment(20, 'newer', 'new-run', 'running')])
    await fixture.broker.reconcile(newer)

    // A stale run finishing cannot appear in the server's canonical mapping.
    await fixture.broker.reconcile(newer)

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

  it('treats reconnect assignments as authoritative', async () => {
    const fixture = createFixture()
    await fixture.broker.reconcile(
      assignments([assignment(99, 'stale', 'stale-run', 'running')]),
    )
    fixture.opened.length = 0
    fixture.glow.length = 0

    await fixture.broker.reconcile(
      assignments([
        assignment(30, 'active', 'active-run', 'running'),
        assignment(31, 'done', 'done-run', 'completed'),
      ]),
    )

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
    await fixture.broker.reconcile(
      assignments([
        assignment(40, 'conversation-4', 'run-4', 'running'),
        assignment(41, 'conversation-4', 'run-4', 'running'),
      ]),
    )

    await fixture.broker.reconcile(
      assignments([
        assignment(40, 'conversation-4', 'run-4', 'completed'),
        assignment(41, 'conversation-4', 'run-4', 'completed'),
      ]),
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

  it('retries the complete handoff after a transient storage failure', async () => {
    const fixture = createFixture({
      writeViews: async (_views, call) => {
        if (call === 1) throw new Error('session storage not ready')
      },
    })
    const current = assignments([
      assignment(50, 'conversation-5', 'run-5', 'running'),
    ])

    await expect(fixture.broker.reconcile(current)).rejects.toThrow(
      'session storage not ready',
    )
    await fixture.broker.reconcile(current)

    expect(fixture.opened).toEqual([{ tabId: 50, windowId: 1 }])
    expect(fixture.views['50']).toMatchObject({ runId: 'run-5' })
  })

  it('retries initialization after a transient storage-read failure', async () => {
    const fixture = createFixture({
      readViews: async (call) => {
        if (call === 1) throw new Error('session storage not ready')
        return {}
      },
    })
    const current = assignments([
      assignment(55, 'conversation-5', 'run-5', 'running'),
    ])

    await expect(fixture.broker.reconcile(current)).rejects.toThrow(
      'session storage not ready',
    )
    await fixture.broker.reconcile(current)

    expect(fixture.opened).toEqual([{ tabId: 55, windowId: 1 }])
  })

  it('reasserts open panels on a heartbeat without restarting their glow', async () => {
    const fixture = createFixture()
    const current = assignments([
      assignment(60, 'conversation-6', 'run-6', 'running'),
    ])

    await fixture.broker.reconcile(current)
    await fixture.broker.reconcile(current)

    expect(fixture.opened).toEqual([
      { tabId: 60, windowId: 1 },
      { tabId: 60, windowId: 1 },
    ])
    expect(fixture.glow).toEqual([
      { tabId: 60, isActive: true, conversationId: 'conversation-6' },
    ])
  })

  it('heals a transient panel-open failure on the next heartbeat', async () => {
    const fixture = createFixture({
      openPanel: async (_target, call) => {
        if (call === 1) throw new Error('side panel temporarily unavailable')
      },
    })
    const current = assignments([
      assignment(70, 'conversation-7', 'run-7', 'running'),
    ])

    await fixture.broker.reconcile(current)
    await fixture.broker.reconcile(current)

    expect(fixture.opened).toEqual([{ tabId: 70, windowId: 1 }])
    expect(fixture.errors).toEqual([
      expect.objectContaining({
        context: { phase: 'open-panel', tabId: 70 },
      }),
    ])
  })
})

interface FixtureOptions {
  readViews?(call: number): Promise<ConversationPanelViews>
  writeViews?(views: ConversationPanelViews, call: number): Promise<void>
  openPanel?(
    target: { tabId: number; windowId: number },
    call: number,
  ): Promise<void>
}

function createFixture(options: FixtureOptions = {}) {
  const views: ConversationPanelViews = {}
  const opened: Array<{ tabId: number; windowId: number }> = []
  const glow: Array<{
    tabId: number
    conversationId: string
    isActive: boolean
    showConfetti?: boolean
  }> = []
  const errors: Array<{
    error: unknown
    context: ConversationPanelBrokerErrorContext
  }> = []
  const markConfettiShown = mock(async () => {})
  let readViewsCalls = 0
  let writeViewsCalls = 0
  let openPanelCalls = 0
  const deps: ConversationPanelBrokerDeps = {
    resolveServerUrl: async () => 'http://127.0.0.1:9000',
    fetch: mock(async () => new Response()),
    getTab: async (tabId) => ({ id: tabId, windowId: 1 }),
    openPanel: async (target) => {
      openPanelCalls += 1
      await options.openPanel?.(target, openPanelCalls)
      opened.push(target)
    },
    readViews: async () => {
      readViewsCalls += 1
      return (await options.readViews?.(readViewsCalls)) ?? { ...views }
    },
    writeViews: async (next) => {
      writeViewsCalls += 1
      await options.writeViews?.(next, writeViewsCalls)
      for (const key of Object.keys(views)) delete views[key]
      Object.assign(views, next)
    },
    sendGlow: (tabId, message) => {
      glow.push({ tabId, ...message })
    },
    hasShownConfetti: async () => false,
    markConfettiShown,
    reportError: (error, context) => errors.push({ error, context }),
  }
  return {
    broker: new ConversationPanelBroker(deps),
    errors,
    glow,
    markConfettiShown,
    opened,
    views,
  }
}

function assignment(
  tabId: number,
  conversationId: string,
  runId: string,
  status: 'running' | 'completed',
) {
  return { tabId, conversationId, runId, status }
}

function assignments(
  values: ConversationPanelAssignments['assignments'],
): ConversationPanelAssignments {
  return { assignments: values }
}
