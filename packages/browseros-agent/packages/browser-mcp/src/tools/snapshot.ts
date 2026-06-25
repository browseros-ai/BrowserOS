import { z } from 'zod'
import { defineTool, textResult } from './framework'
import { formatSnapshotResult } from './snapshot-format'

export const snapshot = defineTool({
  name: 'snapshot',
  description:
    'Capture the page as an indented accessibility tree. Each actionable element carries a stable [ref=eN] you pass to `act`. Iframe content is stitched in inline. Re-snapshot after navigation or large changes (refs are invalidated). This is the start of the loop: snapshot -> act -> (reads back a diff).',
  input: z.object({
    page: z.number().int().describe('Page id from `tabs` or `navigate`.'),
  }),
  annotations: { readOnlyHint: true },
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
