/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * In-memory registry mapping a stable CDP target id to the live
 * activity trail for the agent currently driving that tab. The
 * cockpit's `mcp/register.ts` wrapper appends to the trail after
 * every successful `executeTool` call; the homepage polls
 * `GET /tabs/activity` to render the current view.
 *
 * Each record carries:
 *   - `firstToolAt`: when this agent first touched the tab (does not
 *     update on subsequent tool calls).
 *   - `lastToolAt` / `lastToolName`: the most recent dispatch.
 *   - `toolCount`: total dispatches against this target since the
 *     record was created.
 *   - `recentTools`: ring buffer capped at `RECENT_TOOLS_CAP` so the
 *     UI can render a short trail without unbounded memory.
 *
 * `status` is derived at read time: a record is `active` when the
 * last tool fired within `ACTIVE_WINDOW_MS`, otherwise `idle`. No
 * background timers. Records whose underlying tab has closed are
 * evicted lazily on the next `snapshot()` read; we detect that by
 * looking up `pageId` on the live `PageManager` and confirming the
 * targetId still matches (pageIds are reused after a tab closes).
 */

import type { BrowserSession } from '@browseros/server/browser/core/session'

export interface ToolEvent {
  name: string
  at: number
}

export interface TabActivityRecord {
  targetId: string
  pageId: number
  url: string
  title: string
  agentId: string
  slug: string
  firstToolAt: number
  lastToolAt: number
  lastToolName: string
  toolCount: number
  recentTools: ToolEvent[]
  status: 'active' | 'idle'
}

export const ACTIVE_WINDOW_MS = 5000
export const RECENT_TOOLS_CAP = 8

export interface RegistryDeps {
  getSession(): BrowserSession | null
  now?: () => number
}

interface RawRecord {
  targetId: string
  pageId: number
  agentId: string
  slug: string
  firstToolAt: number
  lastToolAt: number
  lastToolName: string
  toolCount: number
  recentTools: ToolEvent[]
}

export interface TabActivityRegistry {
  recordTool(input: {
    agentId: string
    slug: string
    pageId: number
    targetId: string
    toolName: string
  }): void
  snapshot(): TabActivityRecord[]
  // Test-only escape hatches; let unit tests assert eviction and
  // restore isolation without mocking BrowserSession internals. The
  // singleton lives across the whole test run, so explicit clearing
  // is the only safe way to keep `afterEach` honest.
  size(): number
  clear(): void
}

export function createTabActivityRegistry(
  deps: RegistryDeps,
): TabActivityRegistry {
  const records = new Map<string, RawRecord>()
  const now = deps.now ?? (() => Date.now())

  return {
    recordTool(input) {
      const t = now()
      const existing = records.get(input.targetId)
      if (existing) {
        // Same agent or a different agent rebinding to this target:
        // overwrite the attribution so the homepage reflects the
        // most-recent caller. firstToolAt is preserved so the card
        // can render "started 47s ago" against the original touch.
        existing.agentId = input.agentId
        existing.slug = input.slug
        existing.pageId = input.pageId
        existing.lastToolAt = t
        existing.lastToolName = input.toolName
        existing.toolCount += 1
        existing.recentTools.push({ name: input.toolName, at: t })
        if (existing.recentTools.length > RECENT_TOOLS_CAP) {
          existing.recentTools.shift()
        }
        return
      }
      records.set(input.targetId, {
        targetId: input.targetId,
        pageId: input.pageId,
        agentId: input.agentId,
        slug: input.slug,
        firstToolAt: t,
        lastToolAt: t,
        lastToolName: input.toolName,
        toolCount: 1,
        recentTools: [{ name: input.toolName, at: t }],
      })
    },
    snapshot() {
      const session = deps.getSession()
      if (!session) return []
      const out: TabActivityRecord[] = []
      const t = now()
      for (const [targetId, raw] of records) {
        const live = session.pages.getInfo(raw.pageId)
        // PageManager reuses pageId after a tab closes; the targetId
        // is the stable identity. If they no longer match, the
        // original tab is gone (the pageId may now belong to a
        // different tab).
        if (!live || live.targetId !== targetId) {
          records.delete(targetId)
          continue
        }
        out.push({
          targetId: raw.targetId,
          pageId: raw.pageId,
          url: live.url,
          title: live.title,
          agentId: raw.agentId,
          slug: raw.slug,
          firstToolAt: raw.firstToolAt,
          lastToolAt: raw.lastToolAt,
          lastToolName: raw.lastToolName,
          toolCount: raw.toolCount,
          // Hand the consumer its own copy; the registry keeps mutating
          // its private buffer.
          recentTools: raw.recentTools.slice(),
          status: t - raw.lastToolAt < ACTIVE_WINDOW_MS ? 'active' : 'idle',
        })
      }
      return out.sort((a, b) => b.lastToolAt - a.lastToolAt)
    },
    size() {
      return records.size
    },
    clear() {
      records.clear()
    },
  }
}
