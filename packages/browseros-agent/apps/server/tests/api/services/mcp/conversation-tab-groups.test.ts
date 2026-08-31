import { describe, expect, it } from 'bun:test'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import type { ActiveConversationRun } from '../../../../src/api/services/conversation-runs'
import { ConversationTabGroups } from '../../../../src/api/services/mcp/conversation-tab-groups'

describe('ConversationTabGroups', () => {
  it('creates a group for the first created tab and serially joins later tabs', async () => {
    const fixture = browserFixture()
    const groups = new ConversationTabGroups(fixture.session)
    const run = activeRun('conversation-1')

    groups.addCreatedPages(run, [2])
    groups.addCreatedPages(run, [3])
    await eventually(() =>
      expect(fixture.groups[0]?.tabIds).toEqual([102, 103]),
    )

    expect(fixture.calls.filter((call) => call.method === 'create')).toEqual([
      expect.objectContaining({ tabIds: [102], title: 'browseros/research' }),
    ])
    expect(fixture.calls.filter((call) => call.method === 'add')).toEqual([
      expect.objectContaining({ tabIds: [103], groupId: 'group-1' }),
    ])
  })

  it('repairs a retained group after the user deletes it', async () => {
    const fixture = browserFixture()
    const groups = new ConversationTabGroups(fixture.session)
    const run = activeRun('conversation-2')

    groups.addCreatedPages(run, [2])
    await eventually(() => expect(fixture.groups).toHaveLength(1))
    fixture.groups.length = 0

    groups.addCreatedPages(run, [3])
    await eventually(() => expect(fixture.groups[0]?.tabIds).toEqual([103]))

    expect(
      fixture.calls.filter((call) => call.method === 'create'),
    ).toHaveLength(2)
    expect(fixture.groups[0]?.groupId).toBe('group-2')
  })
})

function activeRun(conversationId: string): ActiveConversationRun {
  return {
    conversationId,
    runId: 'run-1',
    panelsVisible: true,
    tabGroup: { title: 'browseros/research', colorKey: 'browseros' },
    signal: new AbortController().signal,
    associateTabs: () => true,
  }
}

function browserFixture() {
  const pages = [page(1, 101), page(2, 102), page(3, 103)]
  const groups: Array<{
    groupId: string
    tabIds: number[]
    title: string
    color: string
    collapsed: boolean
    windowId: number
  }> = []
  const calls: Array<Record<string, unknown> & { method: string }> = []
  let nextGroup = 1
  const session = {
    pages: {
      list: async () => pages,
      getInfo: (pageId: number) =>
        pages.find((candidate) => candidate.pageId === pageId),
      resolveTabIds: async (tabIds: number[]) =>
        new Map(
          tabIds.flatMap((tabId) => {
            const match = pages.find((candidate) => candidate.tabId === tabId)
            return match ? [[tabId, match.pageId] as const] : []
          }),
        ),
    },
    cdp: async (method: string, params: Record<string, unknown> = {}) => {
      switch (method) {
        case 'Browser.createTabGroup': {
          calls.push({ method: 'create', ...params })
          const group = {
            groupId: `group-${nextGroup++}`,
            tabIds: [...(params.tabIds as number[])],
            title: String(params.title ?? ''),
            color: 'grey',
            collapsed: false,
            windowId: 7,
          }
          groups.push(group)
          return { group }
        }
        case 'Browser.addTabsToGroup': {
          calls.push({ method: 'add', ...params })
          const group = groups.find(
            (candidate) => candidate.groupId === params.groupId,
          )
          if (!group) throw new Error('No such group')
          group.tabIds.push(...(params.tabIds as number[]))
          return { group }
        }
        case 'Browser.updateTabGroup': {
          calls.push({ method: 'update', ...params })
          const group = groups.find(
            (candidate) => candidate.groupId === params.groupId,
          )
          if (!group) throw new Error('No such group')
          if (typeof params.color === 'string') group.color = params.color
          return { group }
        }
        case 'Browser.getTabGroups':
          calls.push({ method: 'list' })
          return { groups }
        default:
          throw new Error(`Unexpected CDP method: ${method}`)
      }
    },
  } as unknown as BrowserSession
  return { calls, groups, session }
}

function page(pageId: number, tabId: number) {
  return {
    pageId,
    tabId,
    targetId: `target-${pageId}`,
    url: 'about:blank',
    title: '',
    isActive: false,
    isLoading: false,
    loadProgress: 1,
    isPinned: false,
    windowId: 7,
  }
}

async function eventually(assertion: () => void): Promise<void> {
  for (let attempt = 0; attempt < 50; attempt += 1) {
    try {
      assertion()
      return
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 0))
    }
  }
  assertion()
}
