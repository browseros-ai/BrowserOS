/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure helpers for the task-detail screen. Isolated from the
 * screen component so they can be unit-tested against synthetic
 * dispatch rows without a React tree.
 */

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
  /** Subset of `allScreenshotIds` whose dispatch belongs to this group. */
  screenshotDispatchIds: number[]
}

/**
 * Groups a task's dispatch stream into one bucket per distinct
 * `pageId`, plus a leftmost "Session" bucket for dispatches with
 * `pageId === null`. Ordering: Session first (if present), then
 * ascending numeric pageId. Chronological order is preserved
 * inside each group.
 *
 * Complexity: O(N) over dispatches, one pass. Callers should
 * memoise per task-detail response.
 */
export function groupDispatchesByTab(
  dispatches: ToolDispatchRow[],
  allScreenshotIds: readonly number[],
): TabGroup[] {
  const screenshotSet = new Set(allScreenshotIds)
  const buckets = new Map<number | 'session', ToolDispatchRow[]>()
  for (const d of dispatches) {
    const key: number | 'session' = d.pageId ?? 'session'
    const arr = buckets.get(key)
    if (arr) arr.push(d)
    else buckets.set(key, [d])
  }

  const groups: TabGroup[] = []

  const sessionRows = buckets.get('session')
  if (sessionRows && sessionRows.length > 0) {
    groups.push({
      id: 'session',
      pageId: null,
      label: 'Session',
      displayUrl: null,
      displayTitle: null,
      dispatches: sessionRows,
      dispatchCount: sessionRows.length,
      screenshotDispatchIds: sessionRows
        .map((d) => d.id)
        .filter((id) => screenshotSet.has(id)),
    })
  }

  const pageKeys: number[] = []
  for (const key of buckets.keys()) {
    if (typeof key === 'number') pageKeys.push(key)
  }
  pageKeys.sort((a, b) => a - b)

  for (const pageId of pageKeys) {
    const rows = buckets.get(pageId)!
    const lastWithUrl = [...rows].reverse().find((d) => d.url !== null)
    const lastWithTitle = [...rows].reverse().find((d) => d.title !== null)
    groups.push({
      id: `page-${pageId}`,
      pageId,
      label: `Page ${pageId}`,
      displayUrl: lastWithUrl?.url ?? null,
      displayTitle: lastWithTitle?.title ?? null,
      dispatches: rows,
      dispatchCount: rows.length,
      screenshotDispatchIds: rows
        .map((d) => d.id)
        .filter((id) => screenshotSet.has(id)),
    })
  }

  return groups
}

/**
 * Picks which tab should be selected first. Prefers the first
 * real page tab (the operator usually wants to see actual page
 * activity first); falls back to Session when the task is
 * pageId-less all the way through.
 */
export function pickDefaultTabId(groups: TabGroup[]): string | undefined {
  const firstPage = groups.find((g) => g.id !== 'session')
  return firstPage?.id ?? groups[0]?.id
}
