/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure unit tests for the per-agent tabs ledger. The ledger backs
 * the isolation guarantees in register.ts (cross-agent page guard +
 * tabs list filter) and is dropped from cleanupSessionState so a
 * new session for the same agentId starts empty.
 */

import { describe, expect, it } from 'bun:test'
import { createAgentTabsRegistry } from '../../src/lib/agent-tabs'

describe('agentTabs registry', () => {
  it('markOpened + ownedBy roundtrip returns the recorded page ids', () => {
    const r = createAgentTabsRegistry()
    r.markOpened('claude-code', 1)
    r.markOpened('claude-code', 3)
    r.markOpened('claude-code', 7)
    expect([...r.ownedBy('claude-code')].sort((a, b) => a - b)).toEqual([
      1, 3, 7,
    ])
  })

  it('ownedBy on an unknown agent returns an empty set (never throws)', () => {
    const r = createAgentTabsRegistry()
    const owned = r.ownedBy('never-seen')
    expect(owned.size).toBe(0)
    expect(owned.has(1)).toBe(false)
  })

  it('markClosed removes a single page id', () => {
    const r = createAgentTabsRegistry()
    r.markOpened('claude-code', 1)
    r.markOpened('claude-code', 2)
    r.markClosed('claude-code', 1)
    expect([...r.ownedBy('claude-code')]).toEqual([2])
  })

  it('markClosed of the last page id drops the whole agent entry', () => {
    const r = createAgentTabsRegistry()
    r.markOpened('claude-code', 1)
    expect(r.size()).toBe(1)
    r.markClosed('claude-code', 1)
    expect(r.size()).toBe(0)
    expect(r.ownedBy('claude-code').size).toBe(0)
  })

  it('markClosed for an unknown pair is a no-op', () => {
    const r = createAgentTabsRegistry()
    r.markOpened('claude-code', 1)
    r.markClosed('claude-code', 99) // was never opened
    r.markClosed('cursor', 1) // unknown agent
    expect([...r.ownedBy('claude-code')]).toEqual([1])
    expect(r.size()).toBe(1)
  })

  it('forgetAgent drops all pages for that agent only', () => {
    const r = createAgentTabsRegistry()
    r.markOpened('claude-code', 1)
    r.markOpened('claude-code', 2)
    r.markOpened('cursor', 5)
    r.forgetAgent('claude-code')
    expect(r.ownedBy('claude-code').size).toBe(0)
    expect([...r.ownedBy('cursor')]).toEqual([5])
    expect(r.size()).toBe(1)
  })

  it("two agents are isolated: one agent cannot see the other's pages", () => {
    const r = createAgentTabsRegistry()
    r.markOpened('claude-code', 1)
    r.markOpened('claude-code', 2)
    r.markOpened('cursor', 10)
    expect([...r.ownedBy('claude-code')].sort((a, b) => a - b)).toEqual([1, 2])
    expect([...r.ownedBy('cursor')]).toEqual([10])
    expect(r.ownedBy('claude-code').has(10)).toBe(false)
    expect(r.ownedBy('cursor').has(1)).toBe(false)
  })

  it('clear resets the whole registry (test-only escape hatch)', () => {
    const r = createAgentTabsRegistry()
    r.markOpened('a', 1)
    r.markOpened('b', 2)
    r.clear()
    expect(r.size()).toBe(0)
    expect(r.ownedBy('a').size).toBe(0)
  })
})
