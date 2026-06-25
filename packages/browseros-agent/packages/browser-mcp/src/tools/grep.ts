import { z } from 'zod'
import { defineTool, errorResult, textResult } from './framework'
import { wrapUntrusted } from './trust-boundary'

export const grep = defineTool({
  name: 'grep',
  description:
    'Search the page without dumping it. over="ax" greps the snapshot lines (matches keep their [ref=eN]); over="content" greps visible text. Returns matching lines.',
  input: z.object({
    page: z.number().int(),
    pattern: z.string().describe('Case-insensitive regular expression.'),
    over: z.enum(['ax', 'content']).default('ax'),
    limit: z.number().optional().describe('Max matching lines (default 50).'),
  }),
  annotations: { readOnlyHint: true },
  handler: async (args, ctx) => {
    let regex: RegExp
    try {
      regex = new RegExp(args.pattern, 'i')
    } catch (err) {
      return errorResult(
        `grep: invalid regex - ${err instanceof Error ? err.message : String(err)}`,
      )
    }

    let haystack: string
    if (args.over === 'ax') {
      haystack = (await ctx.session.observe(args.page).snapshot()).text
    } else {
      const { session } = await ctx.session.pages.getSession(args.page)
      const result = await session.Runtime.evaluate({
        expression: "(document.body?.innerText ?? '')",
        returnByValue: true,
      })
      haystack = (result.result?.value as string) ?? ''
    }

    const matches = haystack
      .split('\n')
      .filter((line) => regex.test(line))
      .slice(0, args.limit ?? 50)
    if (matches.length === 0) {
      return textResult('no matches', {
        page: args.page,
        over: args.over,
        count: 0,
        matches: [],
      })
    }

    const origin = ctx.session.pages.getInfo(args.page)?.url ?? 'unknown'
    return textResult(wrapUntrusted(matches.join('\n'), origin), {
      page: args.page,
      over: args.over,
      count: matches.length,
      matches,
    })
  },
})
