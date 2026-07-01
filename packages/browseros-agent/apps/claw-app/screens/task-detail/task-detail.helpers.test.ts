/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import type { ToolDispatchRow } from '@/modules/api/audit.hooks'
import { groupDispatchesByTab, pickDefaultTabId } from './task-detail.helpers'

function dispatch(
  id: number,
  pageId: number | null,
  overrides: Partial<ToolDispatchRow> = {},
): ToolDispatchRow {
  return {
    id,
    createdAt: 1_000_000 + id,
    agentId: 'codex',
    slug: 'codex',
    agentLabel: 'Codex',
    sessionId: 's',
    toolName: 'snapshot',
    pageId,
    targetId: null,
    url: null,
    title: null,
    argsJson: null,
    resultMeta: null,
    durationMs: 5,
    ...overrides,
  }
}

describe('groupDispatchesByTab', () => {
  it('returns an empty array for zero dispatches', () => {
    expect(groupDispatchesByTab([], [])).toEqual([])
  })

  it('all null-pageId dispatches yields a Session bucket only (no page tabs)', () => {
    const rows = [dispatch(1, null), dispatch(2, null), dispatch(3, null)]
    const groups = groupDispatchesByTab(rows, [])
    expect(groups).toHaveLength(1)
    expect(groups[0]!.id).toBe('session')
    expect(groups[0]!.label).toBe('Session')
    expect(groups[0]!.pageId).toBeNull()
    expect(groups[0]!.dispatchCount).toBe(3)
    expect(groups[0]!.dispatches.map((d) => d.id)).toEqual([1, 2, 3])
  })

  it('single pageId + zero null dispatches yields Session + one page tab', () => {
    const rows = [
      dispatch(1, 7, { url: 'https://a.example/', title: 'A' }),
      dispatch(2, 7, { url: 'https://a.example/next', title: 'A2' }),
    ]
    const groups = groupDispatchesByTab(rows, [])
    expect(groups).toHaveLength(2)
    expect(groups[0]!.id).toBe('session')
    expect(groups[0]!.dispatchCount).toBe(2)
    expect(groups[1]!.id).toBe('page-7')
    expect(groups[1]!.label).toBe('Tab 1')
  })

  it('page tabs are labelled sequentially by first-appearance order (Tab 1, Tab 2, ...) not by raw pageId', () => {
    // codex may open pageIds 43, 21, 55 in that chronological order.
    // We want Tab 1 -> 43, Tab 2 -> 21, Tab 3 -> 55.
    const rows = [
      dispatch(1, 43),
      dispatch(2, 21),
      dispatch(3, 43),
      dispatch(4, 55),
      dispatch(5, 21),
    ]
    const groups = groupDispatchesByTab(rows, [])
    const pageGroups = groups.filter((g) => g.id !== 'session')
    expect(pageGroups.map((g) => g.label)).toEqual(['Tab 1', 'Tab 2', 'Tab 3'])
    expect(pageGroups.map((g) => g.pageId)).toEqual([43, 21, 55])
  })

  it('Session bucket contains EVERY dispatch (overview semantics)', () => {
    const rows = [
      dispatch(1, null),
      dispatch(2, 7),
      dispatch(3, 1),
      dispatch(4, null),
      dispatch(5, 3),
    ]
    const groups = groupDispatchesByTab(rows, [])
    const session = groups.find((g) => g.id === 'session')!
    expect(session.dispatchCount).toBe(5)
    expect(session.dispatches.map((d) => d.id)).toEqual([1, 2, 3, 4, 5])
  })

  it('Session bucket contains EVERY screenshot (overview semantics)', () => {
    const rows = [
      dispatch(1, null),
      dispatch(2, 3),
      dispatch(3, 3),
      dispatch(4, 7),
      dispatch(5, 7),
    ]
    const groups = groupDispatchesByTab(rows, [1, 3, 5])
    const session = groups.find((g) => g.id === 'session')!
    expect(session.screenshotDispatchIds).toEqual([1, 3, 5])
  })

  it('preserves chronological order inside per-page groups', () => {
    const rows = [
      dispatch(1, 5),
      dispatch(2, null),
      dispatch(3, 5),
      dispatch(4, 5),
      dispatch(5, null),
    ]
    const groups = groupDispatchesByTab(rows, [])
    expect(
      groups.find((g) => g.id === 'page-5')!.dispatches.map((d) => d.id),
    ).toEqual([1, 3, 4])
  })

  it('per-page displayUrl uses the LAST non-null url observed in that group', () => {
    const rows = [
      dispatch(1, 7, { url: 'https://first.example/', title: 'First' }),
      dispatch(2, 7, { url: null, title: null }),
      dispatch(3, 7, { url: 'https://latest.example/', title: 'Latest' }),
      dispatch(4, 7, { url: null, title: null }),
    ]
    const g = groupDispatchesByTab(rows, []).find((x) => x.id === 'page-7')!
    expect(g.displayUrl).toBe('https://latest.example/')
    expect(g.displayTitle).toBe('Latest')
  })

  it('per-page displayUrl is null when every url in the group is null', () => {
    const rows = [dispatch(1, 9), dispatch(2, 9)]
    const g = groupDispatchesByTab(rows, []).find((x) => x.id === 'page-9')!
    expect(g.displayUrl).toBeNull()
    expect(g.displayTitle).toBeNull()
  })

  it('per-page displayUrl + displayTitle come from the SAME paired dispatch when one exists', () => {
    // Simulates a mid-navigation dispatch (has url but null title)
    // FOLLOWED by a load-complete dispatch (has both url + title).
    // Without the paired-preference guard, `lastWithTitle` might
    // reach further back and return a stale title next to the
    // freshest url. With the guard, both come from the last
    // dispatch that carried them together.
    const rows = [
      dispatch(1, 7, { url: 'https://old.example/', title: 'Old title' }),
      dispatch(2, 7, { url: 'https://new.example/', title: null }),
      dispatch(3, 7, { url: 'https://new.example/', title: 'New title' }),
    ]
    const g = groupDispatchesByTab(rows, []).find((x) => x.id === 'page-7')!
    expect(g.displayUrl).toBe('https://new.example/')
    expect(g.displayTitle).toBe('New title')
  })

  it('per-page displayUrl/displayTitle fall back independently when no paired dispatch exists', () => {
    // No single dispatch carries both fields; each field falls
    // back to its own most-recent non-null value.
    const rows = [
      dispatch(1, 3, { url: null, title: 'Only title' }),
      dispatch(2, 3, { url: 'https://only-url.example/', title: null }),
    ]
    const g = groupDispatchesByTab(rows, []).find((x) => x.id === 'page-3')!
    expect(g.displayUrl).toBe('https://only-url.example/')
    expect(g.displayTitle).toBe('Only title')
  })

  it('per-page screenshotDispatchIds filters to that page only', () => {
    const rows = [
      dispatch(1, null),
      dispatch(2, 3),
      dispatch(3, 3),
      dispatch(4, 7),
      dispatch(5, 7),
    ]
    // Assume screenshots exist for ids 1, 3, 5 (mixed groups).
    const groups = groupDispatchesByTab(rows, [1, 3, 5])
    expect(
      groups.find((g) => g.id === 'page-3')!.screenshotDispatchIds,
    ).toEqual([3])
    expect(
      groups.find((g) => g.id === 'page-7')!.screenshotDispatchIds,
    ).toEqual([5])
  })
})

describe('pickDefaultTabId', () => {
  it('always returns Session when it exists (overview-first)', () => {
    const rows = [dispatch(1, null), dispatch(2, 3), dispatch(3, 7)]
    expect(pickDefaultTabId(groupDispatchesByTab(rows, []))).toBe('session')
  })

  it('returns Session even for a session-only task', () => {
    const rows = [dispatch(1, null), dispatch(2, null)]
    expect(pickDefaultTabId(groupDispatchesByTab(rows, []))).toBe('session')
  })

  it('returns Session even for a page-only task', () => {
    const rows = [dispatch(1, 5), dispatch(2, 5)]
    expect(pickDefaultTabId(groupDispatchesByTab(rows, []))).toBe('session')
  })

  it('returns undefined for an empty group list', () => {
    expect(pickDefaultTabId([])).toBeUndefined()
  })
})
