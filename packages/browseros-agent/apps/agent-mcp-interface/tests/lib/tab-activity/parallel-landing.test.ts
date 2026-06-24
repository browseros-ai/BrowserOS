/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Drives the exact staggered landing pattern observed when five
 * `tabs.new + snapshot` chains fire in parallel against a live
 * cockpit: even though the HTTP calls are parallel, per-slug
 * dispatch and CDP-per-target serialisation push the registry
 * writes to roughly `t = 0s, 2.3s, 4.3s, 6.3s, 6.4s`. With a 5 s
 * active window the earliest record idles before the latest one
 * even lands. This test pins that the bumped window holds the burst
 * together and that records eventually idle once the window does
 * elapse.
 */

import { beforeEach, describe, expect, it } from 'bun:test'
import type { BrowserSession } from '@browseros/server/browser/core/session'
import {
  ACTIVE_WINDOW_MS,
  createTabActivityRegistry,
  type TabActivityRegistry,
} from '../../../src/lib/tab-activity/registry'

interface FakePageInfo {
  targetId: string
  url: string
  title: string
}

function makeSession(pages: Map<number, FakePageInfo>): BrowserSession {
  return {
    pages: {
      getInfo: (pageId: number) => pages.get(pageId) ?? undefined,
    },
  } as unknown as BrowserSession
}

interface Landing {
  pageId: number
  targetId: string
  url: string
  title: string
  atMs: number
}

// Five-call landings drawn from the parallel-spawn experiment's
// monitor timeline. The exact offsets are not load-bearing; what
// matters is that the spread is longer than the original 5 s window.
const PARALLEL_LANDINGS: ReadonlyArray<Landing> = [
  {
    pageId: 7,
    targetId: 't7',
    url: 'https://www.reddit.com/',
    title: 'Reddit',
    atMs: 0,
  },
  {
    pageId: 8,
    targetId: 't8',
    url: 'https://lobste.rs/',
    title: 'Lobsters',
    atMs: 2_300,
  },
  {
    pageId: 9,
    targetId: 't9',
    url: 'https://hn.algolia.com/',
    title: 'HN Search',
    atMs: 4_300,
  },
  {
    pageId: 10,
    targetId: 't10',
    url: 'https://stackoverflow.com/questions',
    title: 'Stack Overflow',
    atMs: 6_300,
  },
  {
    pageId: 11,
    targetId: 't11',
    url: 'https://arstechnica.com/',
    title: 'Ars Technica',
    atMs: 6_400,
  },
] as const

describe('TabActivityRegistry parallel-landing simulation', () => {
  let pages: Map<number, FakePageInfo>
  let session: BrowserSession
  let nowMs: number
  let registry: TabActivityRegistry

  beforeEach(() => {
    pages = new Map()
    session = makeSession(pages)
    nowMs = 1_000_000_000_000
    registry = createTabActivityRegistry({
      getSession: () => session,
      now: () => nowMs,
    })
    for (const landing of PARALLEL_LANDINGS) {
      pages.set(landing.pageId, {
        targetId: landing.targetId,
        url: landing.url,
        title: landing.title,
      })
    }
  })

  function landAll(baseline: number): void {
    for (const landing of PARALLEL_LANDINGS) {
      nowMs = baseline + landing.atMs
      registry.recordTool({
        agentId: 'a1',
        slug: 'finance-ops',
        pageId: landing.pageId,
        targetId: landing.targetId,
        toolName: 'snapshot',
      })
    }
  }

  it('all five records remain active 10 seconds after the burst starts', () => {
    const baseline = nowMs
    landAll(baseline)
    nowMs = baseline + 10_000
    const snap = registry.snapshot()
    expect(snap).toHaveLength(5)
    for (const row of snap) {
      expect(row.status).toBe('active')
    }
  })

  it('all five records remain active 29 seconds after the burst starts (just under the window)', () => {
    const baseline = nowMs
    landAll(baseline)
    // The earliest landing is at baseline + 0; advance just inside
    // the window so even it has not aged out yet.
    nowMs = baseline + (ACTIVE_WINDOW_MS - 1_000)
    const snap = registry.snapshot()
    expect(snap.every((r) => r.status === 'active')).toBe(true)
  })

  it('the earliest record idles first once the window does eventually elapse', () => {
    const baseline = nowMs
    landAll(baseline)
    // The first landing was at t=0; advance the clock so it has crossed
    // the window but the latest landing has not.
    nowMs = baseline + ACTIVE_WINDOW_MS + 1
    const snap = registry.snapshot()
    const byTarget = new Map(snap.map((r) => [r.targetId, r]))
    expect(byTarget.get('t7')?.status).toBe('idle')
    expect(byTarget.get('t11')?.status).toBe('active')
  })

  it('all records are idle once the window has fully elapsed for every landing', () => {
    const baseline = nowMs
    landAll(baseline)
    nowMs = baseline + 6_400 + ACTIVE_WINDOW_MS + 1
    const snap = registry.snapshot()
    expect(snap.every((r) => r.status === 'idle')).toBe(true)
  })

  it('the registry holds exactly five records throughout the burst', () => {
    const baseline = nowMs
    landAll(baseline)
    // Sample at several checkpoints to make sure no record was lost,
    // duplicated, or evicted prematurely.
    for (const offset of [0, 5_000, 10_000, 20_000, 29_000]) {
      nowMs = baseline + offset
      expect(registry.snapshot()).toHaveLength(5)
    }
  })

  it('a new tool call on an early tab keeps it active even after the original burst window would have elapsed', () => {
    const baseline = nowMs
    landAll(baseline)
    // 25 s after the burst, the agent goes back to the reddit tab.
    // That tab should re-enter the active set and stay there for
    // another full window.
    nowMs = baseline + 25_000
    registry.recordTool({
      agentId: 'a1',
      slug: 'finance-ops',
      pageId: 7,
      targetId: 't7',
      toolName: 'read',
    })
    nowMs = baseline + 25_000 + ACTIVE_WINDOW_MS - 1
    const snap = registry.snapshot()
    const row = snap.find((r) => r.targetId === 't7')
    expect(row?.status).toBe('active')
    expect(row?.toolCount).toBe(2)
  })
})
