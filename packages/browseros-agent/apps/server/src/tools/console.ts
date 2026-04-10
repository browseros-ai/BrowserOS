import { CONTENT_LIMITS } from '@browseros/shared/constants/limits'
import { z } from 'zod'
import type { ConsoleLevel } from '../browser/console-collector'
import { defineToolWithCategory } from './framework'

const pageParam = z.number().describe('Page ID (from list_pages)')
const defineObservationTool = defineToolWithCategory('observation')

export const get_console_logs = defineObservationTool({
  name: 'get_console_logs',
  description:
    'Get browser console output (logs, warnings, errors, exceptions) for a page. Use to debug JavaScript errors, failed network requests, or unexpected page behavior.',
  input: z.object({
    page: pageParam,
    level: z
      .enum(['error', 'warning', 'info', 'debug'])
      .default('info')
      .describe(
        'Minimum severity level. "error" = errors only, "warning" = errors + warnings, "info" = errors + warnings + logs (default), "debug" = everything',
      ),
    search: z
      .string()
      .optional()
      .describe('Filter entries containing this text (case-insensitive)'),
    limit: z
      .number()
      .min(1)
      .max(CONTENT_LIMITS.CONSOLE_MAX_LIMIT)
      .optional()
      .describe(
        `Max entries to return (default ${CONTENT_LIMITS.CONSOLE_DEFAULT_LIMIT}, max ${CONTENT_LIMITS.CONSOLE_MAX_LIMIT}). Returns most recent entries.`,
      ),
    clear: z
      .boolean()
      .default(false)
      .describe('Clear the console buffer after reading'),
  }),
  output: z.object({
    entries: z.array(
      z.object({
        source: z.enum(['console', 'exception', 'browser']),
        level: z.enum(['error', 'warning', 'info', 'debug']),
        text: z.string(),
        url: z.string().optional(),
        lineNumber: z.number().optional(),
        timestamp: z.number(),
      }),
    ),
    totalCount: z.number(),
    returnedCount: z.number(),
  }),
  handler: async (args, ctx, response) => {
    const result = await ctx.browser.getConsoleLogs(args.page, {
      level: args.level as ConsoleLevel,
      search: args.search,
      limit: args.limit,
      clear: args.clear,
    })

    // Empty results
    if (result.entries.length === 0) {
      response.text(
        result.totalCount === 0
          ? `No console output for page ${args.page}.`
          : `No entries match the filter (${result.totalCount} total entries in buffer).`,
      )
      response.data({
        entries: [],
        totalCount: result.totalCount,
        returnedCount: 0,
      })
      return
    }

    // Format each entry as [level] text — url:line
    const lines = result.entries.map((e) => {
      const location = e.url
        ? ` — ${e.url}${e.lineNumber !== undefined ? `:${e.lineNumber}` : ''}`
        : ''
      return `[${e.level}] ${e.text}${location}`
    })

    // Build header with count info
    const header =
      result.entries.length < result.totalCount
        ? `Console logs for page ${args.page} (showing ${result.entries.length} of ${result.totalCount}, level ≥ ${args.level}):`
        : `Console logs for page ${args.page} (${result.entries.length} entries, level ≥ ${args.level}):`

    response.text(`${header}\n\n${lines.join('\n')}`)
    response.data({
      entries: result.entries,
      totalCount: result.totalCount,
      returnedCount: result.entries.length,
    })
  },
})
