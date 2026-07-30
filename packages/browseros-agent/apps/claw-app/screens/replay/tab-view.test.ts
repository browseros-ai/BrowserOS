/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import type { ReplayEvent } from '@/modules/api/replay.hooks'
import type { ReplayTabData } from './replay.data'
import {
  buildReplayDocumentIds,
  buildReplayEventCatalog,
  buildReplayTabIds,
} from './replay-events'
import {
  type BuildTabViewInput,
  buildTabView,
  isPlayableTabView,
} from './tab-view'

function event(
  ts: number,
  documentId: string,
  tabId = 1,
  type = 2,
): ReplayEvent {
  return {
    sessionId: 'test',
    documentId,
    targetId: `target-${documentId}`,
    tabId,
    type,
    data: {},
    ts,
  }
}

function tab(
  tabId: number,
  segments: Array<{
    documentId: string
    firstEventAt: number
    lastEventAt: number
    hasGap?: boolean
    legacy?: boolean
  }>,
): ReplayTabData {
  return {
    tabId,
    complete: true,
    segments: segments.map((segment) => ({
      targetId: `target-${segment.documentId}`,
      hasGap: false,
      legacy: false,
      ...segment,
    })),
  }
}

function makeInput(
  overrides: Partial<BuildTabViewInput> = {},
): BuildTabViewInput {
  return {
    tabs: [],
    eventsForTab: () => [],
    ...overrides,
  }
}

