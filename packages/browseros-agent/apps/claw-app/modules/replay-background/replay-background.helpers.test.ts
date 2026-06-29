/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure-function tests for the background-worker helpers. The
 * chrome.* surface lives in entrypoints/background.ts and is
 * exercised by manual smoke; this file pins the diff + match +
 * url-normalize logic in isolation.
 */

import { describe, expect, it } from 'bun:test'
import {
  diffReplayMap,
  normalizeUrl,
  pickChromeTab,
} from './replay-background.helpers'
import type { ChromeTabRecord, ReplayTab } from './replay-background.types'

function rec(over: Partial<ChromeTabRecord> = {}): ChromeTabRecord {
  return { sessionId: 'sid-1', tabPageId: 1, ...over }
}

function row(over: Partial<ReplayTab> = {}): ReplayTab {
  return {
    sessionId: 'sid-1',
    tabPageId: 1,
    url: 'https://example.com/',
    title: 'Example',
    groupColor: 'orange',
    ...over,
  }
}

describe('diffReplayMap', () => {
  it('flags every resolved entry as added when the map is empty', () => {
    const diff = diffReplayMap(new Map(), [
      { chromeTabId: 1, record: rec() },
      { chromeTabId: 2, record: rec({ sessionId: 'sid-2', tabPageId: 2 }) },
    ])
    expect(diff.added.map((a) => a.chromeTabId).sort()).toEqual([1, 2])
    expect(diff.changed).toEqual([])
    expect(diff.removed).toEqual([])
  })

  it('no diff when current matches resolved exactly', () => {
    const current = new Map<number, ChromeTabRecord>([[1, rec()]])
    const diff = diffReplayMap(current, [{ chromeTabId: 1, record: rec() }])
    expect(diff.added).toEqual([])
    expect(diff.changed).toEqual([])
    expect(diff.removed).toEqual([])
  })

  it('removes entries not present in resolved', () => {
    const current = new Map<number, ChromeTabRecord>([
      [1, rec()],
      [2, rec({ sessionId: 'sid-2' })],
    ])
    const diff = diffReplayMap(current, [{ chromeTabId: 1, record: rec() }])
    expect(diff.added).toEqual([])
    expect(diff.changed).toEqual([])
    expect(diff.removed).toEqual([2])
  })

  it('flags as CHANGED (not added) when sessionId differs for an existing tab', () => {
    const current = new Map<number, ChromeTabRecord>([[1, rec()]])
    const diff = diffReplayMap(current, [
      { chromeTabId: 1, record: rec({ sessionId: 'sid-2' }) },
    ])
    expect(diff.added).toEqual([])
    expect(diff.changed).toEqual([
      { chromeTabId: 1, record: rec({ sessionId: 'sid-2' }) },
    ])
    expect(diff.removed).toEqual([])
  })

  it('flags as CHANGED when tabPageId differs for an existing tab', () => {
    const current = new Map<number, ChromeTabRecord>([[1, rec()]])
    const diff = diffReplayMap(current, [
      { chromeTabId: 1, record: rec({ tabPageId: 99 }) },
    ])
    expect(diff.added).toEqual([])
    expect(diff.changed.map((c) => c.chromeTabId)).toEqual([1])
    expect(diff.removed).toEqual([])
  })

  it('handles overlap: some same, some added, some changed, some removed', () => {
    const current = new Map<number, ChromeTabRecord>([
      [1, rec()],
      [2, rec({ sessionId: 'sid-2' })],
      [3, rec({ sessionId: 'sid-3' })],
    ])
    const diff = diffReplayMap(current, [
      { chromeTabId: 1, record: rec() }, // same
      { chromeTabId: 2, record: rec({ sessionId: 'sid-2-new' }) }, // changed
      { chromeTabId: 4, record: rec({ sessionId: 'sid-4' }) }, // added
    ])
    expect(diff.added.map((a) => a.chromeTabId)).toEqual([4])
    expect(diff.changed.map((c) => c.chromeTabId)).toEqual([2])
    expect(diff.removed).toEqual([3])
  })
})

describe('pickChromeTab', () => {
  it('returns the only candidate when there is one', () => {
    expect(
      pickChromeTab({
        candidates: [{ id: 42, url: 'https://x.com/', groupId: -1 }],
        groupColors: new Map(),
        replayTab: row({ groupColor: null }),
      }),
    ).toBe(42)
  })

  it('returns null when no candidate has an id', () => {
    expect(
      pickChromeTab({
        candidates: [{ url: 'https://x.com/' }],
        groupColors: new Map(),
        replayTab: row(),
      }),
    ).toBeNull()
  })

  it('disambiguates by groupColor when multiple candidates exist', () => {
    expect(
      pickChromeTab({
        candidates: [
          { id: 1, groupId: 100 },
          { id: 2, groupId: 200 },
        ],
        groupColors: new Map([
          [100, 'blue'],
          [200, 'orange'],
        ]),
        replayTab: row({ groupColor: 'orange' }),
      }),
    ).toBe(2)
  })

  it('returns null when groupColor narrow finds no matching colour', () => {
    // Two tabs in two groups, neither matches our colour. We refuse
    // to guess; better to defer to the next poll than risk
    // mis-attributing the operator's tab events to the agent's session.
    expect(
      pickChromeTab({
        candidates: [
          { id: 1, groupId: 100 },
          { id: 2, groupId: 200 },
        ],
        groupColors: new Map([
          [100, 'cyan'],
          [200, 'purple'],
        ]),
        replayTab: row({ groupColor: 'orange' }),
      }),
    ).toBeNull()
  })

  it('returns null when groupColor is null AND multiple candidates exist', () => {
    // Tab-group creation race: the cockpit knows the agent owns SOME
    // tab on this URL but has not yet registered the tab group. We
    // refuse to guess; the next poll will retry once groupColor is
    // populated.
    expect(
      pickChromeTab({
        candidates: [
          { id: 1, groupId: 100 },
          { id: 2, groupId: 200 },
        ],
        groupColors: new Map([[100, 'blue']]),
        replayTab: row({ groupColor: null }),
      }),
    ).toBeNull()
  })

  it('picks first when two candidates share the matching colour (defence in depth)', () => {
    // Should not happen by design (one colour per agent) but the
    // picker still returns SOMETHING rather than null when colour
    // matches.
    expect(
      pickChromeTab({
        candidates: [
          { id: 1, groupId: 100 },
          { id: 2, groupId: 200 },
        ],
        groupColors: new Map([
          [100, 'orange'],
          [200, 'orange'],
        ]),
        replayTab: row({ groupColor: 'orange' }),
      }),
    ).toBe(1)
  })
})

describe('normalizeUrl', () => {
  it('drops fragment', () => {
    expect(normalizeUrl('https://example.com/foo#section')).toBe(
      'https://example.com/foo',
    )
  })

  it('preserves query params', () => {
    expect(normalizeUrl('https://example.com/foo?bar=1')).toBe(
      'https://example.com/foo?bar=1',
    )
  })

  it('returns the input untouched when it is not a parseable URL', () => {
    expect(normalizeUrl('not a url')).toBe('not a url')
  })

  it('canonicalises trailing slash via WHATWG URL', () => {
    expect(normalizeUrl('https://example.com')).toBe('https://example.com/')
  })
})
