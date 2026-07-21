/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import type { ReplayEvent } from '@/modules/api/replay.hooks'
import { buildReplayEventCatalog } from './replay-events'

function event(ts: number, tabId: number, documentId: string): ReplayEvent {
  return {
    sessionId: 'session-1',
    documentId,
    targetId: `target-${tabId}`,
    tabId,
    type: 2,
    data: {},
    ts,
  }
}

describe('buildReplayEventCatalog', () => {
  it('indexes one stable chronological stream per Chrome tab', () => {
    const events = [
      event(1_000, 1, 'document-a'),
      event(2_000, 1, 'document-a'),
      event(3_000, 1, 'document-b'),
      event(4_000, 2, 'document-c'),
    ]
    const catalog = buildReplayEventCatalog(events)

    expect(catalog.eventsForTab(1).map(({ documentId }) => documentId)).toEqual(
      ['document-a', 'document-a', 'document-b'],
    )
    expect(catalog.eventsForTab(2)).toEqual([events[3]])
    expect(catalog.eventsForTab(1)).toBe(catalog.eventsForTab(1))
    expect(catalog.eventsForTab(99)).toBe(catalog.eventsForTab(99))
    expect(catalog.documentIdsForTab(1)).toEqual(['document-a', 'document-b'])
  })
})
