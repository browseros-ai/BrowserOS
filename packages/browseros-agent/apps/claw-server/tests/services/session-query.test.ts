import { describe, expect, it, mock } from 'bun:test'
import type { ClientIdentity } from '../../src/lib/mcp-session'
import type { TabActivityRecord } from '../../src/lib/tab-activity'
import type { SessionTabRow } from '../../src/modules/db/schema/session-tabs.sql'
import type { ScreencastFrame } from '../../src/services/screencast-cache'
import {
  type CurrentBrowserPage,
  createSessionQueryService,
  type SessionQueryDependencies,
} from '../../src/services/session-query'
import type { TaskSummary } from '../../src/services/tasks'

const NOW = 10_000

function identity(
  sessionId: string,
  overrides: Partial<ClientIdentity> = {},
): ClientIdentity {
  return {
    sessionId,
    clientName: 'Codex',
    clientVersion: '1.0.0',
    clientTitle: 'Codex CLI',
    slug: 'codex',
    key: `codex-${sessionId}` as ClientIdentity['key'],
    generatedLabel: 'Quiet Falcon',
    label: 'Current session label',
    renameNudgesLeft: 5,
    firstSeenAt: 1_000,
    ...overrides,
  }
}

function task(
  sessionId: string,
  overrides: Partial<TaskSummary> = {},
): TaskSummary {
  return {
    sessionId,
    agentId: `agent-${sessionId}`,
    slug: 'codex',
    agentLabel: 'Codex',
    title: 'Browsed example.com',
    site: 'example.com',
    startedAt: 1_100,
    endedAt: null,
    durationMs: 100,
    dispatchCount: 1,
    toolSequence: ['snapshot'],
    status: 'done',
    errorCount: 0,
    lastScreenshotDispatchId: null,
    cursorId: 1,
    ...overrides,
  }
}

function ownership(
  sessionId: string,
  tabId: number,
  overrides: Partial<SessionTabRow> = {},
): SessionTabRow {
  return {
    id: tabId,
    sessionId,
    agentId: `agent-${sessionId}`,
    tabId,
    openedTargetId: `target-${tabId}`,
    claimedAt: 1_000,
    releasedAt: null,
    ...overrides,
  }
}

function page(
  tabId: number,
  overrides: Partial<CurrentBrowserPage> = {},
): CurrentBrowserPage {
  return {
    tabId,
    pageId: tabId + 1_000,
    targetId: `target-${tabId}`,
    url: `https://example.com/${tabId.toString()}`,
    title: `Tab ${tabId.toString()}`,
    ...overrides,
  }
}

function activity(
  sessionId: string,
  tabId: number,
  overrides: Partial<TabActivityRecord> = {},
): TabActivityRecord {
  return {
    sessionId,
    tabId,
    pageId: tabId + 1_000,
    targetId: `target-${tabId}`,
    url: `https://stale.example/${tabId.toString()}`,
    title: 'Stale activity title',
    agentId: `agent-${sessionId}`,
    slug: 'codex',
    firstToolAt: 1_500,
    lastToolAt: 2_000,
    lastToolName: 'snapshot',
    toolCount: 2,
    recentTools: [{ name: 'snapshot', at: 2_000 }],
    status: 'active',
    ...overrides,
  }
}

function frame(
  pageId: number,
  targetId: string,
  jpegBase64 = '/9g=',
  sessionId = 'session-a',
): ScreencastFrame {
  return {
    sessionId,
    targetId,
    jpegBase64,
    capturedAt: pageId + 5_000,
    byteLength: jpegBase64.length,
  }
}

