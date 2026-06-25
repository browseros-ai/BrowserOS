import { describe, expect, it } from 'bun:test'
import { createTabGroupTracker } from '../../../src/lib/agent-tab-groups/tracker'

function setup(nowMs = 1_000_000) {
  return createTabGroupTracker({ now: () => nowMs })
}

describe('createTabGroupTracker', () => {
  it('creates a record on first recordOpen and adds pages on subsequent calls', () => {
    const t = setup()
    const a = t.recordOpen({
      agentId: 'claude-code',
      slug: 'claude-code',
      pageId: 1,
    })
    expect(a.groupId).toBeNull()
    expect(Array.from(a.pageIds)).toEqual([1])
    const b = t.recordOpen({
      agentId: 'claude-code',
      slug: 'claude-code',
      pageId: 2,
    })
    expect(b).toBe(a)
    expect(Array.from(a.pageIds).sort()).toEqual([1, 2])
  })

  it('keeps records per agentId independent', () => {
    const t = setup()
    t.recordOpen({ agentId: 'claude-code', slug: 'claude-code', pageId: 1 })
    t.recordOpen({ agentId: 'cursor', slug: 'cursor', pageId: 2 })
    expect(t.size()).toBe(2)
    expect(t.getByAgentId('claude-code')?.slug).toBe('claude-code')
    expect(t.getByAgentId('cursor')?.slug).toBe('cursor')
  })

  it('rememberGroup stamps groupId and windowId', () => {
    const t = setup()
    t.recordOpen({ agentId: 'a', slug: 'a', pageId: 1 })
    t.rememberGroup({ agentId: 'a', groupId: 'G1', windowId: 99 })
    const r = t.getByAgentId('a')
    expect(r?.groupId).toBe('G1')
    expect(r?.windowId).toBe(99)
  })

  it('rememberGroup is a no-op when no record exists', () => {
    const t = setup()
    t.rememberGroup({ agentId: 'ghost', groupId: 'G' })
    expect(t.size()).toBe(0)
  })

  it('refCount starts at 0; incrementSession adds, decrementSession removes', () => {
    const t = setup()
    t.recordOpen({ agentId: 'a', slug: 'a', pageId: 1 })
    expect(t.getByAgentId('a')?.refCount).toBe(0)
    t.incrementSession('a')
    expect(t.getByAgentId('a')?.refCount).toBe(1)
    t.incrementSession('a')
    expect(t.getByAgentId('a')?.refCount).toBe(2)
  })

  it('decrementSession returns null while sessions remain, and the record on the final drop', () => {
    const t = setup()
    t.recordOpen({ agentId: 'a', slug: 'a', pageId: 1 })
    t.incrementSession('a')
    t.incrementSession('a')
    expect(t.decrementSession('a')).toBeNull()
    expect(t.size()).toBe(1)
    const final = t.decrementSession('a')
    expect(final).not.toBeNull()
    expect(final?.agentId).toBe('a')
    expect(t.size()).toBe(0)
  })

  it('incrementSession before recordOpen pre-seeds an empty record', () => {
    const t = setup()
    t.incrementSession('claude-code')
    const r = t.getByAgentId('claude-code')
    expect(r?.refCount).toBe(1)
    expect(r?.pageIds.size).toBe(0)
    expect(r?.groupId).toBeNull()
  })

  it('decrementSession on unknown agentId returns null cleanly', () => {
    const t = setup()
    expect(t.decrementSession('nope')).toBeNull()
  })

  it('forgetGroup clears the groupId so the next tabs new triggers a fresh create', () => {
    const t = setup()
    t.recordOpen({ agentId: 'a', slug: 'a', pageId: 1 })
    t.rememberGroup({ agentId: 'a', groupId: 'G1', windowId: 1 })
    t.forgetGroup('a')
    const r = t.getByAgentId('a')
    expect(r?.groupId).toBeNull()
    expect(r?.windowId).toBeNull()
    expect(r?.pageIds.size).toBe(0)
  })

  it('reset empties the entire map', () => {
    const t = setup()
    t.recordOpen({ agentId: 'a', slug: 'a', pageId: 1 })
    t.recordOpen({ agentId: 'b', slug: 'b', pageId: 2 })
    t.reset()
    expect(t.size()).toBe(0)
  })
})
