import { z } from 'zod'
import type { TabGroup } from '../../browser/tab-groups'
import { defineTool, errorResult, textResult } from './framework'

const TAB_GROUP_COLORS = [
  'grey',
  'blue',
  'red',
  'yellow',
  'green',
  'pink',
  'purple',
  'cyan',
  'orange',
] as const

interface TabGroupWithPages extends Omit<TabGroup, 'tabIds'> {
  pageIds: number[]
}

function formatGroup(group: TabGroupWithPages): string {
  const collapsed = group.collapsed ? ' [COLLAPSED]' : ''
  const pages = group.pageIds.length ? group.pageIds.join(', ') : '(none)'
  return `[${group.groupId}] "${group.title || '(unnamed)'}" (${group.color})${collapsed} pages: ${pages}`
}

export const tab_groups = defineTool({
  name: 'tab_groups',
  description:
    'Manage tab groups: list groups, group pages, update a group (title/color/collapsed), ungroup pages, or close a group. Page ids come from the tabs tool.',
  input: z.object({
    action: z
      .enum(['list', 'create', 'update', 'ungroup', 'close'])
      .default('list'),
    pages: z
      .array(z.number().int())
      .optional()
      .describe('Page ids for action="create" or "ungroup".'),
    groupId: z
      .string()
      .optional()
      .describe(
        'Group id. Required for "update"/"close". Optional on "create" to add pages to an existing group.',
      ),
    title: z.string().optional().describe('Group title for "create"/"update".'),
    color: z
      .enum(TAB_GROUP_COLORS)
      .optional()
      .describe('Group color for "update".'),
    collapsed: z
      .boolean()
      .optional()
      .describe('Collapse/expand the group for "update".'),
  }),
  annotations: { openWorldHint: true },
  handler: async (args, ctx) => {
    const { pages } = ctx.session

    // The tools speak page ids; the CDP tab-group API speaks tab ids. Convert in both directions.
    // Reconcile the registry first so freshly opened pages resolve, matching the legacy facade.
    const toTabIds = async (pageIds: number[]): Promise<number[]> => {
      await pages.list()
      return pageIds.map((pageId) => {
        const info = pages.getInfo(pageId)
        if (!info) {
          throw new Error(
            `Unknown page ${pageId}. Use the tabs tool to list pages.`,
          )
        }
        return info.tabId
      })
    }

    const toPageIds = async (tabIds: number[]): Promise<number[]> => {
      const tabToPage = await pages.resolveTabIds(tabIds)
      return tabIds
        .map((tabId) => tabToPage.get(tabId))
        .filter((id): id is number => id !== undefined)
    }

    const withPages = async (group: TabGroup): Promise<TabGroupWithPages> => {
      const { tabIds, ...rest } = group
      return { ...rest, pageIds: await toPageIds(tabIds) }
    }

    switch (args.action) {
      case 'list': {
        const { groups } = (await ctx.session.cdp('Browser.getTabGroups')) as {
          groups: TabGroup[]
        }
        const resolved = await Promise.all(groups.map(withPages))
        const text = resolved.length
          ? resolved.map(formatGroup).join('\n')
          : '(no tab groups)'
        return textResult(text, { groups: resolved, count: resolved.length })
      }

      case 'create': {
        if (!args.pages?.length) {
          return errorResult('tab_groups create: pages is required.')
        }
        // addTabsToGroup only accepts groupId + tabIds, so title would be silently dropped here.
        if (args.groupId && args.title !== undefined) {
          return errorResult(
            'tab_groups create: title cannot be set when adding pages to an existing groupId; use action="update" to rename.',
          )
        }
        const tabIds = await toTabIds(args.pages)
        const params = args.groupId
          ? { groupId: args.groupId, tabIds }
          : {
              tabIds,
              ...(args.title !== undefined && { title: args.title }),
            }
        const method = args.groupId
          ? 'Browser.addTabsToGroup'
          : 'Browser.createTabGroup'
        const { group } = (await ctx.session.cdp(method, params)) as {
          group: TabGroup
        }
        const resolved = await withPages(group)
        return textResult(`grouped into ${formatGroup(resolved)}`, {
          group: resolved,
        })
      }

      case 'update': {
        if (!args.groupId) {
          return errorResult('tab_groups update: groupId is required.')
        }
        if (
          args.title === undefined &&
          args.color === undefined &&
          args.collapsed === undefined
        ) {
          return errorResult(
            'tab_groups update: provide at least one of title, color, or collapsed.',
          )
        }
        const { group } = (await ctx.session.cdp('Browser.updateTabGroup', {
          groupId: args.groupId,
          ...(args.title !== undefined && { title: args.title }),
          ...(args.color !== undefined && { color: args.color }),
          ...(args.collapsed !== undefined && { collapsed: args.collapsed }),
        })) as { group: TabGroup }
        const resolved = await withPages(group)
        return textResult(`updated ${formatGroup(resolved)}`, {
          group: resolved,
        })
      }

      case 'ungroup': {
        if (!args.pages?.length) {
          return errorResult('tab_groups ungroup: pages is required.')
        }
        const tabIds = await toTabIds(args.pages)
        await ctx.session.cdp('Browser.removeTabsFromGroup', { tabIds })
        return textResult(`ungrouped ${args.pages.length} page(s)`, {
          pageIds: args.pages,
          count: args.pages.length,
        })
      }

      case 'close': {
        if (!args.groupId) {
          return errorResult('tab_groups close: groupId is required.')
        }
        await ctx.session.cdp('Browser.closeTabGroup', {
          groupId: args.groupId,
        })
        return textResult(`closed tab group ${args.groupId} and all its tabs`, {
          groupId: args.groupId,
        })
      }
    }
  },
})
