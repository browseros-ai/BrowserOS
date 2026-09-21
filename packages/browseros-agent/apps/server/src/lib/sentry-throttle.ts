/**
 * @license
 * Copyright 2025 BrowserOS
 */

import type { ErrorEvent, StackFrame } from '@sentry/bun'

/**
 * Per-issue Sentry event budget so one runaway error cannot drain the quota.
 * A retry loop, a full disk, or a missing table hit on every request otherwise
 * emits one event per failure without limit; this caps each issue to a handful
 * of representative events per rolling window and counts the rest as suppressed.
 */

export interface EventBudgetConfig {
  windowMs: number
  maxPerKey: number
  maxKeys: number
}

export const DEFAULT_EVENT_BUDGET: EventBudgetConfig = {
  windowMs: 60 * 60 * 1000,
  maxPerKey: 5,
  maxKeys: 2000,
}

// Innermost frames identify the issue the way Sentry groups it. Prefer in-app
// frames, fall back to all frames, and key on module + filename + function so
// two same-named functions in different files stay distinct. The exception
// message is deliberately excluded: a loop whose message carries a varying id
// (a request id, a timeout handle) would otherwise mint a fresh key per event
// and escape the cap entirely.
function frameSignature(frames: StackFrame[] | undefined): string {
  if (!frames?.length) return ''
  const inApp = frames.filter((frame) => frame.in_app)
  const chosen = (inApp.length > 0 ? inApp : frames).slice(-3)
  return chosen
    .map(
      (frame) =>
        `${frame.module ?? ''}:${frame.filename ?? ''}:${frame.function ?? ''}`,
    )
    .join('>')
}

/**
 * Stable key that matches how Sentry groups the event, so a looping error
 * collapses to one bucket. An explicit fingerprint wins; otherwise fall back to
 * the exception type plus a signature of the innermost frames.
 */
export function throttleKey(event: ErrorEvent): string {
  if (event.fingerprint?.length) return event.fingerprint.join('|')
  const exception = event.exception?.values?.at(-1)
  const type = exception?.type ?? event.level ?? 'event'
  const where =
    frameSignature(exception?.stacktrace?.frames) ||
    event.transaction ||
    exception?.value ||
    ''
  return `${type}|${where}`
}

interface Bucket {
  // Ascending timestamps of admitted events still inside the window, bounded to
  // maxPerKey entries.
  admitted: number[]
  suppressed: number
  lastSeen: number
}

export interface EventBudget {
  /**
   * Number of events suppressed for this issue since its last admitted event
   * (attach it to the admitted event), or null to drop this event.
   */
  admit(event: ErrorEvent, now?: number): number | null
}

export function createEventBudget(
  config: EventBudgetConfig = DEFAULT_EVENT_BUDGET,
): EventBudget {
  const buckets = new Map<string, Bucket>()

  function enforceBound(now: number): void {
    if (buckets.size <= config.maxKeys) return
    for (const [key, bucket] of buckets) {
      if (
        now - bucket.lastSeen >= config.windowMs &&
        bucket.admitted.length === 0
      ) {
        buckets.delete(key)
      }
    }
    while (buckets.size > config.maxKeys) {
      let oldestKey: string | undefined
      let oldestSeen = Number.POSITIVE_INFINITY
      for (const [key, bucket] of buckets) {
        if (bucket.lastSeen < oldestSeen) {
          oldestSeen = bucket.lastSeen
          oldestKey = key
        }
      }
      if (oldestKey === undefined) break
      buckets.delete(oldestKey)
    }
  }

  return {
    admit(event, now = Date.now()) {
      const key = throttleKey(event)
      let bucket = buckets.get(key)
      if (!bucket) {
        bucket = { admitted: [], suppressed: 0, lastSeen: now }
        buckets.set(key, bucket)
        enforceBound(now)
      }
      bucket.lastSeen = now
      // Rolling window: expire admissions older than windowMs so the cap holds
      // over any windowMs span. A fixed window that reset on a boundary would
      // let nearly 2x maxPerKey through around the edge.
      const cutoff = now - config.windowMs
      while (bucket.admitted.length > 0) {
        const oldest = bucket.admitted[0]
        if (oldest === undefined || oldest > cutoff) break
        bucket.admitted.shift()
      }
      if (bucket.admitted.length < config.maxPerKey) {
        const suppressed = bucket.suppressed
        bucket.admitted.push(now)
        bucket.suppressed = 0
        return suppressed
      }
      bucket.suppressed += 1
      return null
    },
  }
}
