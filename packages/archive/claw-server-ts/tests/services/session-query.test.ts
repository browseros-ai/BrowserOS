import { describe, expect, it, mock } from 'bun:test'
import type { ClientIdentity } from '../../src/lib/mcp-session'
import type { TabActivityRecord } from '../../src/lib/tab-activity'
import type { SessionTabRow } from '../../src/modules/db/schema/session-tabs.sql'
import {
  type CurrentBrowserPage,
  createSessionQueryService,
  type SessionQueryDependencies,
} from '../../src/services/session-query'
import type { TaskSummary } from '../../src/services/tasks'

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
    latestScreenshotId: null,
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

function setup(overrides: Partial<SessionQueryDependencies> = {}) {
  const identities = [identity('session-a')]
  const tasks = new Map<string, TaskSummary>([['session-a', task('session-a')]])
  const ownerships = [ownership('session-a', 101)]
  const pages = [page(101)]
  const activities = [activity('session-a', 101)]
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
    listBrowserPages: async () => pages,
    snapshotTabActivity: () => activities,
    ...overrides,
  }
  return { service: createSessionQueryService(deps), deps }
}

describe('session query service', () => {
  it('includes only dispatch-backed connected identities without pagination or slug grouping', async () => {
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
      [
        'session-empty',
        task('session-empty', {
          dispatchCount: 0,
          toolSequence: [],
          cursorId: 0,
        }),
      ],
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
    expect(
      result.items.some((item) => item.sessionId === 'session-empty'),
    ).toBe(false)
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
    ).toEqual([])
    expect(
      (await service.listSessions({ status: 'live', site: 'example.com' }))
        .items[0]?.sessionId,
    ).toBe('session-a')
    expect(
      (await service.listSessions({ status: 'live', search: 'waiting' })).items,
    ).toEqual([])
    expect(
      (await service.listSessions({ status: 'live', since: 2_000 })).items,
    ).toEqual([])
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
    const listTasks = mock(() => ({
      tasks: [task('historical', { status: 'done' })],
      nextCursor: 42,
    }))
    const { service } = setup({
      listTasks,
      listOpenSessionTabs,
      listBrowserPages,
      snapshotTabActivity,
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
  })
})
