import { describe, expect, it } from 'bun:test'
import type { BrowserSession } from '../../../src/browser/core/session'
import type { TabGroup } from '../../../src/browser/tab-groups'
import { executeTool } from '../../../src/tools/browser/framework'
import { tab_groups } from '../../../src/tools/browser/tab-groups'

interface FakeOpts {
  // page id -> tab id (drives getInfo/resolveTabIds in both directions)
  pageTabs?: Record<number, number>
  groups?: TabGroup[]
}

interface CdpCall {
  method: string
  params?: Record<string, unknown>
}

function createSession(opts: FakeOpts = {}) {
  const pageTabs = opts.pageTabs ?? {}
  const tabToPage = new Map<number, number>()
  for (const [pageId, tabId] of Object.entries(pageTabs)) {
    tabToPage.set(tabId, Number(pageId))
  }

  const calls: CdpCall[] = []
  const session = {
    pages: {
      list: async () => [],
      getInfo: (pageId: number) =>
        pageId in pageTabs ? { tabId: pageTabs[pageId] } : undefined,
      resolveTabIds: async (tabIds: number[]) => {
        const result = new Map<number, number>()
        for (const tabId of tabIds) {
          const pageId = tabToPage.get(tabId)
          if (pageId !== undefined) result.set(tabId, pageId)
        }
        return result
      },
    },
    cdp: async (method: string, params?: Record<string, unknown>) => {
      calls.push({ method, params })
      switch (method) {
        case 'Browser.getTabGroups':
          return { groups: opts.groups ?? [] }
        case 'Browser.createTabGroup':
        case 'Browser.addTabsToGroup':
        case 'Browser.updateTabGroup':
          return { group: opts.groups?.[0] }
        default:
          return {}
      }
    },
  } as unknown as BrowserSession

  return { session, calls }
}

function textOf(result: { content: Array<{ type: string; text?: string }> }) {
  return result.content
    .filter((c) => c.type === 'text')
    .map((c) => c.text)
    .join('\n')
}

const GROUP: TabGroup = {
  groupId: 'g1',
  windowId: 1,
  title: 'Work',
  color: 'blue',
  collapsed: false,
  tabIds: [11, 22],
}

