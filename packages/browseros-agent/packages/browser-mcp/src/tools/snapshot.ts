import { z } from 'zod'
import { defineTool, textResult } from './framework'
import { formatSnapshotResult } from './snapshot-format'

export const snapshot = defineTool({
  name: 'snapshot',
  description:
    'Capture the page as an indented accessibility tree. Each actionable element carries a stable [ref=eN] that can be passed to `act`. Iframe content is stitched inline. Refs are invalidated by navigation or large DOM changes.',
  input: z.object({
    page: z.number().int().describe('Page id from `tabs` or `navigate`.'),
  }),
  annotations: { title: 'Snapshot accessibility tree', readOnlyHint: true },
  handler: async (args, ctx) => {
    const { text } = await ctx.session.observe(args.page).snapshot()
    const origin = ctx.session.pages.getInfo(args.page)?.url ?? 'unknown'
    const formatted = await formatSnapshotResult(text, origin)
    return textResult(formatted.text, {
      page: args.page,
      ...formatted.structured,
    })
  },
})
