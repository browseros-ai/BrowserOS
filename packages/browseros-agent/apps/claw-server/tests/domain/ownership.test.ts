/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { agentKeyFromSlug } from '../../src/domain/agent-key'
import { createOwnershipStore, type GroupRef } from '../../src/domain/ownership'

const codex = agentKeyFromSlug('codex')
const claude = agentKeyFromSlug('claude')

function group(overrides: Partial<GroupRef> = {}): GroupRef {
  return {
    id: 'group-1',
    windowId: 10,
    color: 'purple',
    title: 'Codex',
    titleExplicit: false,
    collapsed: false,
    ...overrides,
  }
}

describe('OwnershipStore', () => {
  it('keeps forward and reverse page ownership coherent', () => {
    const store = createOwnershipStore()

    store.claimPage(codex, 1)
    expect(store.pagesOf(codex)).toEqual(new Set([1]))
    expect(store.ownerOf(1)).toBe(codex)

    store.releasePage(codex, 1)
    expect(store.pagesOf(codex)).toEqual(new Set())
    expect(store.ownerOf(1)).toBeNull()
  })

  it('does not clear the reverse owner when a non-owner releases', () => {
    const store = createOwnershipStore()
    store.claimPage(codex, 1)

    store.releasePage(claude, 1)

    expect(store.ownerOf(1)).toBe(codex)
    expect(store.pagesOf(codex)).toEqual(new Set([1]))
  })

  it('lets the last claim win without an earlier release clobbering it', () => {
    const store = createOwnershipStore()
    store.claimPage(codex, 1)
    store.claimPage(claude, 1)

    expect(store.ownerOf(1)).toBe(claude)
    expect(store.pagesOf(codex)).toEqual(new Set([1]))
    expect(store.pagesOf(claude)).toEqual(new Set([1]))

    store.releasePage(codex, 1)
    expect(store.ownerOf(1)).toBe(claude)
    expect(store.pagesOf(codex)).toEqual(new Set())
  })

  it('returns a shared empty set for unknown keys', () => {
    const store = createOwnershipStore()
    const first = store.pagesOf(codex)
    const second = store.pagesOf(claude)

    expect(first).toBe(second)
    expect(first).toEqual(new Set())
  })

  it('prunes dead claims from both indices and keeps live claims', () => {
    const store = createOwnershipStore()
    store.claimPage(codex, 1)
    store.claimPage(codex, 2)

    store.prune(new Set([2]))

    expect(store.pagesOf(codex)).toEqual(new Set([2]))
    expect(store.ownerOf(1)).toBeNull()
    expect(store.ownerOf(2)).toBe(codex)
  })

  it('keeps an empty entry and its group after pruning its last page', () => {
    const store = createOwnershipStore()
    const ref = group()
    store.claimPage(codex, 1)
    store.setGroup(codex, ref)

    store.prune(new Set())

    expect(store.size()).toBe(1)
    expect(store.pagesOf(codex)).toEqual(new Set())
    expect(store.ownerOf(1)).toBeNull()
    expect(store.groupOf(codex)).toBe(ref)
  })

  it('sets, patches, and clears a group without losing pages', () => {
    const store = createOwnershipStore()
    const ref = group()
    store.claimPage(codex, 1)
    store.setGroup(codex, ref)

    expect(store.groupOf(codex)).toBe(ref)

    store.updateGroup(codex, {
      title: 'Research',
      titleExplicit: true,
      collapsed: true,
    })
    expect(store.groupOf(codex)).toEqual(
      group({ title: 'Research', titleExplicit: true, collapsed: true }),
    )

    store.clearGroup(codex)
    expect(store.groupOf(codex)).toBeNull()
    expect(store.pagesOf(codex)).toEqual(new Set([1]))
  })

  it('does not create an entry when patching a missing group', () => {
    const store = createOwnershipStore()

    store.updateGroup(codex, { collapsed: true })

    expect(store.groupOf(codex)).toBeNull()
    expect(store.size()).toBe(0)
  })

  it('forgets an entry and its reverse page claims', () => {
    const store = createOwnershipStore()
    store.claimPage(codex, 1)
    store.claimPage(codex, 2)
    store.setGroup(codex, group())

    store.forget(codex)

    expect(store.size()).toBe(0)
    expect(store.pagesOf(codex)).toEqual(new Set())
    expect(store.ownerOf(1)).toBeNull()
    expect(store.ownerOf(2)).toBeNull()
    expect(store.groupOf(codex)).toBeNull()
  })

  it('does not clear a newer reverse owner when forgetting an old claimant', () => {
    const store = createOwnershipStore()
    store.claimPage(codex, 1)
    store.claimPage(claude, 1)

    store.forget(codex)

    expect(store.ownerOf(1)).toBe(claude)
    expect(store.pagesOf(claude)).toEqual(new Set([1]))
  })

  it('clears all state', () => {
    const store = createOwnershipStore()
    store.claimPage(codex, 1)
    store.setGroup(codex, group())

    store.clear()

    expect(store.size()).toBe(0)
    expect(store.pagesOf(codex)).toEqual(new Set())
    expect(store.ownerOf(1)).toBeNull()
    expect(store.groupOf(codex)).toBeNull()
  })
})
