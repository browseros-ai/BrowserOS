/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * In-memory registry mapping a stable CDP target id to the most
 * recent agent-tool dispatch that touched it. The cockpit's
 * `mcp/register.ts` wrapper writes a record after every successful
 * `executeTool` call; the homepage polls `GET /tabs/activity` to
 * render the live view.
 *
 * `status` is derived at read time: a record is `active` when the
 * last tool fired within `ACTIVE_WINDOW_MS`, otherwise `idle`. No
 * background timers. Records whose underlying tab has closed are
 * evicted lazily on the next `snapshot()` read; we detect that by
 * looking up `pageId` on the live `PageManager` and confirming the
 * targetId still matches (pageIds are reused after a tab closes).
 */

import type { BrowserSession } from '@browseros/server/browser/core/session'

export interface TabActivityRecord {
  targetId: string
  pageId: number
  url: string
  title: string
  agentId: string
  slug: string
  lastToolAt: number
  lastToolName: string
  status: 'active' | 'idle'
}

export const ACTIVE_WINDOW_MS = 5000

export interface RegistryDeps {
  getSession(): BrowserSession | null
  now?: () => number
}

interface RawRecord {
  targetId: string
  pageId: number
  agentId: string
  slug: string
  lastToolAt: number
  lastToolName: string
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
      records.set(input.targetId, {
        targetId: input.targetId,
        pageId: input.pageId,
        agentId: input.agentId,
        slug: input.slug,
        lastToolAt: now(),
        lastToolName: input.toolName,
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
          lastToolAt: raw.lastToolAt,
          lastToolName: raw.lastToolName,
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
