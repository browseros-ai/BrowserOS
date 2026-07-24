/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import {
  act,
  createContext,
  type ReactNode,
  useContext,
  useEffect,
} from 'react'
import type { Root } from 'react-dom/client'
import { MemoryRouter } from 'react-router'
import type { ReplayEvent, ReplayFrame } from '@/modules/api/replay.hooks'
import * as replayDataModule from './replay.data'
import { buildReplayEventCatalog } from './replay-events'
import type { Playback } from './use-playback'

let replayResult: replayDataModule.UseReplayDataResult
let autoCommitPresentedTrack = true
let commitPresentedTrack: ((tabId: number) => void) | null = null

mock.module('./replay.data', () => ({
  ...replayDataModule,
  useReplayData: () => replayResult,
}))

mock.module('./ReplayViewport', () => ({
  ReplayViewport: ({
    activeTrack,
    standbyTrack,
    activeTimeMs,
    frame,
    mode,
    onPresentedTrackChange,
  }: {
    activeTrack: { tabId: number; events: readonly ReplayEvent[] }
    standbyTrack: { tabId: number } | null
    activeTimeMs: number
    frame: ReplayFrame | undefined
    mode: string
    onPresentedTrackChange: (tabId: number) => void
  }) => {
    useEffect(() => {
      commitPresentedTrack = onPresentedTrackChange
      if (autoCommitPresentedTrack) {
        onPresentedTrackChange(activeTrack.tabId)
      }
    }, [activeTrack.tabId, onPresentedTrackChange])
    return (
      <div
        data-active-tab={activeTrack.tabId}
        data-standby-tab={standbyTrack?.tabId}
        data-player-documents={activeTrack.events
          .map((event) => event.documentId)
          .join(',')}
        data-player-types={activeTrack.events
          .map((event) => event.type)
          .join(',')}
        data-player-time={activeTimeMs / 1000}
        data-player-mode={mode}
        data-player-caption={frame?.caption}
        data-player-url={frame?.url}
      />
    )
  },
}))

mock.module('./PlaybackTransport', () => ({
  PlaybackTransport: ({
    playback,
    totalSeconds,
    onSeek,
    transportLabel,
  }: {
    playback: Playback
    totalSeconds: number
    onSeek: (seconds: number) => void
    transportLabel: string
  }) => (
    <div
      data-transport-label={transportLabel}
      data-playback-time={playback.time}
      data-playback-playing={playback.isPlaying}
      data-playback-total={totalSeconds}
    >
      <button
        type="button"
        aria-label={`Toggle ${transportLabel}`}
        onClick={playback.togglePlay}
      >
        Toggle
      </button>
      <button type="button" data-seek-twelve onClick={() => onSeek(12)}>
        Seek 12
      </button>
    </div>
  ),
}))

mock.module('./EventTimeline', () => ({
  EventTimeline: ({
    frames,
    currentFrameIndex,
    onSelectFrame,
  }: {
    frames: readonly ReplayFrame[]
    currentFrameIndex: number
    onSelectFrame: (frame: ReplayFrame) => void
  }) => (
    <div data-timeline-current={currentFrameIndex}>
      {frames.map((frame) => (
        <button
          type="button"
          key={frame.dispatchId}
          data-frame-tab={frame.tabId}
          data-frame-id={frame.dispatchId}
          onClick={() => onSelectFrame(frame)}
        >
          {frame.caption}
        </button>
      ))}
    </div>
  ),
}))

const TabSelectContext = createContext<{
  selected: string
  select: (tabId: string) => void
}>({ selected: '', select: () => {} })

mock.module('@/components/ui/tabs', () => ({
  Tabs: ({
    value,
    onValueChange,
    children,
  }: {
    value: string
    onValueChange: (tabId: string) => void
    children: ReactNode
  }) => (
    <TabSelectContext.Provider
      value={{ selected: value, select: onValueChange }}
    >
      <div data-selected-tab={value}>{children}</div>
    </TabSelectContext.Provider>
  ),
  TabsList: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  TabsTrigger: ({
    value,
    children,
    onClick,
    ...props
  }: {
    value: string
    children: ReactNode
    'aria-label'?: string
    onClick?: () => void
  }) => {
    const tabs = useContext(TabSelectContext)
    return (
      <button
        type="button"
        data-tab-chip={value}
        onClick={() => {
          onClick?.()
          if (value !== tabs.selected) tabs.select(value)
        }}
        {...props}
      >
        {children}
      </button>
    )
  },
}))

function event(
  ts: number,
  documentId: string,
  tabId: number,
  type: number,
): ReplayEvent {
  return {
    sessionId: 'session-1',
    documentId,
    targetId: `target-${tabId}`,
    tabId,
    ts,
    type,
    data: type === 4 ? { width: 1280, height: 720 } : {},
  }
}

