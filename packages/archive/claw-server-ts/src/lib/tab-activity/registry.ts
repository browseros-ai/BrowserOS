/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * In-memory registry mapping a stable CDP target id to the live
 * activity trail for the agent currently driving that tab. The
 * cockpit's dispatch effects append to the trail after every successful
 * page-targeted tool call; the live-session query joins it to durable tab
 * ownership as optional activity metadata.
 *
 * Each record carries:
 *   - `sessionId`: the MCP session currently claiming the tab. The
 *     canonical live-session projection attributes the activity to it,
 *     and recording ingest refuses batches whose claim has drifted from it.
 *   - `tabId`: the browser tab id, kept alongside the CDP identifiers
 *     because the recorder addresses tabs by tab id — it is the join
 *     key between recorder batches and CDP-side state.
 *   - `firstToolAt`: when the currently attributed session first touched the
 *     tab; a different session claiming the same target starts a new history.
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

import type { BrowserSession } from '@browseros/browser-core/core/session'

export interface ToolEvent {
  name: string
  at: number
}

export interface TabActivityRecord {
  targetId: string
  tabId: number
  pageId: number
  url: string
  title: string
  sessionId: string
  agentId: string
  slug: string
  firstToolAt: number
  lastToolAt: number
  lastToolName: string
  toolCount: number
  recentTools: ToolEvent[]
  status: 'active' | 'idle'
}

// PR 1 shipped 5 s as a v1 guess. In practice the cockpit serialises
// `tools/call` per slug and CDP serialises per-target ops, so a
// parallel N-call burst lands in the registry over roughly the same
// duration as the old window. By the time the last write hits, the
// earliest records have already crossed the 5 s threshold and dropped
// from the running grid, so the homepage chip stepped `1 -> 3 -> 5 ->
// 4 -> 2 -> 0` during what was actually a single coherent agent
// burst. 30 s lets the rollup stay stable across the whole burst
// without changing the wire shape; tune from dogfooding.
export const ACTIVE_WINDOW_MS = 30_000
export const RECENT_TOOLS_CAP = 8

export interface RegistryDeps {
  getSession(): BrowserSession | null
  now?: () => number
}

interface RawRecord {
  targetId: string
  tabId: number
  pageId: number
  sessionId: string
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
    sessionId: string
    agentId: string
    slug: string
    tabId: number
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
        if (existing.sessionId !== input.sessionId) {
          records.set(input.targetId, {
            targetId: input.targetId,
            tabId: input.tabId,
            pageId: input.pageId,
            sessionId: input.sessionId,
            agentId: input.agentId,
            slug: input.slug,
            firstToolAt: t,
            lastToolAt: t,
            lastToolName: input.toolName,
            toolCount: 1,
            recentTools: [{ name: input.toolName, at: t }],
          })
          return
        }
        existing.agentId = input.agentId
        existing.slug = input.slug
        existing.tabId = input.tabId
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
        tabId: input.tabId,
        pageId: input.pageId,
        sessionId: input.sessionId,
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
          tabId: raw.tabId,
          pageId: raw.pageId,
          url: live.url,
          title: live.title,
          sessionId: raw.sessionId,
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
