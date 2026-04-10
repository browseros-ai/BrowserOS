import { z } from 'zod'
import { defineToolWithCategory } from './framework'

const defineManagementTool = defineToolWithCategory('data-modification')
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

const tabGroupWithPageIdsSchema = z.object({
  groupId: z.string(),
  windowId: z.number(),
  title: z.string(),
  color: z.string(),
  collapsed: z.boolean(),
  pageIds: z.array(z.number()),
})

export const list_tab_groups = defineManagementTool({
  name: 'list_tab_groups',
  description: 'List all tab groups in the browser',
  input: z.object({}),
  output: z.object({
    groups: z.array(tabGroupWithPageIdsSchema),
    count: z.number(),
  }),
  handler: async (_args, ctx, response) => {
    const groups = await ctx.browser.listTabGroups()

    if (groups.length === 0) {
      response.text('No tab groups found.')
      response.data({ groups: [], count: 0 })
      return
    }

    const lines: string[] = [`Found ${groups.length} tab groups:`, '']

    for (const group of groups) {
      const collapsedMarker = group.collapsed ? ' [COLLAPSED]' : ''
      lines.push(
        `[${group.groupId}] "${group.title || '(unnamed)'}" (${group.color})${collapsedMarker}`,
      )
      lines.push(`    Pages: ${group.pageIds.join(', ')}`)
      lines.push(`    Window: ${group.windowId}`)
    }

    response.text(lines.join('\n'))
    response.data({ groups, count: groups.length })
  },
})

export const group_tabs = defineManagementTool({
  name: 'group_tabs',
  description:
    'Group pages together with an optional title. Color is auto-assigned; use update_tab_group to change it.',
  input: z.object({
    pageIds: z
      .array(z.number())
      .describe('Array of page IDs to group together'),
    title: z.string().optional().describe('Title for the group'),
    groupId: z.string().optional().describe('Existing group ID to add tabs to'),
  }),
  output: z.object({
    group: tabGroupWithPageIdsSchema,
    groupedCount: z.number(),
  }),
  handler: async (args, ctx, response) => {
    const group = await ctx.browser.groupTabs(args.pageIds, {
      title: args.title,
      groupId: args.groupId,
    })
    response.text(
      `Grouped ${args.pageIds.length} pages into "${group.title || '(unnamed)'}" (${group.color})\nGroup ID: ${group.groupId}`,
    )
    response.data({
      group,
      groupedCount: args.pageIds.length,
    })
  },
})

export const update_tab_group = defineManagementTool({
  name: 'update_tab_group',
  description: "Update a tab group's title, color, or collapsed state",
  input: z.object({
    groupId: z.string().describe('ID of the group to update'),
    title: z.string().optional().describe('New title for the group'),
    color: z
      .enum(TAB_GROUP_COLORS)
      .optional()
      .describe('New color for the group'),
    collapsed: z
      .boolean()
      .optional()
      .describe('Whether to collapse (hide) the group tabs'),
  }),
  output: z.object({
    group: tabGroupWithPageIdsSchema,
  }),
  handler: async (args, ctx, response) => {
    const group = await ctx.browser.updateTabGroup(args.groupId, {
      title: args.title,
      color: args.color,
      collapsed: args.collapsed,
    })
    const tabToPage = await ctx.browser.resolveTabIds(group.tabIds)
    const pageIds = group.tabIds
      .map((tabId) => tabToPage.get(tabId))
      .filter((id): id is number => id !== undefined)
    const groupWithPageIds = {
      groupId: group.groupId,
      windowId: group.windowId,
      title: group.title,
      color: group.color,
      collapsed: group.collapsed,
      pageIds,
    }
    response.text(
      `Updated group ${group.groupId}: "${group.title || '(unnamed)'}" (${group.color})${group.collapsed ? ' [COLLAPSED]' : ''}`,
    )
    response.data({
      group: groupWithPageIds,
    })
  },
})

export const ungroup_tabs = defineManagementTool({
  name: 'ungroup_tabs',
  description: 'Remove pages from their groups',
  input: z.object({
    pageIds: z
      .array(z.number())
      .describe('Array of page IDs to remove from their groups'),
  }),
  output: z.object({
    action: z.literal('ungroup_tabs'),
    pageIds: z.array(z.number()),
    count: z.number(),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.ungroupTabs(args.pageIds)
    response.text(`Ungrouped ${args.pageIds.length} pages`)
    response.data({
      action: 'ungroup_tabs',
      pageIds: args.pageIds,
      count: args.pageIds.length,
    })
  },
})

export const close_tab_group = defineManagementTool({
  name: 'close_tab_group',
  description: 'Close a tab group and all its tabs',
  input: z.object({
    groupId: z
      .string()
      .describe('ID of the group to close (closes all tabs in group)'),
  }),
  output: z.object({
    action: z.literal('close_tab_group'),
    groupId: z.string(),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.closeTabGroup(args.groupId)
    response.text(`Closed tab group ${args.groupId} and all its tabs`)
    response.data({ action: 'close_tab_group', groupId: args.groupId })
    response.includePages()
  },
})
