/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure unit tests for `buildTabView`. Lives in `tab-view.ts` (not
 * `replay.data.ts`) so bun test does not import the react-query-kit
 * hook graph, which sibling tests `mock.module`-poison globally.
 */

import { describe, expect, it } from 'bun:test'
import type { ReplayEvent, ReplayFrame } from '@/modules/api/replay.hooks'
import { type BuildTabViewInput, buildTabView } from './tab-view'

function frame(
  t: number,
  pageId: number | null,
  extra: Partial<ReplayFrame> = {},
): ReplayFrame {
  return {
    t,
    kind: 'action',
    verb: 'read',
    node: 'test',
    caption: 'test',
    pageId,
    ...extra,
  }
}

function event(ts: number, tabPageId: number): ReplayEvent {
  return { sessionId: 'test', tabPageId, type: 3, data: {}, ts }
}

function makeInput(
  overrides: Partial<BuildTabViewInput> = {},
): BuildTabViewInput {
  return {
    frames: [],
    eventsForTab: () => [],
    startedAtMs: 1_000_000,
    ...overrides,
  }
}

describe('buildTabView', () => {
  it('returns EMPTY for null tabPageId', () => {
    const v = buildTabView(makeInput(), null)
    expect(v.frames).toEqual([])
    expect(v.events).toEqual([])
    expect(v.totalSeconds).toBe(0)
  })

  it('returns EMPTY when the tab has no frames AND no events', () => {
    const v = buildTabView(makeInput({ frames: [frame(5, 1)] }), 42)
    expect(v.frames).toEqual([])
    expect(v.events).toEqual([])
    expect(v.totalSeconds).toBe(0)
  })

  it('filters frames to only the target tab', () => {
    const v = buildTabView(
      makeInput({
        frames: [frame(1, 1), frame(2, 4), frame(3, 1), frame(4, 5)],
      }),
      1,
    )
    expect(v.frames).toHaveLength(2)
    expect(v.frames.map((f) => f.pageId)).toEqual([1, 1])
  })

  it('shifts frame `t` to be tab-relative (first frame at t=0)', () => {
    // Session started at 1_000_000 ms; frame at t=5 means the tab's
    // first activity is 5 seconds into the session. In tab-relative
    // time that first frame is at t=0.
    const v = buildTabView(
      makeInput({
        frames: [frame(5, 7), frame(8, 7), frame(12, 7)],
        eventsForTab: () => [event(1_005_000, 7), event(1_012_000, 7)],
      }),
      7,
    )
    expect(v.frames.map((f) => f.t)).toEqual([0, 3, 7])
  })

  it('totalSeconds = tab activity window (last event - first event)', () => {
    const v = buildTabView(
      makeInput({
        frames: [frame(3, 1), frame(6, 1)],
        eventsForTab: () => [event(1_003_000, 1), event(1_007_500, 1)],
      }),
      1,
    )
    // 7500 - 3000 = 4500 ms = 4.5s
    expect(v.totalSeconds).toBeCloseTo(4.5)
  })

  it('falls back to frame timespan when no events exist', () => {
    const v = buildTabView(
      makeInput({
        frames: [frame(2, 9), frame(10, 9)],
        eventsForTab: () => [],
      }),
      9,
    )
    // 10 - 2 = 8 seconds
    expect(v.totalSeconds).toBe(8)
    expect(v.frames.map((f) => f.t)).toEqual([0, 8])
  })

  it('preserves other frame fields when shifting `t`', () => {
    const v = buildTabView(
      makeInput({
        frames: [frame(5, 3, { verb: 'navigate', url: 'https://example.com' })],
      }),
      3,
    )
    expect(v.frames[0]?.verb).toBe('navigate')
    expect(v.frames[0]?.url).toBe('https://example.com')
    expect(v.frames[0]?.pageId).toBe(3)
    expect(v.frames[0]?.t).toBe(0)
  })
})
