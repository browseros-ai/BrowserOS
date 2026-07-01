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
 * Groups a task's dispatch stream into a leftmost "Session"
 * overview bucket plus one bucket per distinct `pageId`.
 *
 * The Session bucket is the task-level overview: it contains
 * EVERY dispatch and EVERY screenshot, so the operator can see
 * the whole task at a glance without picking a specific tab.
 * Each per-page bucket contains only the dispatches + screenshots
 * scoped to that tab.
 *
 * Page tabs are labelled sequentially (`Tab 1`, `Tab 2`, ...) in
 * chronological order of first appearance. The underlying BrowserOS
 * pageId is intentionally not surfaced in the label: it is stable
 * across the extension's lifetime and can range into the hundreds,
 * which reads as noise.
 *
 * Complexity: O(N) over dispatches, one pass. Callers should
 * memoise per task-detail response.
 */
export function groupDispatchesByTab(
  dispatches: ToolDispatchRow[],
  allScreenshotIds: readonly number[],
): TabGroup[] {
  const screenshotSet = new Set(allScreenshotIds)
  const buckets = new Map<number, ToolDispatchRow[]>()
  // Preserve first-seen order for the sequential Tab numbering
  // (agents typically open tabs in chronological order, so this
  // matches the operator's mental narrative better than sorting
  // by raw pageId).
  const pageOrder: number[] = []
  for (const d of dispatches) {
    if (d.pageId === null) continue
    const arr = buckets.get(d.pageId)
    if (arr) {
      arr.push(d)
    } else {
      buckets.set(d.pageId, [d])
      pageOrder.push(d.pageId)
    }
  }

  const groups: TabGroup[] = []

  // Session overview: EVERY dispatch and EVERY screenshot. This is
  // the default view when the operator opens the task and does not
  // yet know which tab they want to drill into.
  if (dispatches.length > 0) {
    groups.push({
      id: 'session',
      pageId: null,
      label: 'Session',
      displayUrl: null,
      displayTitle: null,
      dispatches,
      dispatchCount: dispatches.length,
      screenshotDispatchIds: dispatches
        .map((d) => d.id)
        .filter((id) => screenshotSet.has(id)),
    })
  }

  // Per-page buckets, numbered by first-appearance order.
  pageOrder.forEach((pageId, idx) => {
    const rows = buckets.get(pageId)!
    // Prefer a dispatch that carries url + title TOGETHER so the
    // tab header shows a consistent (url, title) pair from a
    // single moment in time. If no such paired dispatch exists in
    // this tab, fall back to the last individual non-null values
    // independently. The edge case: mid-navigation dispatches may
    // have a url but null title (or vice versa); without this
    // guard the header could show a stale title alongside a fresh
    // url.
    const reversed = [...rows].reverse()
    const lastPaired = reversed.find((d) => d.url !== null && d.title !== null)
    const lastWithUrl = lastPaired ?? reversed.find((d) => d.url !== null)
    const lastWithTitle = lastPaired ?? reversed.find((d) => d.title !== null)
    groups.push({
      id: `page-${pageId}`,
      pageId,
      label: `Tab ${idx + 1}`,
      displayUrl: lastWithUrl?.url ?? null,
      displayTitle: lastWithTitle?.title ?? null,
      dispatches: rows,
      dispatchCount: rows.length,
      screenshotDispatchIds: rows
        .map((d) => d.id)
        .filter((id) => screenshotSet.has(id)),
    })
  })

  return groups
}

/**
 * Picks which tab should be selected first. Prefers the Session
 * overview so the operator sees the task-wide screenshot strip
 * and timeline on entry; page tabs are drill-downs. Falls back to
 * the first available id when Session is absent (e.g. an empty
 * task).
 */
export function pickDefaultTabId(groups: TabGroup[]): string | undefined {
  const session = groups.find((g) => g.id === 'session')
  return session?.id ?? groups[0]?.id
}