const sessionEvents: ReplayEvent[] = [
  event(0, 'tab-one-a', 1, 4),
  event(1, 'tab-one-a', 1, 2),
  event(20_000, 'tab-one-a', 1, 3),
  event(30_000, 'tab-one-b', 1, 4),
  event(30_001, 'tab-one-b', 1, 2),
  event(40_000, 'tab-one-b', 1, 3),
  event(5_000, 'tab-two', 2, 4),
  event(5_001, 'tab-two', 2, 2),
  event(50_000, 'tab-two', 2, 3),
  event(6_000, 'tab-four', 4, 4),
  event(6_001, 'tab-four', 4, 2),
  event(50_000, 'tab-four', 4, 3),
  event(3_000, 'tab-three-no-visual', 3, 3),
]

const sessionFrames: ReplayFrame[] = [
  {
    t: 2,
    cameraT: 0,
    kind: 'action',
    verb: 'read',
    node: 'Tab one',
    caption: 'Read tab one',
    url: 'https://one.example/start',
    tabId: 1,
    dispatchId: 1,
  },
  {
    t: 4,
    cameraT: 3,
    kind: 'action',
    verb: 'read',
    node: 'No visual',
    caption: 'Read no-visual tab',
    tabId: 3,
    dispatchId: 3,
  },
  {
    t: 8,
    cameraT: 5,
    kind: 'action',
    verb: 'click',
    node: 'Tab two',
    caption: 'Click tab two',
    url: 'https://two.example/click',
    tabId: 2,
    dispatchId: 2,
  },
  {
    t: 9,
    cameraT: 6,
    kind: 'action',
    verb: 'click',
    node: 'Tab four',
    caption: 'Click tab four',
    url: 'https://four.example/click',
    tabId: 4,
    dispatchId: 4,
  },
  {
    t: 12,
    cameraT: 10,
    kind: 'action',
    verb: 'read',
    node: 'Tab two again',
    caption: 'Return to tab two',
    url: 'https://two.example/result',
    tabId: 2,
    dispatchId: 5,
  },
]

function replayData(
  replayEvents: readonly ReplayEvent[],
  options: {
    frames?: ReplayFrame[]
    tabOrder?: number[]
    totalSeconds?: number
  } = {},
): replayDataModule.ReplayData {
  const eventCatalog = buildReplayEventCatalog(replayEvents)
  const tabOrder =
    options.tabOrder ??
    [1, 2, 4, 3].filter((tabId) => eventCatalog.tabIds.includes(tabId))
  const tabs = tabOrder.map((tabId) => ({
    tabId,
    complete: true,
    segments: eventCatalog.documentIdsForTab(tabId).map((documentId) => {
      const documentEvents = eventCatalog.eventsForDocument(documentId)
      return {
        documentId,
        targetId: documentEvents.find((candidate) => candidate.targetId)
          ?.targetId,
        firstEventAt: documentEvents[0]?.ts ?? 0,
        lastEventAt: documentEvents.at(-1)?.ts ?? 0,
        hasGap: false,
        legacy: false,
      }
    }),
  }))
  const totalSeconds = options.totalSeconds ?? 60
  return {
    sessionId: 'session-1',
    agentLabel: 'Codex',
    taskTitle: 'Replay test',
    harness: 'Codex',
    status: 'done',
    site: 'example.com',
    startedAt: 'Jul 23, 2026',
    startedAtMs: 0,
    duration: `0:${String(totalSeconds).padStart(2, '0')}`,
    tokens: '-',
    steps: String((options.frames ?? sessionFrames).length),
    totalSeconds,
    frames: options.frames ?? sessionFrames,
    complete: true,
    tabs,
    eventsForTab: eventCatalog.eventsForTab,
  }
}

const globalDescriptors = new Map(
  [
    'window',
    'document',
    'navigator',
    'HTMLElement',
    'Node',
    'Event',
    'performance',
  ].map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]),
)

let root: Root | null
let container: HTMLElement
let nowMs = 0
let nextAnimationFrameId = 1
let animationFrames: Map<number, FrameRequestCallback>

beforeEach(async () => {
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  nowMs = 0
  nextAnimationFrameId = 1
  animationFrames = new Map()
  autoCommitPresentedTrack = true
  commitPresentedTrack = null
  const requestAnimationFrame = (callback: FrameRequestCallback): number => {
    const id = nextAnimationFrameId
    nextAnimationFrameId += 1
    animationFrames.set(id, callback)
    return id
  }
  const cancelAnimationFrame = (id: number): void => {
    animationFrames.delete(id)
  }
  Object.assign(dom.window, { requestAnimationFrame, cancelAnimationFrame })
  const globals = {
    window: dom.window,
    document: dom.document,
    navigator: dom.window.navigator,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    Event: dom.window.Event,
    performance: { now: () => nowMs },
  }
  for (const [name, value] of Object.entries(globals)) {
    Object.defineProperty(globalThis, name, {
      configurable: true,
      writable: true,
      value,
    })
  }
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })

  replayResult = {
    replay: replayData(sessionEvents),
    sessionId: 'session-1',
    isLoading: false,
    navigate: mock(() => undefined) as never,
  }
  container = dom.document.getElementById('root') as unknown as HTMLElement
  const { createRoot } = await import('react-dom/client')
  root = createRoot(container)
})

