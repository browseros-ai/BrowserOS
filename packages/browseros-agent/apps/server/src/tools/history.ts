import { z } from 'zod'
import { defineToolWithCategory } from './framework'

const defineManagementTool = defineToolWithCategory('data-modification')
const historyItemSchema = z.object({
  id: z.string(),
  url: z.string(),
  title: z.string(),
  lastVisitTime: z.number(),
  visitCount: z.number().optional(),
  typedCount: z.number().optional(),
})

export const search_history = defineManagementTool({
  name: 'search_history',
  description: 'Search browser history by text query',
  input: z.object({
    query: z.string().describe('Search query'),
    maxResults: z
      .number()
      .optional()
      .describe('Maximum number of results to return (default: 100)'),
  }),
  output: z.object({
    query: z.string(),
    items: z.array(historyItemSchema),
    count: z.number(),
  }),
  handler: async (args, ctx, response) => {
    const items = await ctx.browser.searchHistory(args.query, args.maxResults)

    if (items.length === 0) {
      response.text(`No history items found matching "${args.query}".`)
      response.data({ query: args.query, items: [], count: 0 })
      return
    }

    const lines: string[] = [
      `Found ${items.length} history items matching "${args.query}":`,
      '',
    ]

    for (const item of items) {
      const date = item.lastVisitTime
        ? new Date(item.lastVisitTime).toISOString()
        : 'Unknown date'
      lines.push(`[${item.id}] ${item.title || 'Untitled'}`)
      lines.push(`    ${item.url || 'No URL'}`)
      lines.push(`    Last visited: ${date}`)
      if (item.visitCount !== undefined) {
        lines.push(`    Visit count: ${item.visitCount}`)
      }
      lines.push('')
    }

    response.text(lines.join('\n'))
    response.data({
      query: args.query,
      items,
      count: items.length,
    })
  },
})

export const get_recent_history = defineManagementTool({
  name: 'get_recent_history',
  description: 'Get most recent browser history items',
  input: z.object({
    maxResults: z
      .number()
      .optional()
      .describe('Number of recent items to retrieve (default: 20)'),
  }),
  output: z.object({
    items: z.array(historyItemSchema),
    count: z.number(),
  }),
  handler: async (args, ctx, response) => {
    const items = await ctx.browser.getRecentHistory(args.maxResults)

    if (items.length === 0) {
      response.text('No recent history items.')
      response.data({ items: [], count: 0 })
      return
    }

    const lines: string[] = [
      `Retrieved ${items.length} recent history items:`,
      '',
    ]

    for (const item of items) {
      const date = item.lastVisitTime
        ? new Date(item.lastVisitTime).toISOString()
        : 'Unknown date'
      lines.push(`[${item.id}] ${item.title || 'Untitled'}`)
      lines.push(`    ${item.url || 'No URL'}`)
      lines.push(`    ${date}`)
      if (item.visitCount !== undefined) {
        lines.push(`    Visits: ${item.visitCount}`)
      }
      lines.push('')
    }

    response.text(lines.join('\n'))
    response.data({
      items,
      count: items.length,
    })
  },
})

export const delete_history_url = defineManagementTool({
  name: 'delete_history_url',
  description: 'Delete a specific URL from browser history',
  input: z.object({
    url: z.string().describe('URL to delete from history'),
  }),
  output: z.object({
    action: z.literal('delete_history_url'),
    url: z.string(),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.deleteHistoryUrl(args.url)
    response.text(`Deleted ${args.url} from history`)
    response.data({ action: 'delete_history_url', url: args.url })
  },
})

export const delete_history_range = defineManagementTool({
  name: 'delete_history_range',
  description: 'Delete browser history within a time range',
  input: z.object({
    startTime: z.number().describe('Start time as epoch ms'),
    endTime: z.number().describe('End time as epoch ms'),
  }),
  output: z.object({
    action: z.literal('delete_history_range'),
    startTime: z.number(),
    endTime: z.number(),
    startIso: z.string(),
    endIso: z.string(),
  }),
  handler: async (args, ctx, response) => {
    await ctx.browser.deleteHistoryRange(args.startTime, args.endTime)
    const start = new Date(args.startTime).toISOString()
    const end = new Date(args.endTime).toISOString()
    response.text(`Deleted history from ${start} to ${end}`)
    response.data({
      action: 'delete_history_range',
      startTime: args.startTime,
      endTime: args.endTime,
      startIso: start,
      endIso: end,
    })
  },
})
