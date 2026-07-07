/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Unit tests for the `resolveOwner(pageId)` reverse lookup added to
 * agent-tabs. Ensures the second index stays coherent with the
 * per-agent Set across every mutation path.
 */

import { describe, expect, test } from 'bun:test'
import { createAgentTabsRegistry } from '../../src/lib/agent-tabs/agent-tabs'

describe('agentTabs.resolveOwner', () => {
  test('unknown page id returns null', () => {
    const reg = createAgentTabsRegistry()
    expect(reg.resolveOwner(42)).toBeNull()
  })

  test('returns the agentId that opened the page', () => {
    const reg = createAgentTabsRegistry()
    reg.markOpened('agent-a', 7)
    expect(reg.resolveOwner(7)).toBe('agent-a')
  })

  test('foreign owner is returned to a non-owner caller', () => {
    const reg = createAgentTabsRegistry()
    reg.markOpened('agent-a', 7)
    // A different agent asking about the same page still gets the
    // owner id; the classifier is what interprets it as `other-agent`.
    expect(reg.resolveOwner(7)).toBe('agent-a')
  })

  test('markClosed drops the owner entry', () => {
    const reg = createAgentTabsRegistry()
    reg.markOpened('agent-a', 7)
    reg.markClosed('agent-a', 7)
    expect(reg.resolveOwner(7)).toBeNull()
  })

  test('forgetAgent clears ownership for every page the agent owned', () => {
    const reg = createAgentTabsRegistry()
    reg.markOpened('agent-a', 1)
    reg.markOpened('agent-a', 2)
    reg.markOpened('agent-b', 3)
    reg.forgetAgent('agent-a')
    expect(reg.resolveOwner(1)).toBeNull()
    expect(reg.resolveOwner(2)).toBeNull()
    expect(reg.resolveOwner(3)).toBe('agent-b')
  })

  test('two-agent collision preserves the LAST writer as owner', () => {
    // A defensive edge case: two agents claim the same pageId. The
    // library-level invariant is that `tabs new` allocates a fresh
    // page id per open, so this should not happen in production;
    // if it ever does, ownership goes to whoever wrote last.
    const reg = createAgentTabsRegistry()
    reg.markOpened('agent-a', 7)
    reg.markOpened('agent-b', 7)
    expect(reg.resolveOwner(7)).toBe('agent-b')
    // A close from the original writer must NOT clear the owner
    // slot; only the recorded owner clears its entry.
    reg.markClosed('agent-a', 7)
    expect(reg.resolveOwner(7)).toBe('agent-b')
    reg.markClosed('agent-b', 7)
    expect(reg.resolveOwner(7)).toBeNull()
  })

  test('clear resets the owner index', () => {
    const reg = createAgentTabsRegistry()
    reg.markOpened('agent-a', 1)
    reg.clear()
    expect(reg.resolveOwner(1)).toBeNull()
  })
})
