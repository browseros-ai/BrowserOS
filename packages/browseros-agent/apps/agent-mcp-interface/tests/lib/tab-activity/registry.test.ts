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
      lastToolName: 'navigate',
      status: 'active',
    })
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

  it('last write wins on agent attribution when two agents touch the same tab', () => {
    pages.set(1, { targetId: 't1', url: 'https://a.com/', title: 'A' })
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
      pageId: 1,
      targetId: 't1',
      toolName: 'snapshot',
    })
    const snap = registry.snapshot()
    expect(snap).toHaveLength(1)
    expect(snap[0].agentId).toBe('a2')
    expect(snap[0].slug).toBe('travel')
  })
})
