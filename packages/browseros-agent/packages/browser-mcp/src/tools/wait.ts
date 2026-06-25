import { z } from 'zod'
import {
  abortableDelay,
  clampTimeout,
  defineTool,
  errorResult,
  textResult,
  throwIfAborted,
} from './framework'

// Default pause for for="time"; kept separate from the timeout so a large
// timeout can't balloon a no-value pause (a model that crams the duration
// into timeout still only pauses this long by default).
export const DEFAULT_PAUSE_MS = 2_000
const DEFAULT_WAIT_TIMEOUT_MS = 2_000
const MAX_WAIT_TIMEOUT_MS = 30_000

export const wait = defineTool({
  name: 'wait',
  description:
    'Pause before continuing. Prefer acting directly and reading the diff; use wait only when there is no reliable UI signal yet. for="time" (default) pauses for value ms; "text" waits for a substring to appear; "selector" waits for a CSS selector to match. value is optional — for "time" it defaults to 2000ms, so calling wait with just a page pauses ~2s.',
  input: z.object({
    page: z.number().int(),
    for: z
      .enum(['text', 'selector', 'time'])
      .default('time')
      .describe('What to wait for. Defaults to "time" (a fixed pause).'),
    value: z
      .union([z.string(), z.number()])
      .optional()
      .describe(
        'Optional. For for="time", ms to pause (default 2000). For "text"/"selector", the substring or CSS selector to wait for.',
      ),
    timeout: z
      .number()
      .optional()
      .describe('Max wait in ms before giving up (default 2000).'),
  }),
  annotations: { readOnlyHint: true },
  handler: async (args, ctx) => {
    const timeout = clampTimeout(
      args.timeout,
      DEFAULT_WAIT_TIMEOUT_MS,
      MAX_WAIT_TIMEOUT_MS,
    )
    const value = args.value === undefined ? undefined : String(args.value)

    if (args.for === 'time') {
      const waitMs = Math.min(parseWaitMs(value, DEFAULT_PAUSE_MS), timeout)
      await abortableDelay(waitMs, ctx.signal)
      return textResult(`waited ${waitMs}ms`, {
        matched: true,
        waitedMs: waitMs,
      })
    }
    if (!value) {
      return errorResult(
        `wait: "value" is required for for="${args.for}" (the text or CSS selector to wait for). To just pause, use for="time".`,
      )
    }

    const { session } = await ctx.session.pages.getSession(args.page)
    const expression =
      args.for === 'text'
        ? `(document.body?.innerText ?? '').includes(${JSON.stringify(value)})`
        : `!!document.querySelector(${JSON.stringify(value)})`

    const deadline = Date.now() + timeout
    while (Date.now() < deadline) {
      throwIfAborted(ctx.signal)
      const result = await session.Runtime.evaluate({
        expression,
        returnByValue: true,
      })
      if (result.result?.value === true) {
        return textResult(`matched (${args.for})`, { matched: true })
      }
      await abortableDelay(
        Math.min(300, Math.max(0, deadline - Date.now())),
        ctx.signal,
      )
    }
    return textResult(`timed out after ${timeout}ms waiting for ${args.for}`, {
      matched: false,
    })
  },
})

/** Parse a millisecond pause value, falling back to `fallback` for missing or invalid input. */
export function parseWaitMs(
  value: string | undefined,
  fallback: number,
): number {
  if (value === undefined || value.trim() === '') return fallback
  const ms = Number(value)
  if (!Number.isFinite(ms) || ms < 0) return fallback
  return Math.round(ms)
}