afterEach(async () => {
  if (root) await act(async () => root?.unmount())
  root = null
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
})

async function renderReplay(): Promise<void> {
  await act(async () => {
    root?.render(
      <MemoryRouter initialEntries={['/audit/session-1/replay']}>
        <Replay />
      </MemoryRouter>,
    )
  })
}

async function advanceClock(toMs: number): Promise<void> {
  nowMs = toMs
  const callbacks = [...animationFrames.values()]
  animationFrames.clear()
  await act(async () => {
    for (const callback of callbacks) callback(toMs)
  })
}

function viewport(): Element {
  const element = container.querySelector('[data-active-tab]')
  if (!element) throw new Error('expected replay viewport')
  return element
}

function click(selector: string): Promise<void> {
  const element = container.querySelector(selector)
  if (!element) throw new Error(`missing element: ${selector}`)
  return act(async () => {
    element.dispatchEvent(new window.Event('click', { bubbles: true }))
  })
}

describe('Replay', () => {
  it('follows semantic activity, coalesces parallel work, and never selects a no-visual tab', async () => {
    await renderReplay()

    expect(viewport().getAttribute('data-active-tab')).toBe('1')
    expect(container.textContent).toContain('Following session')
    expect(
      container
        .querySelector('[data-playback-total]')
        ?.getAttribute('data-playback-total'),
    ).toBe('60')

    await advanceClock(3_000)
    expect(viewport().getAttribute('data-active-tab')).toBe('1')
    expect(viewport().getAttribute('data-standby-tab')).toBe('4')

    await advanceClock(5_000)
    expect(viewport().getAttribute('data-standby-tab')).toBe('2')

    await advanceClock(10_000)
    expect(viewport().getAttribute('data-active-tab')).toBe('2')
    expect(viewport().getAttribute('data-player-caption')).toBe(
      'Return to tab two',
    )
    expect(viewport().getAttribute('data-player-url')).toBe(
      'https://two.example/result',
    )
    expect(
      container
        .querySelector('[data-selected-tab]')
        ?.getAttribute('data-selected-tab'),
    ).toBe('2')
    expect(container.querySelectorAll('[data-frame-tab="3"]')).toHaveLength(1)
  })

  it('keeps the chip, URL, and caption on the committed player until promotion is ready', async () => {
    autoCommitPresentedTrack = false
    await renderReplay()

    await advanceClock(10_000)
    expect(viewport().getAttribute('data-active-tab')).toBe('2')
    expect(
      container
        .querySelector('[data-selected-tab]')
        ?.getAttribute('data-selected-tab'),
    ).toBe('1')
    expect(viewport().getAttribute('data-player-url')).not.toBe(
      'https://two.example/result',
    )

    await act(async () => commitPresentedTrack?.(2))
    expect(
      container
        .querySelector('[data-selected-tab]')
        ?.getAttribute('data-selected-tab'),
    ).toBe('2')
    expect(viewport().getAttribute('data-player-caption')).toBe(
      'Return to tab two',
    )
    expect(viewport().getAttribute('data-player-url')).toBe(
      'https://two.example/result',
    )
  })

  it('globally seeks, prefers a selected timeline tab, and ignores a selected no-visual tab', async () => {
    await renderReplay()

    await click('[data-seek-twelve]')
    expect(viewport().getAttribute('data-active-tab')).toBe('2')
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('12')
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('false')

    await click('[data-frame-id="4"]')
    expect(viewport().getAttribute('data-active-tab')).toBe('4')
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('9')

    await click('[data-frame-id="3"]')
    expect(viewport().getAttribute('data-active-tab')).toBe('1')
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('4')
  })

  it('pins a tab at local zero and resumes the exact preserved global moment paused', async () => {
    await renderReplay()
    await advanceClock(2_000)
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('4')

    nowMs = 2_250
    await click('[data-tab-chip="2"]')
    expect(viewport().getAttribute('data-active-tab')).toBe('2')
    expect(viewport().getAttribute('data-player-mode')).toBe('inspect')
    expect(viewport().getAttribute('data-player-time')).toBe('0')
    expect(viewport().getAttribute('data-standby-tab')).toBe(null)
    expect(container.textContent).toContain('Inspecting Tab 2')
    expect(container.textContent).toContain('Tab 2 · Pinned')
    expect(
      container
        .querySelector('[data-transport-label]')
        ?.getAttribute('data-transport-label'),
    ).toBe('Tab 2 playback')

    await click('[aria-label="Toggle Tab 2 playback"]')
    await advanceClock(3_250)
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('2')

    const resumeButton = [...container.querySelectorAll('button')].find(
      (button) => button.textContent === 'Resume session',
    )
    if (!resumeButton) throw new Error('missing Resume session')
    await act(async () => {
      resumeButton.dispatchEvent(new window.Event('click', { bubbles: true }))
    })

    expect(viewport().getAttribute('data-active-tab')).toBe('1')
    expect(viewport().getAttribute('data-player-mode')).toBe('follow')
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('4.5')
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('false')
  })

  it('enters inspection when the already-selected follow chip is clicked', async () => {
    await renderReplay()

    await click('[data-tab-chip="1"]')

    expect(viewport().getAttribute('data-active-tab')).toBe('1')
    expect(viewport().getAttribute('data-player-mode')).toBe('inspect')
    expect(viewport().getAttribute('data-player-time')).toBe('0')
    expect(container.textContent).toContain('Inspecting Tab 1')
    expect(container.textContent).toContain('Tab 1 · Pinned')
  })

  it('switches before ten real seconds when the active track ends', async () => {
    const earlyEvents = [
      event(0, 'short-tab', 1, 4),
      event(1, 'short-tab', 1, 2),
      event(6_000, 'short-tab', 1, 3),
      event(1_000, 'next-tab', 2, 4),
      event(1_001, 'next-tab', 2, 2),
      event(20_000, 'next-tab', 2, 3),
    ]
    replayResult = {
      ...replayResult,
      replay: replayData(earlyEvents, {
        frames: [
          {
            ...sessionFrames[0],
            t: 1,
            cameraT: 0,
            tabId: 1,
          },
          {
            ...sessionFrames[2],
            t: 2,
            cameraT: 1,
            tabId: 2,
          },
        ],
        tabOrder: [1, 2],
        totalSeconds: 20,
      }),
    }
    await renderReplay()

    await advanceClock(3_000)
    expect(viewport().getAttribute('data-active-tab')).toBe('2')
  })

  it('merges same-tab navigation lifecycles into one automatic track', async () => {
    await renderReplay()

    expect(viewport().getAttribute('data-player-documents')).toBe(
      'tab-one-a,tab-one-a,tab-one-a,tab-one-b,tab-one-b,tab-one-b',
    )
    expect(container.textContent).not.toContain('Navigation 1')
    expect(container.querySelector('[data-tab-chip="tab-one-b"]')).toBeNull()
  })

  it("keeps warnings scoped to the camera's active tab", async () => {
    const incompleteEvents = [
      event(0, 'incomplete-tab', 1, 3),
      event(3_000, 'incomplete-tab', 1, 2),
      event(5_000, 'incomplete-tab', 1, 3),
      ...sessionEvents.filter(({ tabId }) => tabId === 2),
    ]
    const partialReplay = replayData(incompleteEvents, {
      frames: sessionFrames.filter(({ tabId }) => tabId === 1 || tabId === 2),
      tabOrder: [1, 2],
    })
    const secondTab = partialReplay.tabs[1]
    if (secondTab) {
      secondTab.complete = false
      const firstSegment = secondTab.segments[0]
      if (firstSegment) firstSegment.hasGap = true
    }
    replayResult = { ...replayResult, replay: partialReplay }
    await renderReplay()

    expect(container.textContent).toContain(
      'Recording incomplete — playback starts at 0:03',
    )
    expect(container.textContent).not.toContain(
      'this replay contains a known gap',
    )
  })

  it('restarts both the global clock and automatic camera from the beginning', async () => {
    await renderReplay()
    await advanceClock(30_000)
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('60')
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('false')

    await click('[aria-label="Toggle Session playback"]')
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('0')
    expect(viewport().getAttribute('data-active-tab')).toBe('1')
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('true')
  })

  it('keeps a single-tab replay on the global transport without tab controls', async () => {
    const singleTabEvents = sessionEvents.filter(({ tabId }) => tabId === 1)
    replayResult = {
      ...replayResult,
      replay: replayData(singleTabEvents, {
        frames: sessionFrames.filter(({ tabId }) => tabId === 1),
        tabOrder: [1],
      }),
    }
    await renderReplay()

    expect(container.querySelectorAll('[data-tab-chip]')).toHaveLength(0)
    expect(container.textContent).toContain('Following session')
    expect(
      container
        .querySelector('[data-playback-total]')
        ?.getAttribute('data-playback-total'),
    ).toBe('60')
    expect(viewport().getAttribute('data-active-tab')).toBe('1')
  })
})

const { Replay } = await import('./Replay')
