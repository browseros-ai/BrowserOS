import type {
  GetRecentResult,
  HistoryEntry,
} from '@browseros/cdp-protocol/domains/history'
import { z } from 'zod/v4'
import { defineTool, textResult } from './framework'

const DEFAULT_MAX_RESULTS = 100

export const history = defineTool({
  name: 'history',
  description:
    'Get recent browser history entries, including URLs, titles, visit times, and visit counts. Use maxResults to limit how many entries are returned.',
  input: z
    .object({
      maxResults: z
        .number()
        .int()
        .positive()
        .default(DEFAULT_MAX_RESULTS)
        .describe(
          'Maximum number of recent entries to return. Defaults to 100.',
        ),
    })
    .strict(),
  annotations: {
    title: 'Get recent browser history',
    readOnlyHint: true,
  },
  handler: async (args, ctx) => {
    const { entries } = (await ctx.session.cdp('History.getRecent', {
      maxResults: args.maxResults,
    })) as GetRecentResult
    return textResult(formatHistory(entries), {
      entries,
      count: entries.length,
    })
  },
})

function formatHistory(entries: HistoryEntry[]): string {
  if (entries.length === 0) return '(no history)'

  const lines = [`Recent history (${entries.length}):`, '']
  for (const entry of entries) {
    const destination = entry.title
      ? `${entry.title} (${entry.url})`
      : entry.url
    const visits = entry.visitCount === 1 ? 'visit' : 'visits'
    lines.push(
      `- ${destination} — last visited ${formatVisitTime(entry.lastVisitTime)}; ${entry.visitCount} ${visits}; ${entry.typedCount} typed`,
    )
  }
  return lines.join('\n')
}

function formatVisitTime(milliseconds: number): string {
  const timestamp = new Date(milliseconds)
  if (Number.isNaN(timestamp.getTime())) return String(milliseconds)
  return timestamp.toISOString().replace('.000Z', 'Z')
}
