import { beforeEach, describe, expect, it } from 'bun:test'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import {
  ACTIVE_WINDOW_MS,
  createTabActivityRegistry,
  RECENT_TOOLS_CAP,
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

describe('TabActivityRegistry', () => {
  let pages: Map<number, FakePageInfo>
  let session: BrowserSession
  let nowMs: number
  let registry: TabActivityRegistry

  beforeEach(() => {
    pages = new Map()
    session = makeSession(pages)
    nowMs = 1_000_000
    registry = createTabActivityRegistry({
      getSession: () => session,
      now: () => nowMs,
    })
  })

  it('records a tool dispatch and surfaces it via snapshot', () => {
    pages.set(1, { targetId: 't1', url: 'https://example.com/', title: 'Ex' })
    registry.recordTool({
      agentId: 'a1',
      slug: 'finance-ops',
      pageId: 1,
      targetId: 't1',
      toolName: 'navigate',
    })
    const snap = registry.snapshot()
    expect(snap).toHaveLength(1)
    expect(snap[0]).toMatchObject({
      targetId: 't1',
      pageId: 1,
      url: 'https://example.com/',
      title: 'Ex',
      agentId: 'a1',
      slug: 'finance-ops',
      firstToolAt: nowMs,
      lastToolAt: nowMs,
      lastToolName: 'navigate',
      toolCount: 1,
      status: 'active',
    })
    expect(snap[0].recentTools).toEqual([{ name: 'navigate', at: nowMs }])
  })

  it('updates an existing record rather than appending a duplicate', () => {
    pages.set(1, { targetId: 't1', url: 'https://example.com/', title: 'Ex' })
    registry.recordTool({
      agentId: 'a1',
      slug: 'finance-ops',
      pageId: 1,
      targetId: 't1',
      toolName: 'navigate',
    })
    nowMs += 1000
    registry.recordTool({
      agentId: 'a1',
      slug: 'finance-ops',
      pageId: 1,
      targetId: 't1',
      toolName: 'snapshot',
    })
    const snap = registry.snapshot()
    expect(snap).toHaveLength(1)
    expect(snap[0].lastToolName).toBe('snapshot')
    expect(snap[0].lastToolAt).toBe(1_001_000)
    // firstToolAt is set on the first write and never moves.
    expect(snap[0].firstToolAt).toBe(1_000_000)
    expect(snap[0].toolCount).toBe(2)
    expect(snap[0].recentTools).toEqual([
      { name: 'navigate', at: 1_000_000 },
      { name: 'snapshot', at: 1_001_000 },
    ])
  })

  it('caps recentTools at RECENT_TOOLS_CAP and drops the oldest entry', () => {
    pages.set(1, { targetId: 't1', url: 'https://example.com/', title: 'Ex' })
    const tools = [
      'navigate',
      'snapshot',
      'read',
      'grep',
      'screenshot',
      'act',
      'diff',
      'read',
      'snapshot',
      'act',
    ]
    for (let i = 0; i < tools.length; i++) {
      nowMs = 1_000_000 + i * 100
      registry.recordTool({
        agentId: 'a1',
        slug: 'finance-ops',
        pageId: 1,
        targetId: 't1',
        toolName: tools[i],
      })
    }
    const snap = registry.snapshot()
    expect(snap).toHaveLength(1)
    expect(snap[0].toolCount).toBe(tools.length)
    expect(snap[0].recentTools).toHaveLength(RECENT_TOOLS_CAP)
    // The two oldest (navigate, snapshot) should have dropped off; the
    // newest (act) is the tail.
    expect(snap[0].recentTools[0].name).toBe('read')
    expect(snap[0].recentTools[snap[0].recentTools.length - 1].name).toBe('act')
  })

  it('hands consumers a defensive copy of recentTools', () => {
    pages.set(1, { targetId: 't1', url: 'https://example.com/', title: 'Ex' })
    registry.recordTool({
      agentId: 'a1',
      slug: 'finance-ops',
      pageId: 1,
      targetId: 't1',
      toolName: 'navigate',
    })
    const first = registry.snapshot()[0].recentTools
    first.push({ name: 'mutated', at: 0 })
    const second = registry.snapshot()[0].recentTools
    expect(second).toHaveLength(1)
    expect(second[0].name).toBe('navigate')
  })

  it('overwrites attribution but preserves firstToolAt when a different agent claims the same tab', () => {
    pages.set(1, { targetId: 't1', url: 'https://example.com/', title: 'Ex' })
    registry.recordTool({
      agentId: 'a1',
      slug: 'finance',
      pageId: 1,
      targetId: 't1',
      toolName: 'navigate',
    })
    nowMs += 500
    registry.recordTool({
      agentId: 'a2',
      slug: 'travel',
      pageId: 1,
      targetId: 't1',
      toolName: 'snapshot',
    })
    const snap = registry.snapshot()
    expect(snap).toHaveLength(1)
    expect(snap[0].agentId).toBe('a2')
    expect(snap[0].slug).toBe('travel')
    expect(snap[0].firstToolAt).toBe(1_000_000)
    expect(snap[0].toolCount).toBe(2)
  })

  it('marks records active within the window and idle outside it', () => {
    pages.set(1, { targetId: 't1', url: 'https://example.com/', title: 'Ex' })
    registry.recordTool({
      agentId: 'a1',
      slug: 'finance-ops',
      pageId: 1,
      targetId: 't1',
      toolName: 'navigate',
    })
    expect(registry.snapshot()[0].status).toBe('active')
    nowMs += ACTIVE_WINDOW_MS - 1
    expect(registry.snapshot()[0].status).toBe('active')
    nowMs += 2
    expect(registry.snapshot()[0].status).toBe('idle')
  })

  it('ACTIVE_WINDOW_MS is 30 seconds (regression guard)', () => {
    // Reverting this to a smaller value would silently re-introduce
    // the homepage flicker observed when a single agent fires a
    // parallel burst of tool calls across several tabs. Tune
    // intentionally; do not edit blindly.
    expect(ACTIVE_WINDOW_MS).toBe(30_000)
  })

  it('evicts records whose pageId no longer maps to the original targetId', () => {
    pages.set(1, { targetId: 't1', url: 'https://example.com/', title: 'Ex' })
    registry.recordTool({
      agentId: 'a1',
      slug: 'finance-ops',
      pageId: 1,
      targetId: 't1',
      toolName: 'navigate',
    })
    expect(registry.size()).toBe(1)
    // The tab closes, pageId 1 is reused by a fresh tab with a new targetId.
    pages.set(1, { targetId: 't2-different', url: 'about:blank', title: '' })
    expect(registry.snapshot()).toHaveLength(0)
    expect(registry.size()).toBe(0)
  })

  it('evicts records whose pageId no longer exists at all', () => {
    pages.set(1, { targetId: 't1', url: 'https://example.com/', title: 'Ex' })
    registry.recordTool({
      agentId: 'a1',
      slug: 'finance-ops',
      pageId: 1,
      targetId: 't1',
      toolName: 'navigate',
    })
    pages.delete(1)
    expect(registry.snapshot()).toHaveLength(0)
    expect(registry.size()).toBe(0)
  })

  it('returns an empty snapshot when no session is connected', () => {
    pages.set(1, { targetId: 't1', url: 'https://example.com/', title: 'Ex' })
    registry.recordTool({
      agentId: 'a1',
      slug: 'finance-ops',
      pageId: 1,
      targetId: 't1',
      toolName: 'navigate',
    })
    const detached = createTabActivityRegistry({
      getSession: () => null,
      now: () => nowMs,
    })
    detached.recordTool({
      agentId: 'a1',
      slug: 'finance-ops',
      pageId: 1,
      targetId: 't1',
      toolName: 'navigate',
    })
    expect(detached.snapshot()).toEqual([])
  })

  it('keeps separate records per target id', () => {
    pages.set(1, { targetId: 't1', url: 'https://a.com/', title: 'A' })
    pages.set(2, { targetId: 't2', url: 'https://b.com/', title: 'B' })
    registry.recordTool({
      agentId: 'a1',
      slug: 'finance',
      pageId: 1,
      targetId: 't1',
      toolName: 'navigate',
    })
    nowMs += 100
    registry.recordTool({
      agentId: 'a2',
      slug: 'travel',
      pageId: 2,
      targetId: 't2',
      toolName: 'read',
    })
    const snap = registry.snapshot()
    expect(snap).toHaveLength(2)
    expect(snap.map((r) => r.targetId)).toEqual(['t2', 't1'])
  })

  it('sorts the snapshot by lastToolAt descending', () => {
    pages.set(1, { targetId: 't1', url: 'https://a.com/', title: 'A' })
    pages.set(2, { targetId: 't2', url: 'https://b.com/', title: 'B' })
    registry.recordTool({
      agentId: 'a1',
      slug: 'finance',
      pageId: 1,
      targetId: 't1',
      toolName: 'navigate',
    })
    nowMs += 100
    registry.recordTool({
      agentId: 'a2',
      slug: 'travel',
      pageId: 2,
      targetId: 't2',
      toolName: 'read',
    })
    nowMs += 100
    registry.recordTool({
      agentId: 'a1',
      slug: 'finance',
      pageId: 1,
      targetId: 't1',
      toolName: 'snapshot',
    })
    const snap = registry.snapshot()
    expect(snap.map((r) => r.targetId)).toEqual(['t1', 't2'])
  })

  // Superseded by "overwrites attribution but preserves firstToolAt"
  // above; that case also checks the firstToolAt + toolCount semantics
  // we want from PR 2.
})
