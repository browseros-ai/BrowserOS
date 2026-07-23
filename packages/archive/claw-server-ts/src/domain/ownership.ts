/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

/** Ownership uses a per-session key; prune only removes claims for pages no longer live in the browser. */

import type { TabGroupColor } from '../lib/agent-tab-groups'
import type { AgentKey } from './agent-key'

export interface GroupRef {
  id: string
  windowId: number | null
  color: TabGroupColor
  title: string
  collapsed: boolean
}

export type GroupPatch = Partial<Pick<GroupRef, 'title' | 'collapsed'>>

export interface OwnershipStore {
  claimPage(key: AgentKey, pageId: number): void
  releasePage(key: AgentKey, pageId: number): void
  pagesOf(key: AgentKey): ReadonlySet<number>
  ownerOf(pageId: number): AgentKey | null
  setGroup(key: AgentKey, ref: GroupRef): void
  groupOf(key: AgentKey): GroupRef | null
  clearGroup(key: AgentKey): void
  updateGroup(key: AgentKey, patch: GroupPatch): void
  prune(livePageIds: ReadonlySet<number>): void
  forget(key: AgentKey): void
  size(): number
  clear(): void
}

interface OwnershipState {
  pages: Set<number>
  group: GroupRef | null
}

const EMPTY_PAGES: ReadonlySet<number> = new Set<number>()

/** Creates an isolated durable page and tab-group ownership store. */
export function createOwnershipStore(): OwnershipStore {
  const records = new Map<AgentKey, OwnershipState>()
  const owners = new Map<number, AgentKey>()

  function stateFor(key: AgentKey): OwnershipState {
    let state = records.get(key)
    if (!state) {
      state = { pages: new Set(), group: null }
      records.set(key, state)
    }
    return state
  }

  return {
    claimPage(key, pageId) {
      stateFor(key).pages.add(pageId)
      owners.set(pageId, key)
    },
    releasePage(key, pageId) {
      records.get(key)?.pages.delete(pageId)
      if (owners.get(pageId) === key) owners.delete(pageId)
    },
    pagesOf(key) {
      return records.get(key)?.pages ?? EMPTY_PAGES
    },
    ownerOf(pageId) {
      return owners.get(pageId) ?? null
    },
    setGroup(key, ref) {
      stateFor(key).group = ref
    },
    groupOf(key) {
      return records.get(key)?.group ?? null
    },
    clearGroup(key) {
      const state = records.get(key)
      if (state) state.group = null
    },
    updateGroup(key, patch) {
      const group = records.get(key)?.group
      if (group) Object.assign(group, patch)
    },
    prune(livePageIds) {
      for (const state of records.values()) {
        for (const pageId of state.pages) {
          if (!livePageIds.has(pageId)) state.pages.delete(pageId)
        }
      }
      for (const pageId of owners.keys()) {
        if (!livePageIds.has(pageId)) owners.delete(pageId)
      }
    },
    forget(key) {
      const state = records.get(key)
      if (!state) return
      for (const pageId of state.pages) {
        if (owners.get(pageId) === key) owners.delete(pageId)
      }
      records.delete(key)
    },
    size() {
      return records.size
    },
    clear() {
      records.clear()
      owners.clear()
    },
  }
}

export const ownershipStore = createOwnershipStore()
