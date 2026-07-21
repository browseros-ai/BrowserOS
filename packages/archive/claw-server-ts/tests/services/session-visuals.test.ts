import { describe, expect, it, mock } from 'bun:test'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import type { TabActivityRecord } from '../../src/lib/tab-activity'
import type { SessionTabRow } from '../../src/modules/db/schema/session-tabs.sql'
import {
  createSessionVisualService,
  type SessionVisualDependencies,
} from '../../src/services/session-visuals'

function ownership(sessionId: string, tabId: number): SessionTabRow {
  return {
    id: tabId,
    sessionId,
    agentId: `agent-${sessionId}`,
    tabId,
    openedTargetId: `target-${tabId.toString()}`,
    claimedAt: 100,
    releasedAt: null,
  }
}

function activity(
  sessionId: string,
  tabId: number,
  lastToolAt: number,
): TabActivityRecord {
  return {
    sessionId,
    agentId: `agent-${sessionId}`,
    slug: 'codex',
    tabId,
    pageId: tabId + 1_000,
    targetId: `target-${tabId.toString()}`,
    url: 'https://example.com',
    title: 'Example',
    firstToolAt: lastToolAt,
    lastToolAt,
    lastToolName: 'snapshot',
    toolCount: 1,
    recentTools: [],
    status: 'active',
  }
}

function setup(overrides: Partial<SessionVisualDependencies> = {}): {
  service: ReturnType<typeof createSessionVisualService>
  capture: ReturnType<typeof mock>
  list: ReturnType<typeof mock>
} {
  const list = mock(async () => [
    {
      tabId: 11,
      pageId: 1_011,
      targetId: 'target-11',
      url: 'https://one.example',
      title: 'One',
    },
    {
      tabId: 22,
      pageId: 1_022,
      targetId: 'target-22',
      url: 'https://two.example',
      title: 'Two',
    },
  ])
  const capture = mock(async () => ({
    data: '/9g=',
    mimeType: 'image/jpeg',
    annotations: [],
  }))
  const browser = {
    pages: { list },
    screenshotForTarget: capture,
  } as unknown as BrowserSession
  const ownerships = [ownership('session-a', 11), ownership('session-a', 22)]
  const deps: SessionVisualDependencies = {
    isSessionLive: (sessionId) => sessionId === 'session-a',
    getBrowserSession: () => browser,
    listOpenSessionTabs: () => ownerships,
    getOpenSessionTab: (sessionId, tabId) =>
      ownerships.find(
        (row) => row.sessionId === sessionId && row.tabId === tabId,
      ) ?? null,
    snapshotTabActivity: () => [
      activity('session-a', 11, 200),
      activity('session-a', 22, 300),
    ],
    ...overrides,
  }
  return { service: createSessionVisualService(deps), capture, list }
}

describe('session visual service', () => {
  it('captures the most recently active owned tab with canonical options', async () => {
    const { service, capture } = setup()

    expect(await service.capture('session-a')).toEqual(
      new Uint8Array([0xff, 0xd8]),
    )
    expect(capture).toHaveBeenCalledWith(1_022, 'target-22', {
      format: 'jpeg',
      quality: 50,
      fullPage: false,
      annotate: false,
    })
  })

  it('uses tab id as a deterministic activity tie-breaker', async () => {
    const { service, capture } = setup({
      snapshotTabActivity: () => [
        activity('session-a', 22, 300),
        activity('session-a', 11, 300),
      ],
    })

    await service.capture('session-a')
    expect(capture).toHaveBeenCalledWith(1_011, 'target-11', expect.anything())
  })

  it('rejects unknown, ended, unowned, and stale-target sessions', async () => {
    const unknown = setup()
    expect(await unknown.service.capture('session-b')).toBeNull()
    expect(unknown.list).not.toHaveBeenCalled()

    const unowned = setup({ listOpenSessionTabs: () => [] })
    expect(await unowned.service.capture('session-a')).toBeNull()
    expect(unowned.capture).not.toHaveBeenCalled()

    const stale = setup({
      getOpenSessionTab: () => null,
    })
    expect(await stale.service.capture('session-a')).toBeNull()
    expect(stale.capture).toHaveBeenCalledTimes(1)
  })

  it('performs one fresh browser capture on every call', async () => {
    const { service, capture } = setup()

    await service.capture('session-a')
    await service.capture('session-a')
    expect(capture).toHaveBeenCalledTimes(2)
  })

  it('does not queue another capture while timed-out CDP work is unresolved', async () => {
    let releaseCapture: (value: {
      data: string
      mimeType: string
      annotations: never[]
    }) => void = () => undefined
    const pendingCapture = new Promise<{
      data: string
      mimeType: string
      annotations: never[]
    }>((resolve) => {
      releaseCapture = resolve
    })
    const { service, capture } = setup({ captureTimeoutMs: 5 })
    capture.mockImplementation(() => pendingCapture)

    await expect(service.capture('session-a')).rejects.toThrow(
      'session preview capture timed out',
    )
    expect(await service.capture('session-a')).toBeNull()
    expect(capture).toHaveBeenCalledTimes(1)

    releaseCapture({ data: '/9g=', mimeType: 'image/jpeg', annotations: [] })
    await pendingCapture
    expect(await service.capture('session-a')).toEqual(
      new Uint8Array([0xff, 0xd8]),
    )
    expect(capture).toHaveBeenCalledTimes(2)
  })
})