function setup(overrides: Partial<SessionQueryDependencies> = {}) {
  const identities = [identity('session-a')]
  const tasks = new Map<string, TaskSummary>([['session-a', task('session-a')]])
  const ownerships = [ownership('session-a', 101)]
  const pages = [page(101)]
  const activities = [activity('session-a', 101)]
  const frames = [frame(1_101, 'target-101')]
  const deps: SessionQueryDependencies = {
    listConnectedIdentities: () => identities,
    getConnectedIdentity: (sessionId) =>
      identities.find((record) => record.sessionId === sessionId) ?? null,
    listTasks: () => ({ tasks: Array.from(tasks.values()), nextCursor: null }),
    getTaskSummaries: (sessionIds) =>
      new Map(
        sessionIds.flatMap((sessionId) => {
          const summary = tasks.get(sessionId)
          return summary ? [[sessionId, summary] as const] : []
        }),
      ),
    listOpenSessionTabs: () => ownerships,
    getOpenSessionTab: (sessionId, tabId) =>
      ownerships.find(
        (row) => row.sessionId === sessionId && row.tabId === tabId,
      ) ?? null,
    listBrowserPages: async () => pages,
    snapshotTabActivity: () => activities,
    getScreencastFrame: (sessionId, pageId, targetId) =>
      frames.find(
        (candidate) =>
          candidate.sessionId === sessionId &&
          candidate.capturedAt === pageId + 5_000 &&
          candidate.targetId === targetId,
      ) ?? null,
    now: () => NOW,
    ...overrides,
  }
  return { service: createSessionQueryService(deps), deps }
}

