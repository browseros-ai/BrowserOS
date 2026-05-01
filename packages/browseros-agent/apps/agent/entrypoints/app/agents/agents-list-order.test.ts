import { describe, expect, it } from 'bun:test'
import type { HarnessAgent } from './agent-harness-types'
import {
  compareAgentsByPinThenRecency,
  orderAgentsByPinThenRecency,
} from './agents-list-order'

function makeAgent(input: {
  id: string
  pinned?: boolean
  lastUsedAt?: number | null
}): HarnessAgent {
  return {
    id: input.id,
    name: input.id,
    adapter: 'codex',
    permissionMode: 'approve-all',
    sessionKey: 'session',
    createdAt: 0,
    updatedAt: 0,
    pinned: input.pinned,
    lastUsedAt: input.lastUsedAt,
  }
}

describe('orderAgentsByPinThenRecency', () => {
  it('floats pinned agents to the top regardless of recency', () => {
    const result = orderAgentsByPinThenRecency([
      makeAgent({ id: 'a', pinned: false, lastUsedAt: 1_000 }),
      makeAgent({ id: 'b', pinned: true, lastUsedAt: 100 }),
      makeAgent({ id: 'c', pinned: false, lastUsedAt: 500 }),
    ])
    expect(result.map((entry) => entry.id)).toEqual(['b', 'a', 'c'])
  })

  it('sorts by lastUsedAt desc within each pin group', () => {
    const result = orderAgentsByPinThenRecency([
      makeAgent({ id: 'older-pin', pinned: true, lastUsedAt: 100 }),
      makeAgent({ id: 'newer-pin', pinned: true, lastUsedAt: 200 }),
      makeAgent({ id: 'older', pinned: false, lastUsedAt: 50 }),
      makeAgent({ id: 'newer', pinned: false, lastUsedAt: 80 }),
    ])
    expect(result.map((entry) => entry.id)).toEqual([
      'newer-pin',
      'older-pin',
      'newer',
      'older',
    ])
  })

  it('seed-pins the gateway main agent above other never-used agents', () => {
    const result = orderAgentsByPinThenRecency([
      makeAgent({ id: 'aaa', pinned: false, lastUsedAt: null }),
      makeAgent({ id: 'main', pinned: false, lastUsedAt: null }),
      makeAgent({ id: 'zzz', pinned: false, lastUsedAt: null }),
    ])
    expect(result.map((entry) => entry.id)).toEqual(['main', 'aaa', 'zzz'])
  })

  it('drops the main seed-pin once the agent has been used', () => {
    const result = orderAgentsByPinThenRecency([
      makeAgent({ id: 'aaa', pinned: false, lastUsedAt: 999 }),
      makeAgent({ id: 'main', pinned: false, lastUsedAt: 1 }),
    ])
    expect(result.map((entry) => entry.id)).toEqual(['aaa', 'main'])
  })

  it('puts never-used agents below recently-used ones', () => {
    const result = orderAgentsByPinThenRecency([
      makeAgent({ id: 'fresh', pinned: false, lastUsedAt: null }),
      makeAgent({ id: 'used', pinned: false, lastUsedAt: 100 }),
    ])
    expect(result.map((entry) => entry.id)).toEqual(['used', 'fresh'])
  })

  it('id-stable tiebreaks two agents with identical lastUsedAt', () => {
    const result = orderAgentsByPinThenRecency([
      makeAgent({ id: 'b', pinned: false, lastUsedAt: 100 }),
      makeAgent({ id: 'a', pinned: false, lastUsedAt: 100 }),
    ])
    expect(result.map((entry) => entry.id)).toEqual(['a', 'b'])
  })
})

describe('compareAgentsByPinThenRecency', () => {
  it('produces the same order as the harness-shape helper', () => {
    const items = [
      { id: 'older', pinned: false, lastUsedAt: 50 },
      { id: 'newer', pinned: false, lastUsedAt: 80 },
      { id: 'pinned', pinned: true, lastUsedAt: 1 },
    ]
    const sorted = [...items].sort(compareAgentsByPinThenRecency)
    expect(sorted.map((item) => item.id)).toEqual(['pinned', 'newer', 'older'])
  })

  it('seeds the main agent above other never-used rows', () => {
    const items = [
      { id: 'zzz', pinned: false, lastUsedAt: null },
      { id: 'main', pinned: false, lastUsedAt: null },
    ]
    const sorted = [...items].sort(compareAgentsByPinThenRecency)
    expect(sorted.map((item) => item.id)).toEqual(['main', 'zzz'])
  })
})
