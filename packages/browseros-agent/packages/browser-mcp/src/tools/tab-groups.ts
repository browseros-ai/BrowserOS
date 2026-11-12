import type { BrowserSession } from '@browseros/browser-core/core/session'
import type { TabGroup } from '@browseros/browser-core/tab-groups'
import type { ClosedTabGroupInfo } from '@browseros/cdp-protocol/domains/browser'
import { z } from 'zod/v4'
import { supportsClosedTabGroups } from '../native-browseros-version'
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

function formatClosedGroup(group: ClosedTabGroupInfo): string {
  return `[saved:${group.savedGroupId}] "${group.title}" (${group.color}) [CLOSED${group.shared ? ', SHARED' : ''}] profile: ${group.browserProfileId} tabs: ${group.tabCount}`
}

export const tab_groups = defineTool({
  name: 'tab_groups',
  description:
    'Manage tab groups: list groups, group pages, update a group (title/color/collapsed), ungroup pages, or close a group (preserving its saved record). List with includeClosed=true to see closed saved groups in loaded profiles; delete removes one closed non-shared saved group using savedGroupId and browserProfileId. These closed-group operations require BrowserOS 0.52.2.0 or newer. Deletion also syncs to other devices. Page ids come from the tabs tool.',
  input: z
    .object({
      action: z
        .enum(['list', 'create', 'update', 'ungroup', 'close', 'delete'])
        .default('list'),
      includeClosed: z
        .boolean()
        .optional()
        .describe(
          'Include closed saved groups from loaded profiles when listing.',
        ),
      savedGroupId: z
        .string()
        .optional()
        .describe('Saved group UUID from closedGroups, required for delete.'),
      browserProfileId: z
        .string()
        .optional()
        .describe('Profile handle from closedGroups, required for delete.'),
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
      title: z
        .string()
        .optional()
        .describe('Group title for "create"/"update".'),
      color: z
        .enum(TAB_GROUP_COLORS)
        .optional()
        .describe('Group color for "update".'),
      collapsed: z
        .boolean()
        .optional()
        .describe('Collapse/expand the group for "update".'),
    })
    .strict(),
  annotations: {
    title: 'Manage tab groups',
    destructiveHint: true,
    openWorldHint: true,
  },
  handler: async (args, ctx) => {
    const versionError = closedTabGroupsVersionError(
      args.includeClosed,
      args.action,
      ctx.nativeBrowserosVersion,
    )
    if (versionError) return versionError

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
        if (!args.includeClosed) {
          return textResult(text, { groups: resolved, count: resolved.length })
        }
        const { groups: closedGroups } = (await ctx.session.cdp(
          'Browser.getClosedTabGroups',
        )) as {
          groups: ClosedTabGroupInfo[]
        }
        const closedText = closedGroups.map(formatClosedGroup)
        return textResult([text, ...closedText].join('\n'), {
          groups: resolved,
          count: resolved.length,
          closedGroups,
        })
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

      case 'delete':
        return deleteClosedGroup(args, ctx.session)

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

async function deleteClosedGroup(
  args: { groupId?: string; savedGroupId?: string; browserProfileId?: string },
  session: BrowserSession,
) {
  if (args.groupId !== undefined) {
    return errorResult(
      'tab_groups delete: use savedGroupId and browserProfileId from list with includeClosed=true, not groupId.',
    )
  }
  if (!args.savedGroupId || !args.browserProfileId) {
    return errorResult(
      'tab_groups delete: savedGroupId and browserProfileId are required.',
    )
  }
  await session.cdp('Browser.deleteClosedTabGroup', {
    savedGroupId: args.savedGroupId,
    browserProfileId: args.browserProfileId,
  })
  return textResult(`deleted closed saved tab group ${args.savedGroupId}`, {
    savedGroupId: args.savedGroupId,
    browserProfileId: args.browserProfileId,
  })
}

function closedTabGroupsVersionError(
  includeClosed: boolean | undefined,
  action: string,
  nativeBrowserosVersion: string | undefined,
) {
  if (!includeClosed && action !== 'delete') return undefined
  if (supportsClosedTabGroups(nativeBrowserosVersion)) return undefined

  const reportedVersion = nativeBrowserosVersion?.trim() || 'unknown'
  return errorResult(
    `tab_groups: closed saved groups require BrowserOS 0.52.2.0 or newer; connected browser version is ${reportedVersion}.`,
  )
}
