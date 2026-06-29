/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure helpers shared by the background service worker. Kept side-
 * effect-free so they unit-test cleanly under bun:test; chrome.*
 * API calls live in `entrypoints/background.ts`.
 */

import type {
  ChromeTabRecord,
  ReplayMapDiff,
  ReplayTab,
} from './replay-background.types'

/**
 * Diffs the current `chromeTabId -> record` map against a fresh
 * `(chromeTabId, ReplayTab)` resolution and emits what the
 * background should add / remove via chrome.* APIs.
 *
 * `currentMap` is the background's authoritative local state.
 * `resolved` is what the latest /replay/tabs poll produced AFTER
 * the chrome tab id has been resolved via chrome.tabs.query.
 * Rows in /replay/tabs whose chrome tab id could not be resolved
 * (no matching open tab) are simply omitted from `resolved` and
 * therefore appear as no-ops here; the background re-tries on the
 * next poll.
 */
export function diffReplayMap(
  currentMap: ReadonlyMap<number, ChromeTabRecord>,
  resolved: ReadonlyArray<{ chromeTabId: number; record: ChromeTabRecord }>,
): ReplayMapDiff {
  const nextIds = new Set<number>(resolved.map((r) => r.chromeTabId))
  const added: ReplayMapDiff['added'] = []
  const changed: ReplayMapDiff['changed'] = []
  for (const r of resolved) {
    const existing = currentMap.get(r.chromeTabId)
    if (!existing) {
      added.push(r)
      continue
    }
    if (
      existing.sessionId !== r.record.sessionId ||
      existing.tabPageId !== r.record.tabPageId
    ) {
      changed.push(r)
    }
  }
  const removed: number[] = []
  for (const tabId of currentMap.keys()) {
    if (!nextIds.has(tabId)) removed.push(tabId)
  }
  return { added, changed, removed }
}

/**
 * Pick the chrome tab that matches a `ReplayTab` row. The chrome
 * tab id is NOT something the cockpit knows; the cockpit knows
 * URL + agent tab-group colour. We pass in:
 *
 *   - `candidates`: result of `chrome.tabs.query({url})`
 *   - `groupColors`: `Map<groupId, TabGroupColor>` populated from
 *      `chrome.tabGroups.query({})`
 *   - `replayTab`: the row to match
 *
 * Match strategy:
 *
 *   1. If only one candidate, return it. Most common.
 *   2. If multiple AND `groupColor` is null (the cockpit's tab
 *      group has not been registered yet), return null. We would
 *      rather defer to the next poll than guess wrong. Wrong picks
 *      attribute the operator's tab events to the agent's replay,
 *      and the chrome.tabs.query result order is implementation
 *      defined.
 *   3. If multiple AND `groupColor` is set, narrow by colour. The
 *      cockpit assigns each agent a distinct tab-group colour so
 *      concurrent agents on the same URL pick different groups.
 *   4. If colour narrowing finds none (no matching colour), return
 *      null too: no candidate matches our agent. We do not fall
 *      back to first-match because that risks mis-attribution.
 *   5. Returns null when no candidate has a usable id.
 */
export function pickChromeTab(args: {
  candidates: Array<{
    id?: number
    groupId?: number
    url?: string
    title?: string
  }>
  groupColors: ReadonlyMap<number, string>
  replayTab: ReplayTab
}): number | null {
  const usable = args.candidates.filter(
    (c): c is { id: number; groupId?: number; url?: string; title?: string } =>
      typeof c.id === 'number',
  )
  if (usable.length === 0) return null
  if (usable.length === 1) return usable[0].id

  // Multiple URL matches and we have no colour to disambiguate; defer.
  if (args.replayTab.groupColor === null) return null

  const byColor = usable.filter((c) => {
    if (typeof c.groupId !== 'number') return false
    return args.groupColors.get(c.groupId) === args.replayTab.groupColor
  })
  if (byColor.length >= 1) return byColor[0].id
  // No colour match either. Defer rather than risk mis-attribution.
  return null
}

/**
 * Match-pattern friendly URL. `chrome.tabs.query({url})` accepts
 * a URL-or-pattern but interprets bare URLs as exact matches
 * including trailing slash semantics. Normalising both sides
 * first reduces miss-matches when the recorder side has appended
 * a slash or stripped a hash.
 */
export function normalizeUrl(url: string): string {
  try {
    const u = new URL(url)
    // Drop fragment; the recorder does not care about hash navigation
    // for the chrome.tabs match (rrweb captures the in-page state).
    u.hash = ''
    return u.toString()
  } catch {
    return url
  }
}
