import { TIMEOUTS } from '@browseros/shared/constants/timeouts'
import { z } from 'zod/v4'
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
// A for="time" pause does no page work (it is an abortable sleep), so it is not
// bound by the 30s page-work polling cap that text/selector waits use. An explicit
// timeout still bounds it from above; a value past this ceiling is rejected (naming
// the cap) rather than silently clamped, so a caller pacing a long in-page loop
// never under-waits without knowing (#2701). Kept below TIMEOUTS.TOOL_CALL so the
// documented maximum still completes within the tool-call budget after dispatch and
// result-building overhead, rather than racing the outer timeout.
const MAX_TIME_WAIT_MS = TIMEOUTS.TOOL_CALL - 30_000

export const wait = defineTool({
  name: 'wait',
  description:
    'Pause before continuing. Prefer acting directly and reading the diff; use wait only when there is no reliable UI signal yet. for="time" (default) pauses for value ms (honored up to 90000; an explicit timeout caps it lower, and a larger value is rejected, not silently shortened); "text" waits for a substring to appear; "selector" waits for a CSS selector to match. value is optional; for "time" it defaults to 2000ms, so calling wait with just a page pauses ~2s.',
  input: z
    .object({
      page: z.number().int(),
      for: z
        .enum(['text', 'selector', 'time'])
        .default('time')
        .describe('What to wait for. Defaults to "time" (a fixed pause).'),
      value: z
        .union([z.string(), z.number()])
        .optional()
        .describe(
          'Optional. For for="time", ms to pause (default 2000, honored up to 90000). For "text"/"selector", the substring or CSS selector to wait for.',
        ),
      timeout: z
        .number()
        .optional()
        .describe(
          'Max wait in ms. For "text"/"selector" it caps polling before giving up (default 2000, capped at 30000). For "time" it optionally caps the pause from above (default: pause for value).',
        ),
    })
    .strict(),
  annotations: { title: 'Wait', readOnlyHint: true },
  handler: async (args, ctx) => {
    const value = args.value === undefined ? undefined : String(args.value)

    if (args.for === 'time') {
      const resolved = resolveTimeWaitMs(value, args.timeout)
      if ('error' in resolved) return errorResult(resolved.error)
      await abortableDelay(resolved.waitMs, ctx.signal)
      return textResult(`waited ${resolved.waitMs}ms`, {
        matched: true,
        waitedMs: resolved.waitMs,
      })
    }

    const timeout = clampTimeout(
      args.timeout,
      DEFAULT_WAIT_TIMEOUT_MS,
      MAX_WAIT_TIMEOUT_MS,
    )
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

/**
 * Resolves a for="time" pause: the requested ms (optionally capped by an explicit
 * timeout), or an error message when it exceeds the ceiling. An explicit timeout
 * still bounds the pause from above, but the pause is no longer forced down to the
 * 30s page-work polling cap; a value past the ceiling is rejected (rather than
 * silently clamped) so a caller never under-waits without knowing (#2701).
 */
export function resolveTimeWaitMs(
  value: string | undefined,
  timeout?: number,
): { waitMs: number } | { error: string } {
  let waitMs = parseWaitMs(value, DEFAULT_PAUSE_MS)
  if (timeout !== undefined && Number.isFinite(timeout) && timeout >= 0) {
    waitMs = Math.min(waitMs, Math.round(timeout))
  }
  if (waitMs > MAX_TIME_WAIT_MS) {
    return {
      error: `wait: ${waitMs}ms exceeds the ${MAX_TIME_WAIT_MS}ms cap for for="time". Pass ${MAX_TIME_WAIT_MS} or less, cap it with timeout, or split it into shorter pauses.`,
    }
  }
  return { waitMs }
}

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
