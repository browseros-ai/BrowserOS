import { TOOL_LIMITS } from '@browseros/shared/constants/limits'
import { z } from 'zod/v4'
import { clampTimeout, defineTool, errorResult, textResult } from './framework'
import { writeTempToolOutputFile } from './output-file'
import { wrapUntrusted } from './trust-boundary'

const DEFAULT_TIMEOUT_MS = 30_000
const MAX_TIMEOUT_MS = 30_000

// Ceiling for the opt-in `maxChars`: a caller may pull up to this much of a large
// result inline instead of having it spilled to a local file. The default inline
// size stays small (INLINE_PAGE_CONTENT_MAX_CHARS) so ordinary results do not
// flood the model's context.
const MAX_INLINE_OVERRIDE_CHARS = 200_000

const DESCRIPTION = `Evaluate JavaScript in a page context through CDP Runtime.evaluate. Use this for page-state reads or small DOM scripts that are awkward with read/grep. Return a value to read it back. \`timeout\` is capped at 30000 ms; for page work that needs longer, start it on the page and poll with short follow-up calls rather than one long evaluate. A result larger than the inline limit is truncated and its full text is written to a local file whose path a remote MCP client cannot read; return only what you need, or raise \`maxChars\` to receive more of the value inline.`

export const evaluate = defineTool({
  name: 'evaluate',
  description: DESCRIPTION,
  input: z
    .object({
      page: z.number().int().describe('Page id from `tabs`.'),
      code: z
        .string()
        .describe(
          'Async-capable JS body evaluated inside the page. Use `return` to read a value.',
        ),
      timeout: z
        .number()
        .optional()
        .describe(
          'Max evaluation time in ms. Hard cap: 30000 (larger values are clamped to it). For work longer than 30s, start it on the page and poll the result with short follow-up calls instead of one long evaluate.',
        ),
      maxChars: z
        .number()
        .optional()
        .describe(
          'Max size of the result kept inline, in characters (default 5000, max 200000). A larger result is truncated inline and its full text is written to a local file, whose path a remote MCP client cannot open; raise this to receive more of the value inline.',
        ),
    })
    .strict(),
  annotations: {
    title: 'Run JavaScript in page',
    destructiveHint: true,
    openWorldHint: true,
  },
  handler: async (args, ctx) => {
    const { session } = await ctx.session.pages.getSession(args.page)
    const timeout = clampTimeout(
      args.timeout,
      DEFAULT_TIMEOUT_MS,
      MAX_TIMEOUT_MS,
    )
    const result = await session.Runtime.evaluate({
      expression: wrapAsAsyncIife(args.code),
      returnByValue: true,
      awaitPromise: true,
      timeout,
      userGesture: true,
    })

    if (result.exceptionDetails) {
      return errorResult(
        `evaluate: ${
          result.exceptionDetails.exception?.description ??
          result.exceptionDetails.text
        }`,
      )
    }

    const value = result.result?.value ?? result.result?.description
    const text = value === undefined ? 'undefined' : safeStringify(value)
    const origin = ctx.session.pages.getInfo(args.page)?.url ?? 'unknown'
    const inlineLimit = resolveInlineLimit(args.maxChars)
    if (text.length > inlineLimit) {
      const excerpt = text.slice(0, inlineLimit)
      const wrappedText = wrapUntrusted(text, origin)
      const contentLength = wrappedText.length
      try {
        const path = await writeTempToolOutputFile({
          toolName: 'evaluate',
          extension: 'txt',
          content: wrappedText,
        })
        return textResult(
          [
            wrapUntrusted(excerpt, origin),
            `Evaluate result truncated at ${inlineLimit} chars. Full result (${text.length} chars) saved to: ${path}`,
          ].join('\n\n'),
          {
            page: args.page,
            contentLength,
            writtenToFile: true,
            path,
          },
        )
      } catch (error) {
        const saveError = error instanceof Error ? error.message : String(error)
        return textResult(
          [
            wrapUntrusted(excerpt, origin),
            `Evaluate result truncated at ${inlineLimit} chars. Full result (${text.length} chars) could not be saved to a BrowserOS output file: ${saveError}`,
          ].join('\n\n'),
          {
            page: args.page,
            contentLength,
            writtenToFile: false,
            outputWriteFailed: true,
            error: saveError,
          },
        )
      }
    }

    return textResult(wrapUntrusted(text, origin), {
      page: args.page,
      value,
    })
  },
})

function wrapAsAsyncIife(code: string): string {
  return `(async () => {\n${code}\n})()`
}

function safeStringify(value: unknown): string {
  if (typeof value === 'string') return value
  try {
    return JSON.stringify(value, null, 2) ?? String(value)
  } catch {
    return String(value)
  }
}

/**
 * Resolves the inline size budget: the caller's `maxChars` capped at
 * `MAX_INLINE_OVERRIDE_CHARS`, or the small default when unset. A result larger
 * than this is truncated inline and its full text spilled to a file. Measured in
 * the same units as `String.length` (UTF-16 code units) to match the inline limit.
 */
export function resolveInlineLimit(maxChars: number | undefined): number {
  if (maxChars === undefined) return TOOL_LIMITS.INLINE_PAGE_CONTENT_MAX_CHARS
  return Math.min(Math.max(0, Math.floor(maxChars)), MAX_INLINE_OVERRIDE_CHARS)
}
