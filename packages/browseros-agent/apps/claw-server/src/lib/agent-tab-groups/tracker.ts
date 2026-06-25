/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * In-memory map: agentId to the cockpit-owned tab group container the
 * agent's tabs land in. The orchestrator (`services/tab-group-ops`)
 * writes here on a successful `tabs new`; the session-close path
 * decrements the ref count and reads back the record so it knows
 * which group to close in BrowserOS.
 *
 * Ref-counted by MCP session: two parallel sessions whose clients
 * identify with the same name share one group; the group only
 * closes when the last session for that agentId ends.
 */

import { colorForSlug, type TabGroupColor } from './group-color'

export interface TabGroupRecord {
  agentId: string
  slug: string
  /** Set after the cockpit calls `tab_groups create` for the first time. */
  groupId: string | null
  /** Set once we know which window the group lives in (returned alongside groupId). */
  windowId: number | null
  /** Pages the cockpit has added to the group so far. */
  pageIds: Set<number>
  color: TabGroupColor
  title: string
  firstOpenedAt: number
  /** Live MCP sessions whose identity resolves to this agentId. */
  refCount: number
}

export interface RecordOpenInput {
  agentId: string
  slug: string
  pageId: number
}

export interface RememberGroupInput {
  agentId: string
  groupId: string
  windowId?: number | null
}

export interface TabGroupTracker {
  /**
   * Idempotent. Creates a record on first call, or adds `pageId` to
   * the existing record's pageIds. Does NOT increment refCount; that
   * happens at session-open time via `incrementSession`.
   */
  recordOpen(input: RecordOpenInput): TabGroupRecord
  /** Called after `tab_groups create` returns so the cockpit can reuse the groupId for subsequent pages. */
  rememberGroup(input: RememberGroupInput): void
  /** Called once per MCP session-init keyed by agentId. */
  incrementSession(agentId: string): void
  /**
   * Called once per session close. Returns the record IF this was the
   * last session for that agentId (caller should now close the group
   * in BrowserOS); otherwise returns null.
   */
  decrementSession(agentId: string): TabGroupRecord | null
  /** Called when the cockpit detects the on-disk group is gone or stale. */
  forgetGroup(agentId: string): void
  list(): readonly TabGroupRecord[]
  getByAgentId(agentId: string): TabGroupRecord | null
  size(): number
  reset(): void
}

export interface TabGroupTrackerDeps {
  now?: () => number
}

export function createTabGroupTracker(
  deps: TabGroupTrackerDeps = {},
): TabGroupTracker {
  const records = new Map<string, TabGroupRecord>()
  const now = deps.now ?? (() => Date.now())

  return {
    recordOpen({ agentId, slug, pageId }) {
      const existing = records.get(agentId)
      if (existing) {
        existing.pageIds.add(pageId)
        return existing
      }
      const record: TabGroupRecord = {
        agentId,
        slug,
        groupId: null,
        windowId: null,
        pageIds: new Set([pageId]),
        color: colorForSlug(slug),
        title: slug,
        firstOpenedAt: now(),
        refCount: 0,
      }
      records.set(agentId, record)
      return record
    },
    rememberGroup({ agentId, groupId, windowId }) {
      const record = records.get(agentId)
      if (!record) return
      record.groupId = groupId
      if (typeof windowId === 'number') record.windowId = windowId
    },
    incrementSession(agentId) {
      const record = records.get(agentId)
      if (record) {
        record.refCount += 1
        return
      }
      // No tab opened yet for this agent; pre-seed a refCount-only
      // record so the close path can decrement cleanly.
      records.set(agentId, {
        agentId,
        slug: agentId,
        groupId: null,
        windowId: null,
        pageIds: new Set(),
        color: colorForSlug(agentId),
        title: agentId,
        firstOpenedAt: now(),
        refCount: 1,
      })
    },
    decrementSession(agentId) {
      const record = records.get(agentId)
      if (!record) return null
      record.refCount -= 1
      if (record.refCount <= 0) {
        records.delete(agentId)
        return record
      }
      return null
    },
    forgetGroup(agentId) {
      const record = records.get(agentId)
      if (!record) return
      record.groupId = null
      record.windowId = null
      record.pageIds.clear()
    },
    list() {
      return Array.from(records.values())
    },
    getByAgentId(agentId) {
      return records.get(agentId) ?? null
    },
    size() {
      return records.size
    },
    reset() {
      records.clear()
    },
  }
}