describe('tab_groups tool', () => {
  it('lists an empty set of groups', async () => {
    const { session, calls } = createSession({ groups: [] })
    const result = await executeTool(
      tab_groups,
      { action: 'list' },
      { session },
    )

    expect(result.isError).toBeFalsy()
    expect(textOf(result)).toBe('(no tab groups)')
    expect(result.structuredContent).toEqual({ groups: [], count: 0 })
    expect(calls).toEqual([
      { method: 'Browser.getTabGroups', params: undefined },
    ])
  })

  it('defaults to list when no action is given', async () => {
    const { calls, session } = createSession({ groups: [] })
    const result = await executeTool(tab_groups, {}, { session })

    expect(result.isError).toBeFalsy()
    expect(calls[0]?.method).toBe('Browser.getTabGroups')
  })

  it('lists populated groups with tab ids mapped back to page ids', async () => {
    const { session } = createSession({
      pageTabs: { 1: 11, 2: 22 },
      groups: [GROUP],
    })
    const result = await executeTool(
      tab_groups,
      { action: 'list' },
      { session },
    )

    expect(result.isError).toBeFalsy()
    expect(result.structuredContent).toEqual({
      groups: [
        {
          groupId: 'g1',
          windowId: 1,
          title: 'Work',
          color: 'blue',
          collapsed: false,
          pageIds: [1, 2],
        },
      ],
      count: 1,
    })
    expect(textOf(result)).toContain('[g1] "Work" (blue) pages: 1, 2')
  })

  it('creates a new group from page ids', async () => {
    const { session, calls } = createSession({
      pageTabs: { 1: 11, 2: 22 },
      groups: [GROUP],
    })
    const result = await executeTool(
      tab_groups,
      { action: 'create', pages: [1, 2], title: 'Work' },
      { session },
    )

    expect(result.isError).toBeFalsy()
    expect(calls[0]).toEqual({
      method: 'Browser.createTabGroup',
      params: { tabIds: [11, 22], title: 'Work' },
    })
    expect(result.structuredContent).toMatchObject({
      group: { groupId: 'g1', pageIds: [1, 2] },
    })
  })

  it('adds pages to an existing group when groupId is provided on create', async () => {
    const { session, calls } = createSession({
      pageTabs: { 1: 11, 2: 22 },
      groups: [GROUP],
    })
    const result = await executeTool(
      tab_groups,
      { action: 'create', pages: [1], groupId: 'g1' },
      { session },
    )

    expect(result.isError).toBeFalsy()
    expect(calls[0]).toEqual({
      method: 'Browser.addTabsToGroup',
      params: { groupId: 'g1', tabIds: [11] },
    })
  })

  it('errors when create combines an existing groupId with a title', async () => {
    const { session, calls } = createSession({ pageTabs: { 1: 11 } })
    const result = await executeTool(
      tab_groups,
      { action: 'create', pages: [1], groupId: 'g1', title: 'Renamed' },
      { session },
    )

    expect(result.isError).toBe(true)
    expect(textOf(result)).toContain('use action="update" to rename')
    expect(calls).toEqual([])
  })

  it('updates a group title and color', async () => {
    const { session, calls } = createSession({
      pageTabs: { 1: 11, 2: 22 },
      groups: [{ ...GROUP, title: 'Renamed', color: 'red' }],
    })
    const result = await executeTool(
      tab_groups,
      { action: 'update', groupId: 'g1', title: 'Renamed', color: 'red' },
      { session },
    )

    expect(result.isError).toBeFalsy()
    expect(calls[0]).toEqual({
      method: 'Browser.updateTabGroup',
      params: { groupId: 'g1', title: 'Renamed', color: 'red' },
    })
    expect(result.structuredContent).toMatchObject({
      group: { title: 'Renamed', color: 'red' },
    })
  })

  it('ungroups pages', async () => {
    const { session, calls } = createSession({ pageTabs: { 1: 11, 2: 22 } })
    const result = await executeTool(
      tab_groups,
      { action: 'ungroup', pages: [1, 2] },
      { session },
    )

    expect(result.isError).toBeFalsy()
    expect(calls[0]).toEqual({
      method: 'Browser.removeTabsFromGroup',
      params: { tabIds: [11, 22] },
    })
    expect(result.structuredContent).toEqual({ pageIds: [1, 2], count: 2 })
  })

  it('closes a group', async () => {
    const { session, calls } = createSession()
    const result = await executeTool(
      tab_groups,
      { action: 'close', groupId: 'g1' },
      { session },
    )

    expect(result.isError).toBeFalsy()
    expect(calls[0]).toEqual({
      method: 'Browser.closeTabGroup',
      params: { groupId: 'g1' },
    })
    expect(result.structuredContent).toEqual({ groupId: 'g1' })
  })

  it('errors when create is missing pages', async () => {
    const { session, calls } = createSession()
    const result = await executeTool(
      tab_groups,
      { action: 'create' },
      { session },
    )

    expect(result.isError).toBe(true)
    expect(textOf(result)).toContain('pages is required')
    expect(calls).toEqual([])
  })

  it('errors when ungroup is missing pages', async () => {
    const { session } = createSession()
    const result = await executeTool(
      tab_groups,
      { action: 'ungroup', pages: [] },
      { session },
    )

    expect(result.isError).toBe(true)
    expect(textOf(result)).toContain('pages is required')
  })

  it('errors when update is missing groupId', async () => {
    const { session } = createSession()
    const result = await executeTool(
      tab_groups,
      { action: 'update', title: 'x' },
      { session },
    )

    expect(result.isError).toBe(true)
    expect(textOf(result)).toContain('groupId is required')
  })

  it('errors when update has no fields to change', async () => {
    const { session } = createSession()
    const result = await executeTool(
      tab_groups,
      { action: 'update', groupId: 'g1' },
      { session },
    )

    expect(result.isError).toBe(true)
    expect(textOf(result)).toContain(
      'at least one of title, color, or collapsed',
    )
  })

  it('errors when close is missing groupId', async () => {
    const { session } = createSession()
    const result = await executeTool(
      tab_groups,
      { action: 'close' },
      { session },
    )

    expect(result.isError).toBe(true)
    expect(textOf(result)).toContain('groupId is required')
  })

  it('errors on an unknown page id when creating', async () => {
    const { session } = createSession({ pageTabs: { 1: 11 } })
    const result = await executeTool(
      tab_groups,
      { action: 'create', pages: [99] },
      { session },
    )

    expect(result.isError).toBe(true)
    expect(textOf(result)).toContain('Unknown page 99')
  })
})
