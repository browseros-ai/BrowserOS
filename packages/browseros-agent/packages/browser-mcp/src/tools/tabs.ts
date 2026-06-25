import { z } from 'zod'
import { defineTool, errorResult, textResult } from './framework'

export const tabs = defineTool({
  name: 'tabs',
  description:
    'Manage browser tabs: list open pages (with their page ids), show the active page, open a new page, or close one. Use the returned page id with snapshot/act/navigate.',
  input: z.object({
    action: z.enum(['list', 'active', 'new', 'close']).default('list'),
    url: z
      .string()
      .optional()
      .describe('URL for action="new" (defaults to about:blank).'),
    background: z
      .boolean()
      .default(true)
      .describe('Open without stealing focus for action="new".'),
    hidden: z
      .boolean()
      .default(false)
      .describe('Create in a hidden window for action="new".'),
    page: z.number().int().optional().describe('Page id for action="close".'),
  }),
  annotations: { openWorldHint: true },
  handler: async (args, ctx) => {
    switch (args.action) {
      case 'list': {
        const pages = await ctx.session.pages.list()
        const lines = pages.map(formatPageLine)
        return textResult(lines.join('\n') || '(no open pages)', {
          pages: pages.map((p) => ({
            page: p.pageId,
            url: p.url,
            title: p.title,
          })),
        })
      }
      case 'active': {
        const page = await ctx.session.pages.getActive()
        if (!page) {
          return errorResult('tabs active: no active page found.')
        }
        return textResult(`Active page: ${formatPageLine(page)}`, {
          action: 'active',
          page,
        })
      }
      case 'new': {
        const page = await ctx.session.pages.newPage(
          args.url ?? 'about:blank',
          {
            background: args.background,
            hidden: args.hidden,
            windowId: ctx.defaultWindowId,
            tabGroupId: ctx.defaultTabGroupId,
          },
        )
        return textResult(`opened page ${page}`, { page })
      }
      case 'close': {
        if (args.page === undefined) {
          return errorResult('tabs close: page is required.')
        }
        await ctx.session.pages.close(args.page)
        return textResult(`closed page ${args.page}`, { page: args.page })
      }
      default:
        return errorResult('tabs: unsupported action.')
    }
  },
})

function formatPageLine(page: { pageId: number; url: string; title?: string }) {
  return `[${page.pageId}] ${page.url}${page.title ? ` (${page.title})` : ''}`
}
