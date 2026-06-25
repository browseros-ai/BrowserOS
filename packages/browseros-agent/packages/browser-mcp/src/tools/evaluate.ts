import { z } from 'zod'
import { clampTimeout, defineTool, errorResult, textResult } from './framework'
import { wrapUntrusted } from './trust-boundary'

const DEFAULT_TIMEOUT_MS = 30_000
const MAX_TIMEOUT_MS = 30_000

const DESCRIPTION = `Evaluate JavaScript in a page context through CDP Runtime.evaluate. Use this for page-state reads or small DOM scripts that are awkward with read/grep. Return a value to read it back.`

export const evaluate = defineTool({
  name: 'evaluate',
  description: DESCRIPTION,
  input: z.object({
    page: z.number().int().describe('Page id from `tabs`.'),
    code: z
      .string()
      .describe(
        'Async-capable JS body evaluated inside the page. Use `return` to read a value.',
      ),
    timeout: z
      .number()
      .optional()
      .describe('Max evaluation time in ms (default 30000).'),
  }),
  annotations: { openWorldHint: true },
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
