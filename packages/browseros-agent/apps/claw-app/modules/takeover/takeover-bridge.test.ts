import { beforeEach, describe, expect, it, mock } from 'bun:test'
import {
  type HelpRequest,
  SessionStatus,
  type SessionSummary,
} from '@browseros/claw-api'

let sessions: SessionSummary[] = []
let listCalls = 0
const resolveCalls: Array<{ sessionId: string; note?: string }> = []

mock.module('@browseros/claw-api-client', () => ({
  ClawApiClient: class {
    async listSessions() {
      listCalls += 1
      return { items: sessions }
    }
    async resolveHelp(request: { sessionId: string; note?: string }) {
      resolveCalls.push(request)
      return { resolved: true }
    }
  },
}))

const { createTakeoverBridge } = await import('./takeover-bridge')

function session(
  tabId: number,
  over: Partial<HelpRequest> = {},
): SessionSummary {
  const help: HelpRequest = {
    requestId: 'help-1',
    reason: 'Enter the login',
    browserTabId: tabId,
    requestedAt: 5_000,
    ...over,
  }
  return {
    sessionId: 'session-1',
    slug: 'claude-code',
    label: 'Claude-code',
    name: 'Outreach',
    durationMs: 0,
    dispatchCount: 3,
    errorCount: 0,
    toolSequence: [],
    status: SessionStatus.Live,
    startedAt: 1_000,
    live: { state: 'idle', browserTabs: [], helpRequest: help },
  }
}

beforeEach(() => {
  sessions = []
  listCalls = 0
  resolveCalls.length = 0
})

describe('takeover bridge', () => {
  it('reports the blocked tab as active and others as inactive', async () => {
    sessions = [session(77)]
    const bridge = createTakeoverBridge({
      resolveServerBaseUrl: async () => 'http://127.0.0.1:9000',
    })

    expect(await bridge.pollForTab(77)).toEqual({
      active: true,
      context: {
        sessionId: 'session-1',
        agentLabel: 'Claude-code',
        reason: 'Enter the login',
        resumeHint: undefined,
        site: undefined,
        requestedAt: 5_000,
      },
    })
    expect(await bridge.pollForTab(42)).toEqual({ active: false })
  })

  it('coalesces concurrent polls within the cache window into one read', async () => {
    sessions = [session(77)]
    let clock = 10_000
    const bridge = createTakeoverBridge({
      resolveServerBaseUrl: async () => 'http://127.0.0.1:9000',
      now: () => clock,
    })

    await Promise.all([bridge.pollForTab(77), bridge.pollForTab(42)])
    clock += 500 // still inside the TTL
    await bridge.pollForTab(77)
    expect(listCalls).toBe(1)

    clock += 2_000 // past the TTL
    await bridge.pollForTab(77)
    expect(listCalls).toBe(2)
  })

  it('resolves through the client and clears the cache', async () => {
    sessions = [session(77)]
    const bridge = createTakeoverBridge({
      resolveServerBaseUrl: async () => 'http://127.0.0.1:9000',
    })

    await bridge.pollForTab(77)
    await bridge.resolve('session-1', 'signed in')
    expect(resolveCalls).toEqual([
      { sessionId: 'session-1', note: 'signed in' },
    ])

    await bridge.pollForTab(77)
    // One read before resolve, one after (the resolve dropped the cache).
    expect(listCalls).toBe(2)
  })
})