describe('session query service', () => {
  it('includes every connected identity without pagination or slug grouping', async () => {
    const identities = [
      identity('session-a'),
      identity('session-b', { label: 'Another run' }),
      identity('session-empty', {
        clientName: 'Claude Code',
        clientTitle: null,
        slug: 'claude-code',
        label: 'Waiting for first tool',
        firstSeenAt: 2_500,
      }),
    ]
    const tasks = new Map<string, TaskSummary>([
      ['session-a', task('session-a')],
      ['session-b', task('session-b')],
    ])
    const { service } = setup({
      listConnectedIdentities: () => identities,
      getConnectedIdentity: (sessionId) =>
        identities.find((record) => record.sessionId === sessionId) ?? null,
      getTaskSummaries: (sessionIds) =>
        new Map(
          sessionIds.flatMap((sessionId) => {
            const summary = tasks.get(sessionId)
            return summary ? [[sessionId, summary] as const] : []
          }),
        ),
      listOpenSessionTabs: () => [],
      listBrowserPages: async () => [],
      snapshotTabActivity: () => [],
    })

    const result = await service.listSessions({
      status: 'live',
      cursor: 999,
      limit: 1,
    })

    expect(result.nextCursor).toBeUndefined()
    expect(result.items.map((item) => item.sessionId)).toEqual([
      'session-a',
      'session-b',
      'session-empty',
    ])
    expect(result.items.filter((item) => item.slug === 'codex')).toHaveLength(2)
    expect(result.items[0]).toMatchObject({
      slug: 'codex',
      label: 'Codex',
      name: 'Current session label',
      status: 'live',
      harness: 'Codex',
      color: '#7A5AF8',
      live: { state: 'idle', browserTabs: [] },
    })
    expect(result.items[0]?.color).toBe(result.items[1]?.color)
    expect(result.items[2]).toEqual({
      sessionId: 'session-empty',
      harness: 'Claude Code',
      color: expect.any(String),
      slug: 'claude-code',
      label: 'Claude Code',
      name: 'Waiting for first tool',
      startedAt: 2_500,
      durationMs: NOW - 2_500,
      dispatchCount: 0,
      toolSequence: [],
      status: 'live',
      errorCount: 0,
      live: { state: 'idle', browserTabs: [] },
    })
    expect(result.items[2]).not.toHaveProperty('profileId')
  })

  it('applies non-pagination filters to the connected snapshot', async () => {
    const identities = [
      identity('session-a'),
      identity('session-empty', {
        clientName: 'Claude Code',
        clientTitle: null,
        slug: 'claude-code',
        label: 'Waiting for first tool',
        firstSeenAt: 2_500,
      }),
    ]
    const { service } = setup({
      listConnectedIdentities: () => identities,
      getTaskSummaries: (sessionIds) =>
        new Map(
          sessionIds.includes('session-a')
            ? [['session-a', task('session-a')]]
            : [],
        ),
      listOpenSessionTabs: () => [],
      listBrowserPages: async () => [],
      snapshotTabActivity: () => [],
    })

    expect(
      (await service.listSessions({ status: 'live', slug: 'claude-code' }))
        .items,
    ).toHaveLength(1)
    expect(
      (await service.listSessions({ status: 'live', site: 'example.com' }))
        .items[0]?.sessionId,
    ).toBe('session-a')
    expect(
      (await service.listSessions({ status: 'live', search: 'waiting' }))
        .items[0]?.sessionId,
    ).toBe('session-empty')
    expect(
      (await service.listSessions({ status: 'live', since: 2_000 })).items[0]
        ?.sessionId,
    ).toBe('session-empty')
    expect(
      (await service.listSessions({ status: 'live', profileId: 'unknown' }))
        .items,
    ).toEqual([])
  })

  it('projects open ownership against one current browser reconciliation', async () => {
    const listBrowserPages = mock(async () => [
      page(101),
      page(102),
      page(104, { targetId: 'target-current' }),
      page(201),
    ])
    const getScreencastFrame = mock(
      (
        sessionId: string,
        pageId: number,
        targetId: string,
      ): ScreencastFrame | null =>
        sessionId === 'session-a' &&
        pageId === 1_101 &&
        targetId === 'target-101'
          ? frame(pageId, targetId)
          : null,
    )
    const identities = [identity('session-a'), identity('session-b')]
    const { service } = setup({
      listConnectedIdentities: () => identities,
      getConnectedIdentity: (sessionId) =>
        identities.find((record) => record.sessionId === sessionId) ?? null,
      getTaskSummaries: (sessionIds) =>
        new Map(sessionIds.map((sessionId) => [sessionId, task(sessionId)])),
      listOpenSessionTabs: () => [
        ownership('session-a', 101),
        ownership('session-a', 102),
        ownership('session-a', 103),
        ownership('session-a', 104),
        ownership('session-b', 201),
      ],
      listBrowserPages,
      snapshotTabActivity: () => [
        activity('session-a', 101, { lastToolAt: 3_000 }),
        activity('session-b', 102, { lastToolAt: 4_000 }),
        activity('session-a', 104, { targetId: 'target-old' }),
        activity('session-b', 201, { status: 'idle', lastToolAt: 2_500 }),
      ],
      getScreencastFrame,
    })

    const result = await service.listSessions({ status: 'live' })
    const first = result.items.find((item) => item.sessionId === 'session-a')
    const second = result.items.find((item) => item.sessionId === 'session-b')

    expect(listBrowserPages).toHaveBeenCalledTimes(1)
    expect(first?.live).toEqual({
      state: 'active',
      browserTabs: [
        {
          browserTabId: 101,
          url: 'https://example.com/101',
          title: 'Tab 101',
          firstActivityAt: 1_500,
          lastActivityAt: 3_000,
          lastToolName: 'snapshot',
          toolCount: 2,
          recentTools: [{ name: 'snapshot', at: 2_000 }],
          previewCapturedAt: 6_101,
        },
        {
          browserTabId: 102,
          url: 'https://example.com/102',
          title: 'Tab 102',
          toolCount: 0,
          recentTools: [],
        },
        {
          browserTabId: 104,
          url: 'https://example.com/104',
          title: 'Tab 104',
          toolCount: 0,
          recentTools: [],
        },
      ],
    })
    expect(second?.live?.browserTabs.map((tab) => tab.browserTabId)).toEqual([
      201,
    ])
    expect(JSON.stringify(first?.live?.browserTabs)).not.toMatch(
      /pageId|targetId|sessionId|profileId|slug|label|harness|color/,
    )
    expect(getScreencastFrame).toHaveBeenCalledWith(
      'session-a',
      1_101,
      'target-101',
    )
    expect(getScreencastFrame).toHaveBeenCalledWith(
      'session-a',
      1_102,
      'target-102',
    )
    expect(getScreencastFrame).toHaveBeenCalledWith(
      'session-a',
      1_104,
      'target-current',
    )
  })

  it('derives active and idle state from current exact activity', async () => {
    let status: 'active' | 'idle' = 'active'
    const { service } = setup({
      snapshotTabActivity: () => [activity('session-a', 101, { status })],
    })

    expect(
      (await service.listSessions({ status: 'live' })).items[0]?.live?.state,
    ).toBe('active')
    status = 'idle'
    expect(
      (await service.listSessions({ status: 'live' })).items[0]?.live?.state,
    ).toBe('idle')
  })

  it('uses one summary-only read bounded to the connected session ids', async () => {
    const identities = [identity('session-a'), identity('session-b')]
    const getTaskSummaries = mock(
      (sessionIds: readonly string[]) =>
        new Map(
          sessionIds.map((sessionId) => [sessionId, task(sessionId)] as const),
        ),
    )
    const getTask = mock(() => {
      throw new Error('detail reader must not be used by the live query')
    })
    const overrides = {
      listConnectedIdentities: () => identities,
      getTaskSummaries,
      listOpenSessionTabs: () => [],
      listBrowserPages: async () => [],
      snapshotTabActivity: () => [],
      getTask,
    }
    const { service } = setup(overrides)

    const result = await service.listSessions({ status: 'live' })

    expect(result.items.map((item) => item.sessionId)).toEqual([
      'session-a',
      'session-b',
    ])
    expect(getTaskSummaries).toHaveBeenCalledTimes(1)
    expect(getTaskSummaries).toHaveBeenCalledWith(['session-a', 'session-b'])
    expect(getTask).not.toHaveBeenCalled()
  })

  it('preserves activity when page reconciliation is unavailable and restores it on recovery', async () => {
    let attempts = 0
    const snapshotTabActivity = mock(() => [activity('session-a', 101)])
    const { service } = setup({
      listBrowserPages: async () => {
        attempts += 1
        return attempts === 1 ? null : [page(101)]
      },
      snapshotTabActivity,
    })

    const unavailable = await service.listSessions({ status: 'live' })
    expect(unavailable.items).toHaveLength(1)
    expect(unavailable.items[0]?.live).toEqual({
      state: 'idle',
      browserTabs: [],
    })
    expect(snapshotTabActivity).not.toHaveBeenCalled()

    const recovered = await service.listSessions({ status: 'live' })
    expect(recovered.items).toHaveLength(1)
    expect(recovered.items[0]?.live).toMatchObject({
      state: 'active',
      browserTabs: [
        {
          browserTabId: 101,
          toolCount: 2,
          recentTools: [{ name: 'snapshot', at: 2_000 }],
        },
      ],
    })
    expect(snapshotTabActivity).toHaveBeenCalledTimes(1)
  })

  it('reads final liveness and ownership after page reconciliation', async () => {
    let identities = [identity('session-a'), identity('session-b')]
    let ownerships = [ownership('session-a', 101)]
    const reconciliationStarted = Promise.withResolvers<void>()
    const reconciliation = Promise.withResolvers<CurrentBrowserPage[]>()
    const { service } = setup({
      listConnectedIdentities: () => identities,
      getConnectedIdentity: (sessionId) =>
        identities.find((record) => record.sessionId === sessionId) ?? null,
      getTaskSummaries: (sessionIds) =>
        new Map(sessionIds.map((sessionId) => [sessionId, task(sessionId)])),
      listOpenSessionTabs: () => ownerships,
      listBrowserPages: () => {
        reconciliationStarted.resolve()
        return reconciliation.promise
      },
      snapshotTabActivity: () => [activity('session-b', 101)],
    })

    const pending = service.listSessions({ status: 'live' })
    await reconciliationStarted.promise
    identities = [identity('session-b')]
    ownerships = [ownership('session-b', 101)]
    reconciliation.resolve([page(101)])
    const result = await pending

    expect(result.items.map((item) => item.sessionId)).toEqual(['session-b'])
    expect(result.items[0]?.live?.browserTabs).toEqual([
      expect.objectContaining({ browserTabId: 101, toolCount: 2 }),
    ])
  })

  it('does not transfer an unchanged target frame between session owners', async () => {
    const identities = [identity('session-a'), identity('session-b')]
    let ownerships = [ownership('session-a', 101)]
    let cached = frame(1_101, 'target-101', '/9g=', 'session-a')
    const { service } = setup({
      listConnectedIdentities: () => identities,
      getConnectedIdentity: (sessionId) =>
        identities.find((record) => record.sessionId === sessionId) ?? null,
      getTaskSummaries: (sessionIds) =>
        new Map(sessionIds.map((sessionId) => [sessionId, task(sessionId)])),
      listOpenSessionTabs: () => ownerships,
      getOpenSessionTab: (sessionId, tabId) =>
        ownerships.find(
          (row) => row.sessionId === sessionId && row.tabId === tabId,
        ) ?? null,
      getScreencastFrame: (sessionId, pageId, targetId) =>
        cached.sessionId === sessionId &&
        cached.capturedAt === pageId + 5_000 &&
        cached.targetId === targetId
          ? cached
          : null,
    })

    expect(await service.getSessionBrowserTabPreview('session-a', 101)).toEqual(
      cached,
    )

    ownerships = [ownership('session-b', 101)]
    expect(
      await service.getSessionBrowserTabPreview('session-a', 101),
    ).toBeNull()
    expect(
      await service.getSessionBrowserTabPreview('session-b', 101),
    ).toBeNull()
    expect(
      (await service.listSessions({ status: 'live' })).items.find(
        (item) => item.sessionId === 'session-b',
      )?.live?.browserTabs[0],
    ).not.toHaveProperty('previewCapturedAt')

    cached = frame(1_101, 'target-101', '/9g=', 'session-b')
    expect(await service.getSessionBrowserTabPreview('session-b', 101)).toEqual(
      cached,
    )
    expect(
      (await service.listSessions({ status: 'live' })).items.find(
        (item) => item.sessionId === 'session-b',
      )?.live?.browserTabs[0],
    ).toHaveProperty('previewCapturedAt', 6_101)
  })

  it('keeps unfiltered and historical status queries audit-only', async () => {
    const listOpenSessionTabs = mock(() => {
      throw new Error('ownership must not be read')
    })
    const listBrowserPages = mock(async () => {
      throw new Error('browser must not be read')
    })
    const snapshotTabActivity = mock(() => {
      throw new Error('activity must not be read')
    })
    const getScreencastFrame = mock(() => {
      throw new Error('cache must not be read')
    })
    const listTasks = mock(() => ({
      tasks: [task('historical', { status: 'done' })],
      nextCursor: 42,
    }))
    const { service } = setup({
      listTasks,
      listOpenSessionTabs,
      listBrowserPages,
      snapshotTabActivity,
      getScreencastFrame,
    })

    for (const query of [
      {},
      { status: 'done' as const },
      { status: 'failed' as const },
    ]) {
      const result = await service.listSessions(query)
      expect(result).toMatchObject({ nextCursor: 42 })
      expect(result.items[0]).not.toHaveProperty('live')
    }
    expect(listTasks).toHaveBeenCalledTimes(3)
    expect(listOpenSessionTabs).not.toHaveBeenCalled()
    expect(listBrowserPages).not.toHaveBeenCalled()
    expect(snapshotTabActivity).not.toHaveBeenCalled()
    expect(getScreencastFrame).not.toHaveBeenCalled()
  })

  it('serves only a non-empty frame for exact connected ownership and incarnation', async () => {
    let identities = [identity('session-a')]
    let ownerships = [ownership('session-a', 101)]
    let pages = [page(101)]
    let cached: ScreencastFrame | null = frame(1_101, 'target-101')
    let reassignDuringBrowserRead = false
    const calls: string[] = []
    const { service } = setup({
      getConnectedIdentity: (sessionId) => {
        calls.push('identity')
        return (
          identities.find((record) => record.sessionId === sessionId) ?? null
        )
      },
      getOpenSessionTab: (sessionId, tabId) => {
        calls.push('ownership')
        return (
          ownerships.find(
            (row) => row.sessionId === sessionId && row.tabId === tabId,
          ) ?? null
        )
      },
      listBrowserPages: async () => {
        calls.push('browser')
        if (reassignDuringBrowserRead) {
          ownerships = [ownership('session-b', 101)]
        }
        return pages
      },
      getScreencastFrame: (sessionId, pageId, targetId) => {
        calls.push('cache')
        return cached?.sessionId === sessionId &&
          cached.targetId === targetId &&
          pageId === 1_101
          ? cached
          : null
      },
    })

    expect(await service.getSessionBrowserTabPreview('session-a', 101)).toEqual(
      cached,
    )
    expect(calls).toEqual([
      'ownership',
      'identity',
      'browser',
      'ownership',
      'identity',
      'cache',
    ])

    for (const missing of [
      { sessionId: 'missing', tabId: 101, owner: ownerships, current: pages },
      { sessionId: 'session-a', tabId: 999, owner: ownerships, current: pages },
      { sessionId: 'session-b', tabId: 101, owner: ownerships, current: pages },
      { sessionId: 'session-a', tabId: 101, owner: [], current: pages },
      { sessionId: 'session-a', tabId: 101, owner: ownerships, current: [] },
    ]) {
      ownerships = missing.owner
      pages = missing.current
      expect(
        await service.getSessionBrowserTabPreview(
          missing.sessionId,
          missing.tabId,
        ),
      ).toBeNull()
    }

    ownerships = [ownership('session-a', 101)]
    identities = [identity('session-a')]
    reassignDuringBrowserRead = true
    expect(
      await service.getSessionBrowserTabPreview('session-a', 101),
    ).toBeNull()
    reassignDuringBrowserRead = false

    ownerships = [ownership('session-a', 101)]
    pages = [page(101, { targetId: 'target-new' })]
    expect(
      await service.getSessionBrowserTabPreview('session-a', 101),
    ).toBeNull()

    pages = [page(101)]
    cached = null
    expect(
      await service.getSessionBrowserTabPreview('session-a', 101),
    ).toBeNull()
    cached = frame(1_101, 'target-101', '')
    expect(
      await service.getSessionBrowserTabPreview('session-a', 101),
    ).toBeNull()
  })

  it('does not read the cache when the session disconnects during browser reconciliation', async () => {
    let connectedIdentity: ClientIdentity | null = identity('session-a')
    const openOwnership = ownership('session-a', 101)
    const currentPage = page(101)
    const getScreencastFrame = mock(() =>
      frame(currentPage.pageId, currentPage.targetId),
    )
    const { service } = setup({
      getConnectedIdentity: () => connectedIdentity,
      getOpenSessionTab: () => openOwnership,
      listBrowserPages: async () => {
        connectedIdentity = null
        return [currentPage]
      },
      getScreencastFrame,
    })

    expect(
      await service.getSessionBrowserTabPreview('session-a', 101),
    ).toBeNull()
    expect(openOwnership.releasedAt).toBeNull()
    expect(currentPage.targetId).toBe('target-101')
    expect(getScreencastFrame).not.toHaveBeenCalled()
  })
})