describe('buildTabView', () => {
  it('returns empty without a selected tab', () => {
    const view = buildTabView(makeInput(), null)
    expect(view.events).toEqual([])
    expect(view.originMs).toBeNull()
    expect(view.endMs).toBeNull()
    expect(view.totalSeconds).toBe(0)
    expect(isPlayableTabView(view)).toBe(false)
  })

  it('merges every document lifecycle from one tab into one track', () => {
    const events = [
      event(1_001_000, 'document-a', 1, 0),
      event(1_001_001, 'document-a', 1, 4),
      event(1_002_000, 'document-a', 1, 2),
      event(1_003_000, 'document-a', 1, 3),
      event(1_004_000, 'document-b', 1, 0),
      event(1_004_001, 'document-b', 1, 4),
      event(1_005_000, 'document-b', 1, 2),
      event(1_006_000, 'document-b', 1, 3),
      event(1_007_000, 'document-c', 2, 2),
    ]
    const catalog = buildReplayEventCatalog(events)
    const input = makeInput({
      tabs: [
        tab(1, [
          {
            documentId: 'document-a',
            firstEventAt: 1_001_000,
            lastEventAt: 1_003_000,
          },
          {
            documentId: 'document-b',
            firstEventAt: 1_004_000,
            lastEventAt: 1_006_000,
          },
        ]),
      ],
      eventsForTab: catalog.eventsForTab,
    })

    const view = buildTabView(input, 1)
    expect(view.events.map(({ documentId }) => documentId)).toEqual([
      'document-a',
      'document-a',
      'document-a',
      'document-a',
      'document-b',
      'document-b',
      'document-b',
      'document-b',
    ])
    expect(view.originMs).toBe(1_001_000)
    expect(view.endMs).toBe(1_006_000)
    expect(view.totalSeconds).toBe(5)
    expect(isPlayableTabView(view)).toBe(true)
  })

  it('keeps a merged tab event array stable across audit polling', () => {
    const events = [
      event(1_001_000, 'document-a'),
      event(1_002_000, 'document-a', 1, 3),
      event(1_003_000, 'document-b'),
      event(1_004_000, 'document-b', 1, 3),
    ]
    const catalog = buildReplayEventCatalog(events)
    const tabs = [
      tab(1, [
        {
          documentId: 'document-a',
          firstEventAt: 1_001_000,
          lastEventAt: 1_002_000,
        },
        {
          documentId: 'document-b',
          firstEventAt: 1_003_000,
          lastEventAt: 1_004_000,
        },
      ]),
    ]
    const first = buildTabView(
      makeInput({ tabs, eventsForTab: catalog.eventsForTab }),
      1,
    )
    const afterAuditPoll = buildTabView(
      makeInput({ tabs, eventsForTab: catalog.eventsForTab }),
      1,
    )

    // Stable identity keeps the single rrweb player alive across polls.
    expect(afterAuditPoll.events).toBe(first.events)
  })

  it('reuses the playable stream after leading orphan mutations', () => {
    const rawEvents = [
      event(1_001_000, 'document-a', 1, 3),
      event(1_004_000, 'document-a'),
      event(1_005_000, 'document-a', 1, 3),
    ]
    const input = makeInput({
      tabs: [
        tab(1, [
          {
            documentId: 'document-a',
            firstEventAt: 1_001_000,
            lastEventAt: 1_005_000,
          },
        ]),
      ],
      eventsForTab: () => rawEvents,
    })

    const first = buildTabView(input, 1)
    const second = buildTabView(input, 1)
    expect(first.events.map(({ type }) => type)).toEqual([2, 3])
    expect(second.events).toBe(first.events)
    // Bounds follow the playable slice, not the discarded orphan mutation.
    expect(first.originMs).toBe(1_004_000)
    expect(first.incompleteUntilMs).toBe(3_000)
    expect(first.knownIncomplete).toBe(true)
  })

  it('surfaces a cataloged gap even when the tab is playable', () => {
    const input = makeInput({
      tabs: [
        tab(1, [
          {
            documentId: 'document-gap',
            firstEventAt: 1_001_000,
            lastEventAt: 1_002_000,
            hasGap: true,
          },
        ]),
      ],
      eventsForTab: () => [
        event(1_001_000, 'document-gap'),
        event(1_002_000, 'document-gap', 1, 3),
      ],
    })

    expect(buildTabView(input, 1).knownIncomplete).toBe(true)
  })

  it('marks an event stream without a full snapshot as incomplete', () => {
    const input = makeInput({
      tabs: [
        tab(1, [
          {
            documentId: 'document-missing-snapshot',
            firstEventAt: 1_001_000,
            lastEventAt: 1_002_000,
          },
        ]),
      ],
      eventsForTab: () => [event(1_001_000, 'document-missing-snapshot', 1, 3)],
    })

    const view = buildTabView(input, 1)
    expect(view.hasFullSnapshot).toBe(false)
    expect(view.knownIncomplete).toBe(true)
    expect(isPlayableTabView(view)).toBe(false)
  })

  it('reports a generic gap when omissions occur before and during playback', () => {
    const events = [
      event(1_000_000, 'document-a', 1, 3),
      event(1_001_000, 'document-a'),
      event(1_002_000, 'document-a', 1, 3),
      event(1_003_000, 'document-b', 1, 3),
      event(1_004_000, 'document-c'),
      event(1_005_000, 'document-c', 1, 3),
    ]
    const input = makeInput({
      tabs: [
        tab(1, [
          {
            documentId: 'document-a',
            firstEventAt: 1_000_000,
            lastEventAt: 1_002_000,
          },
          {
            documentId: 'document-b',
            firstEventAt: 1_003_000,
            lastEventAt: 1_003_000,
          },
          {
            documentId: 'document-c',
            firstEventAt: 1_004_000,
            lastEventAt: 1_005_000,
          },
        ]),
      ],
      eventsForTab: buildReplayEventCatalog(events).eventsForTab,
    })

    const view = buildTabView(input, 1)
    expect(view.events.map(({ documentId }) => documentId)).toEqual([
      'document-a',
      'document-a',
      'document-c',
      'document-c',
    ])
    expect(view.knownIncomplete).toBe(true)
    expect(view.incompleteUntilMs).toBeNull()
  })
})

describe('catalog ordering', () => {
  it('orders logical tabs and documents from metadata before discoveries', () => {
    expect(
      buildReplayTabIds(
        [
          {
            tabId: 2,
            complete: true,
            firstEventAt: 20,
            lastEventAt: 30,
            segments: [],
          },
          {
            tabId: 1,
            complete: true,
            firstEventAt: 10,
            lastEventAt: 15,
            segments: [],
          },
        ],
        [3],
      ),
    ).toEqual([1, 2, 3])
    expect(
      buildReplayDocumentIds(
        [
          {
            documentId: 'later',
            firstEventAt: 20,
            lastEventAt: 30,
            sizeBytes: 1,
            eventCount: 1,
            hasGap: false,
          },
          {
            documentId: 'first',
            firstEventAt: 10,
            lastEventAt: 15,
            sizeBytes: 1,
            eventCount: 1,
            hasGap: false,
          },
        ],
        ['stream-only'],
      ),
    ).toEqual(['first', 'later', 'stream-only'])
  })
})
