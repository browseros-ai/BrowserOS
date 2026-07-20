/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import type { ReplayEvent, ReplayFrame } from '@/modules/api/replay.hooks'
import type { ReplayTabData } from './replay.data'
import {
  buildReplayDocumentIds,
  buildReplayEventCatalog,
  buildReplayTabIds,
} from './replay-events'
import {
  type BuildTabViewInput,
  buildTabView,
  tabSeekForFrame,
} from './tab-view'

function frame(
  t: number,
  tabId: number | null,
  extra: Partial<ReplayFrame> = {},
): ReplayFrame {
  return {
    t,
    kind: 'action',
    verb: 'read',
    node: 'test',
    caption: 'test',
    tabId,
    ...extra,
  }
}

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
    frames: [],
    tabs: [],
    eventsForDocument: () => [],
    startedAtMs: 1_000_000,
    ...overrides,
  }
}

describe('buildTabView', () => {
  it('returns empty without a selected tab and document', () => {
    const view = buildTabView(makeInput(), null, null)
    expect(view.events).toEqual([])
    expect(view.frames).toEqual([])
    expect(view.totalSeconds).toBe(0)
  })

  it('keeps navigation documents in separate rrweb player inputs', () => {
    const events = [
      event(1_001_000, 'document-a'),
      event(1_002_000, 'document-a', 1, 3),
      event(1_003_000, 'document-b'),
      event(1_004_000, 'document-b', 1, 3),
    ]
    const catalog = buildReplayEventCatalog(events)
    const input = makeInput({
      tabs: [
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
      ],
      eventsForDocument: catalog.eventsForDocument,
    })

    expect(
      buildTabView(input, 1, 'document-a').events.map(
        (candidate) => candidate.documentId,
      ),
    ).toEqual(['document-a', 'document-a'])
    expect(
      buildTabView(input, 1, 'document-b').events.map(
        (candidate) => candidate.documentId,
      ),
    ).toEqual(['document-b', 'document-b'])
  })

  it('assigns tab captions to the nearest document ownership window', () => {
    const input = makeInput({
      frames: [frame(1, 1), frame(3, 1), frame(3, 2), frame(6, 1)],
      tabs: [
        tab(1, [
          {
            documentId: 'document-a',
            firstEventAt: 1_002_000,
            lastEventAt: 1_004_000,
          },
          {
            documentId: 'document-b',
            firstEventAt: 1_005_000,
            lastEventAt: 1_007_000,
          },
        ]),
      ],
    })
    const view = buildTabView(input, 1, 'document-a')

    expect(view.frames).toHaveLength(2)
    expect(view.frames.map((candidate) => candidate.t)).toEqual([0, 2])
  })

  it('keeps a document event array stable across audit polling', () => {
    const catalog = buildReplayEventCatalog([
      event(1_002_000, 'document-a'),
      event(1_003_000, 'document-a', 1, 3),
    ])
    const tabs = [
      tab(1, [
        {
          documentId: 'document-a',
          firstEventAt: 1_002_000,
          lastEventAt: 1_003_000,
        },
      ]),
    ]
    const first = buildTabView(
      makeInput({
        frames: [frame(2, 1)],
        tabs,
        eventsForDocument: catalog.eventsForDocument,
      }),
      1,
      'document-a',
    )
    const afterAuditPoll = buildTabView(
      makeInput({
        frames: [frame(2, 1), frame(3, 1)],
        tabs,
        eventsForDocument: catalog.eventsForDocument,
      }),
      1,
      'document-a',
    )

    expect(afterAuditPoll.events).toBe(first.events)
    expect(afterAuditPoll.frames).not.toBe(first.frames)
  })

  it('reuses the playable slice after leading orphan mutations', () => {
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
      eventsForDocument: () => rawEvents,
    })

    const first = buildTabView(input, 1, 'document-a')
    const second = buildTabView(input, 1, 'document-a')
    expect(first.events.map(({ type }) => type)).toEqual([2, 3])
    expect(second.events).toBe(first.events)
    expect(first.incompleteUntilMs).toBe(3_000)
    expect(first.knownIncomplete).toBe(true)
  })

  it('surfaces a cataloged gap even when the segment is playable', () => {
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
      eventsForDocument: () => [
        event(1_001_000, 'document-gap'),
        event(1_002_000, 'document-gap', 1, 3),
      ],
    })

    expect(buildTabView(input, 1, 'document-gap').knownIncomplete).toBe(true)
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
      eventsForDocument: () => [
        event(1_001_000, 'document-missing-snapshot', 1, 3),
      ],
    })

    const view = buildTabView(input, 1, 'document-missing-snapshot')
    expect(view.hasFullSnapshot).toBe(false)
    expect(view.knownIncomplete).toBe(true)
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

describe('tabSeekForFrame', () => {
  it('switches tab and navigation segment using dispatch tab identity and time', () => {
    const selectedFrame = frame(12, 2, { dispatchId: 22 })
    const events = [
      event(1_010_000, 'document-b', 2),
      event(1_015_000, 'document-b', 2, 3),
    ]
    const input = makeInput({
      frames: [frame(1, 1), selectedFrame],
      tabs: [
        tab(1, [
          {
            documentId: 'document-a',
            firstEventAt: 1_001_000,
            lastEventAt: 1_005_000,
          },
        ]),
        tab(2, [
          {
            documentId: 'document-b',
            firstEventAt: 1_010_000,
            lastEventAt: 1_015_000,
          },
        ]),
      ],
      eventsForDocument: buildReplayEventCatalog(events).eventsForDocument,
    })

    expect(tabSeekForFrame(input, 1, 'document-a', selectedFrame)).toEqual({
      tabId: 2,
      documentId: 'document-b',
      seconds: 2,
    })
  })
})
