/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * In-memory LRU mapping `pageId -> ScreencastFrame` plus a per-page
 * consecutive-failure counter that drives the poller's backoff
 * behaviour. Cache writes bump recency; reads do not. When the cache
 * crosses `maxEntries`, the oldest pageId is evicted.
 *
 * Failure tracking is a sibling map keyed on `pageId`. Three
 * consecutive failures put the page in backoff; the poller checks
 * `isInBackoff` to decide whether to retry. Backoff lifts when the
 * registry's `lastToolAt` for that page advances past the recorded
 * `lastFailureAt` (the agent did something new since we last failed),
 * or when a successful capture writes a frame and clears the counter.
 *
 * Poller frames stay ephemeral by design: they are never persisted
 * to the audit log nor to disk. The audit-log narrative reflects what
 * the agent chose to capture, not what the cockpit polled in the
 * background. The same precedent `tab-group-ops.ts` set for
 * cockpit-driven `executeTool` calls bypassing the audit hook applies
 * here.
 */

import { logger } from '../lib/logger'

export interface ScreencastFrame {
  /** Raw base64; no `data:` prefix. */
  jpegBase64: string
  /** Unix ms when the frame was captured by the poller. */
  capturedAt: number
  /** Decoded byte length; diagnostic + future eviction heuristics. */
  byteLength: number
}

export interface ScreencastCacheOptions {
  maxEntries: number
  maxConsecutiveFailures: number
}

interface FailureState {
  consecutive: number
  lastFailureAt: number
}

export interface ScreencastCache {
  get(pageId: number): ScreencastFrame | null
  set(pageId: number, frame: ScreencastFrame): void
  /** Drop both the cached frame and the failure state for a pageId. */
  delete(pageId: number): void
  /**
   * Drop the cached frame but keep the failure state. Used by the
   * poller when entering backoff: the prior JPEG would be stale
   * (the agent has navigated away under us) so we hide it, but the
   * failure counter must persist so isInBackoff() keeps returning
   * true until the agent does something new.
   */
  clearFrame(pageId: number): void
  markFailure(pageId: number): boolean
  clearFailure(pageId: number): void
  isInBackoff(pageId: number, sinceMs: number): boolean
  snapshot(): ReadonlyMap<number, ScreencastFrame>
  /** Test-only: forget everything. */
  resetForTesting(): void
}

export function createScreencastCache(
  options: ScreencastCacheOptions,
): ScreencastCache {
  const { maxEntries, maxConsecutiveFailures } = options
  const frames = new Map<number, ScreencastFrame>()
  const failures = new Map<number, FailureState>()

  return {
    get(pageId) {
      return frames.get(pageId) ?? null
    },
    set(pageId, frame) {
      // Delete-then-insert bumps the key to the end of the Map's
      // insertion order, giving us cheap LRU semantics without a
      // doubly-linked-list.
      if (frames.has(pageId)) frames.delete(pageId)
      frames.set(pageId, frame)
      if (frames.size > maxEntries) {
        const oldest = frames.keys().next().value
        if (oldest !== undefined) {
          frames.delete(oldest)
        }
      }
    },
    delete(pageId) {
      frames.delete(pageId)
      failures.delete(pageId)
    },
    clearFrame(pageId) {
      frames.delete(pageId)
    },
    markFailure(pageId) {
      const prev = failures.get(pageId)
      const next: FailureState = {
        consecutive: (prev?.consecutive ?? 0) + 1,
        lastFailureAt: Date.now(),
      }
      failures.set(pageId, next)
      const inBackoff = next.consecutive >= maxConsecutiveFailures
      if (inBackoff) {
        logger.warn('screencast: page enters backoff', {
          pageId,
          consecutive: next.consecutive,
        })
      }
      return inBackoff
    },
    clearFailure(pageId) {
      failures.delete(pageId)
    },
    isInBackoff(pageId, sinceMs) {
      const state = failures.get(pageId)
      if (!state) return false
      if (state.consecutive < maxConsecutiveFailures) return false
      // The agent has done something new since the last failure.
      // Lift the backoff so the next tick retries.
      if (sinceMs > state.lastFailureAt) return false
      return true
    },
    snapshot() {
      return frames
    },
    resetForTesting() {
      frames.clear()
      failures.clear()
    },
  }
}

export const SCREENCAST_CACHE_MAX_ENTRIES = 50
export const SCREENCAST_CACHE_MAX_CONSECUTIVE_FAILURES = 3

/** Process-wide singleton consumed by the poller + the route. */
export const screencastCache = createScreencastCache({
  maxEntries: SCREENCAST_CACHE_MAX_ENTRIES,
  maxConsecutiveFailures: SCREENCAST_CACHE_MAX_CONSECUTIVE_FAILURES,
})
