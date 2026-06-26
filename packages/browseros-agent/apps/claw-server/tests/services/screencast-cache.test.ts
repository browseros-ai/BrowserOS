/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import {
  createScreencastCache,
  type ScreencastFrame,
} from '../../src/services/screencast-cache'

function frame(byteLength = 100): ScreencastFrame {
  return {
    jpegBase64: 'AAAA',
    capturedAt: Date.now(),
    byteLength,
  }
}

describe('screencast cache', () => {
  it('get returns null for an unknown pageId', () => {
    const cache = createScreencastCache({
      maxEntries: 10,
      maxConsecutiveFailures: 3,
    })
    expect(cache.get(1)).toBeNull()
  })

  it('set then get round-trips the frame', () => {
    const cache = createScreencastCache({
      maxEntries: 10,
      maxConsecutiveFailures: 3,
    })
    const f = frame()
    cache.set(7, f)
    expect(cache.get(7)).toBe(f)
  })

  it('evicts the oldest entry once maxEntries is crossed', () => {
    const cache = createScreencastCache({
      maxEntries: 3,
      maxConsecutiveFailures: 3,
    })
    cache.set(1, frame())
    cache.set(2, frame())
    cache.set(3, frame())
    cache.set(4, frame())
    expect(cache.get(1)).toBeNull()
    expect(cache.get(2)).not.toBeNull()
    expect(cache.get(3)).not.toBeNull()
    expect(cache.get(4)).not.toBeNull()
  })

  it('re-inserting an existing pageId bumps it to the end (LRU)', () => {
    const cache = createScreencastCache({
      maxEntries: 3,
      maxConsecutiveFailures: 3,
    })
    cache.set(1, frame())
    cache.set(2, frame())
    cache.set(3, frame())
    // Refresh pageId 1 so it becomes the freshest; 2 should evict next.
    cache.set(1, frame())
    cache.set(4, frame())
    expect(cache.get(2)).toBeNull()
    expect(cache.get(1)).not.toBeNull()
    expect(cache.get(3)).not.toBeNull()
    expect(cache.get(4)).not.toBeNull()
  })

  it('delete removes the entry and the failure state', () => {
    const cache = createScreencastCache({
      maxEntries: 10,
      maxConsecutiveFailures: 3,
    })
    cache.set(5, frame())
    cache.markFailure(5)
    cache.delete(5)
    expect(cache.get(5)).toBeNull()
    // After delete, the failure history is gone so a single new
    // failure should not put us in backoff.
    cache.markFailure(5)
    expect(cache.isInBackoff(5, 0)).toBe(false)
  })

  it('markFailure increments and trips at threshold', () => {
    const cache = createScreencastCache({
      maxEntries: 10,
      maxConsecutiveFailures: 3,
    })
    expect(cache.markFailure(9)).toBe(false)
    expect(cache.markFailure(9)).toBe(false)
    expect(cache.markFailure(9)).toBe(true)
  })

  it('clearFailure resets the counter', () => {
    const cache = createScreencastCache({
      maxEntries: 10,
      maxConsecutiveFailures: 2,
    })
    cache.markFailure(2)
    cache.markFailure(2)
    expect(cache.isInBackoff(2, 0)).toBe(true)
    cache.clearFailure(2)
    expect(cache.isInBackoff(2, 0)).toBe(false)
  })

  it('isInBackoff lifts when sinceMs is past lastFailureAt', () => {
    const cache = createScreencastCache({
      maxEntries: 10,
      maxConsecutiveFailures: 2,
    })
    cache.markFailure(4)
    cache.markFailure(4)
    // Pretend the registry says the agent did a new tool dispatch
    // far in the future; backoff should lift.
    expect(cache.isInBackoff(4, Number.MAX_SAFE_INTEGER)).toBe(false)
    // Whereas a stamp from the deep past keeps us in backoff.
    expect(cache.isInBackoff(4, 0)).toBe(true)
  })

  it('snapshot reflects current entries', () => {
    const cache = createScreencastCache({
      maxEntries: 10,
      maxConsecutiveFailures: 3,
    })
    cache.set(1, frame())
    cache.set(2, frame())
    const snap = cache.snapshot()
    expect(snap.size).toBe(2)
    expect(snap.has(1)).toBe(true)
    expect(snap.has(2)).toBe(true)
  })
})
