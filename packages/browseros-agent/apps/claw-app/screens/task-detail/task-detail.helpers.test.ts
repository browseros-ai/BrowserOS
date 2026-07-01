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

  it('groups all null-pageId dispatches into a single Session bucket', () => {
    const rows = [dispatch(1, null), dispatch(2, null), dispatch(3, null)]
    const groups = groupDispatchesByTab(rows, [])
    expect(groups).toHaveLength(1)
    expect(groups[0]!.id).toBe('session')
    expect(groups[0]!.label).toBe('Session')
    expect(groups[0]!.pageId).toBeNull()
    expect(groups[0]!.dispatchCount).toBe(3)
    expect(groups[0]!.dispatches.map((d) => d.id)).toEqual([1, 2, 3])
  })

  it('single pageId with no null dispatches yields exactly one page bucket', () => {
    const rows = [
      dispatch(1, 7, { url: 'https://a.example/', title: 'A' }),
      dispatch(2, 7, { url: 'https://a.example/next', title: 'A2' }),
    ]
    const groups = groupDispatchesByTab(rows, [])
    expect(groups).toHaveLength(1)
    expect(groups[0]!.id).toBe('page-7')
    expect(groups[0]!.label).toBe('Page 7')
    expect(groups[0]!.dispatchCount).toBe(2)
  })

  it('mixed session + pages produces Session first, then ascending pageIds', () => {
    const rows = [
      dispatch(1, null),
      dispatch(2, 7),
      dispatch(3, 1),
      dispatch(4, null),
      dispatch(5, 3),
      dispatch(6, 7),
      dispatch(7, 3),
      dispatch(8, 1),
    ]
    const groups = groupDispatchesByTab(rows, [])
    expect(groups.map((g) => g.id)).toEqual([
      'session',
      'page-1',
      'page-3',
      'page-7',
    ])
    expect(groups[0]!.dispatchCount).toBe(2)
    expect(groups[1]!.dispatchCount).toBe(2)
    expect(groups[2]!.dispatchCount).toBe(2)
    expect(groups[3]!.dispatchCount).toBe(2)
  })

  it('preserves chronological order inside each group', () => {
    const rows = [
      dispatch(1, 5),
      dispatch(2, null),
      dispatch(3, 5),
      dispatch(4, 5),
      dispatch(5, null),
    ]
    const groups = groupDispatchesByTab(rows, [])
    expect(
      groups.find((g) => g.id === 'session')!.dispatches.map((d) => d.id),
    ).toEqual([2, 5])
    expect(
      groups.find((g) => g.id === 'page-5')!.dispatches.map((d) => d.id),
    ).toEqual([1, 3, 4])
  })

  it('displayUrl uses the LAST non-null url observed in the group', () => {
    const rows = [
      dispatch(1, 7, { url: 'https://first.example/', title: 'First' }),
      dispatch(2, 7, { url: null, title: null }),
      dispatch(3, 7, { url: 'https://latest.example/', title: 'Latest' }),
      dispatch(4, 7, { url: null, title: null }),
    ]
    const g = groupDispatchesByTab(rows, [])[0]!
    expect(g.displayUrl).toBe('https://latest.example/')
    expect(g.displayTitle).toBe('Latest')
  })

  it('displayUrl is null when every url in the group is null', () => {
    const rows = [dispatch(1, 9), dispatch(2, 9)]
    const g = groupDispatchesByTab(rows, [])[0]!
    expect(g.displayUrl).toBeNull()
    expect(g.displayTitle).toBeNull()
  })

  it('screenshotDispatchIds filters allScreenshotIds to this group only', () => {
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
      groups.find((g) => g.id === 'session')!.screenshotDispatchIds,
    ).toEqual([1])
    expect(
      groups.find((g) => g.id === 'page-3')!.screenshotDispatchIds,
    ).toEqual([3])
    expect(
      groups.find((g) => g.id === 'page-7')!.screenshotDispatchIds,
    ).toEqual([5])
  })
})

describe('pickDefaultTabId', () => {
  it('returns the first non-session page tab when one exists', () => {
    const rows = [dispatch(1, null), dispatch(2, 3), dispatch(3, 7)]
    expect(pickDefaultTabId(groupDispatchesByTab(rows, []))).toBe('page-3')
  })

  it('falls back to session when no page tabs exist', () => {
    const rows = [dispatch(1, null), dispatch(2, null)]
    expect(pickDefaultTabId(groupDispatchesByTab(rows, []))).toBe('session')
  })

  it('returns undefined for an empty group list', () => {
    expect(pickDefaultTabId([])).toBeUndefined()
  })
})
