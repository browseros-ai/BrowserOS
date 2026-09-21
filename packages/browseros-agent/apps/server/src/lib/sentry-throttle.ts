/**
 * @license
 * Copyright 2025 BrowserOS
 */

import type { ErrorEvent } from '@sentry/bun'

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

/**
 * Stable key that matches how Sentry groups the event, so a looping error
 * collapses to one bucket. An explicit fingerprint wins; otherwise fall back to
 * the exception type plus the innermost frame.
 */
export function throttleKey(event: ErrorEvent): string {
  if (event.fingerprint?.length) return event.fingerprint.join('|')
  const exception = event.exception?.values?.at(-1)
  const type = exception?.type ?? event.level ?? 'event'
  const innermost = exception?.stacktrace?.frames?.at(-1)
  const where =
    innermost?.function ?? event.transaction ?? exception?.value ?? ''
  return `${type}|${where}`
}

interface Bucket {
  windowStart: number
  sent: number
  suppressed: number
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
      if (now - bucket.windowStart >= config.windowMs) buckets.delete(key)
    }
    while (buckets.size > config.maxKeys) {
      let oldestKey: string | undefined
      let oldestStart = Number.POSITIVE_INFINITY
      for (const [key, bucket] of buckets) {
        if (bucket.windowStart < oldestStart) {
          oldestStart = bucket.windowStart
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
        bucket = { windowStart: now, sent: 0, suppressed: 0 }
        buckets.set(key, bucket)
        enforceBound(now)
      } else if (now - bucket.windowStart >= config.windowMs) {
        // Roll the per-window send counter but keep the suppressed tally: it is
        // "since last admitted event", so it survives the window boundary and is
        // reported on the next event that gets through.
        bucket.windowStart = now
        bucket.sent = 0
      }
      if (bucket.sent < config.maxPerKey) {
        const suppressed = bucket.suppressed
        bucket.sent += 1
        bucket.suppressed = 0
        return suppressed
      }
      bucket.suppressed += 1
      return null
    },
  }
}
