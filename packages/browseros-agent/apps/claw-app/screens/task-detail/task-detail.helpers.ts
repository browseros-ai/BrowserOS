/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure helpers for the task-detail screen. Isolated from the
 * screen component so they can be unit-tested against synthetic
 * dispatch rows without a React tree.
 */

import type { SessionScreenshot } from '@browseros/claw-api'
import type { ToolDispatchRow } from '@/modules/api/audit.hooks'

export interface TabGroup {
  /**
   * Stable id used by the shadcn Tabs primitive. `'session'` for
   * pageId-less dispatches; `'page-${pageId}'` for a real tab.
   */
  id: string
  pageId: number | null
  /** Human label shown on the tab trigger. */
  label: string
  /**
   * Latest URL observed among this group's dispatches. Null when
   * every dispatch in the group has a null url (typical for the
   * Session bucket). Displayed as the tab body's header hint.
   */
  displayUrl: string | null
  displayTitle: string | null
  /** Chronological subset of the task's dispatches for this group. */
  dispatches: ToolDispatchRow[]
  dispatchCount: number
  /** Chronological subset of the session's screenshots for this group. */
  screenshots: SessionScreenshot[]
}

/** Groups task dispatches into a session overview and first-seen page buckets for task detail. */
export function groupDispatchesByTab(
  dispatches: ToolDispatchRow[],
  allScreenshots: readonly SessionScreenshot[],
): TabGroup[] {
  const buckets = new Map<number, ToolDispatchRow[]>()
  const pageBuckets: Array<[number, ToolDispatchRow[]]> = []
  for (const d of dispatches) {
    if (d.pageId === undefined) continue
    const arr = buckets.get(d.pageId)
    if (arr) {
      arr.push(d)
    } else {
      const rows = [d]
      buckets.set(d.pageId, rows)
      pageBuckets.push([d.pageId, rows])
    }
  }

  const groups: TabGroup[] = []

  if (dispatches.length > 0) {
    groups.push({
      id: 'session',
      pageId: null,
      label: 'Session',
      displayUrl: null,
      displayTitle: null,
      dispatches,
      dispatchCount: dispatches.length,
      screenshots: [...allScreenshots],
    })
  }

  pageBuckets.forEach(([pageId, rows], idx) => {
    // Prefer a paired title/url snapshot to avoid showing a stale title beside a fresh URL.
    const reversed = [...rows].reverse()
    const lastPaired = reversed.find((d) => d.url && d.title)
    const lastWithUrl = lastPaired ?? reversed.find((d) => d.url)
    const lastWithTitle = lastPaired ?? reversed.find((d) => d.title)
    const screenshotIds = new Set(
      rows.flatMap((row) =>
        row.screenshotId === undefined ? [] : [row.screenshotId],
      ),
    )
    groups.push({
      id: `page-${pageId}`,
      pageId,
      label: `Tab ${idx + 1}`,
      displayUrl: lastWithUrl?.url ?? null,
      displayTitle: lastWithTitle?.title ?? null,
      dispatches: rows,
      dispatchCount: rows.length,
      screenshots: allScreenshots.filter((screenshot) =>
        screenshotIds.has(screenshot.screenshotId),
      ),
    })
  })

  return groups
}

/** Selects the session overview when available so task detail opens at the task-wide timeline. */
export function pickDefaultTabId(groups: TabGroup[]): string | undefined {
  const session = groups.find((g) => g.id === 'session')
  return session?.id ?? groups[0]?.id
}
