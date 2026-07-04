import { TOOL_LIMITS } from '@browseros/shared/constants/limits'
import { z } from 'zod'
import { defineTool, errorResult, textResult } from './framework'
import { writeTempToolOutputFile } from './output-file'
import { wrapUntrusted } from './trust-boundary'

const DEFAULT_LIMIT = 50
const LINE_TRUNCATION_MARKER = '... [truncated]'

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

    const limit = clampLimit(args.limit)
    const matches = haystack
      .split('\n')
      .filter((line) => regex.test(line))
      .slice(0, limit)
    if (matches.length === 0) {
      return textResult('no matches', {
        page: args.page,
        over: args.over,
        count: 0,
      })
    }

    const origin = ctx.session.pages.getInfo(args.page)?.url ?? 'unknown'
    const renderedMatches = matches.map(clampRenderedLine)
    const fullMatchesText = matches.join('\n')
    const renderedText = renderedMatches.join('\n')
    const lineTruncated = renderedMatches.some((line, index) => {
      return line !== matches[index]
    })
    const totalTruncated =
      renderedText.length > TOOL_LIMITS.INLINE_PAGE_CONTENT_MAX_CHARS
    const inlineText = clampText(
      renderedText,
      TOOL_LIMITS.INLINE_PAGE_CONTENT_MAX_CHARS,
    )

    if (lineTruncated || totalTruncated) {
      try {
        const path = await writeTempToolOutputFile({
          toolName: 'grep',
          extension: 'txt',
          content: wrapUntrusted(fullMatchesText, origin),
        })
        return textResult(
          [
            wrapUntrusted(inlineText, origin),
            `Grep output truncated for ${matches.length} match(es). Full matches (${fullMatchesText.length} chars) saved to: ${path}`,
          ].join('\n\n'),
          {
            page: args.page,
            over: args.over,
            count: matches.length,
            truncated: true,
            path,
          },
        )
      } catch (error) {
        const saveError = error instanceof Error ? error.message : String(error)
        return textResult(
          [
            wrapUntrusted(inlineText, origin),
            `Grep output truncated for ${matches.length} match(es). Full matches (${fullMatchesText.length} chars) could not be saved to a BrowserOS output file: ${saveError}`,
          ].join('\n\n'),
          {
            page: args.page,
            over: args.over,
            count: matches.length,
            truncated: true,
            writtenToFile: false,
            outputWriteFailed: true,
            error: saveError,
          },
        )
      }
    }

    return textResult(wrapUntrusted(renderedText, origin), {
      page: args.page,
      over: args.over,
      count: matches.length,
    })
  },
})

function clampLimit(limit: number | undefined): number {
  if (limit === undefined) return DEFAULT_LIMIT
  if (!Number.isFinite(limit)) return DEFAULT_LIMIT
  return Math.min(Math.max(Math.floor(limit), 0), TOOL_LIMITS.GREP_MAX_MATCHES)
}

function clampRenderedLine(line: string): string {
  return clampText(line, TOOL_LIMITS.GREP_MATCH_LINE_MAX_CHARS)
}

function clampText(text: string, maxChars: number): string {
  if (text.length <= maxChars) return text
  const prefixLength = Math.max(0, maxChars - LINE_TRUNCATION_MARKER.length)
  return `${text.slice(0, prefixLength)}${LINE_TRUNCATION_MARKER}`
}
