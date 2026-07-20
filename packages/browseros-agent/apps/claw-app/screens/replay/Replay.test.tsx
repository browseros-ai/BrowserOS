import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act, createContext, type ReactNode, useContext } from 'react'
import type { Root } from 'react-dom/client'
import { MemoryRouter } from 'react-router'
import type { ReplayEvent, ReplayFrame } from '@/modules/api/replay.hooks'
import * as replayDataModule from './replay.data'
import { buildReplayEventCatalog } from './replay-events'

let replayResult: replayDataModule.UseReplayDataResult

mock.module('./replay.data', () => ({
  ...replayDataModule,
  useReplayData: () => replayResult,
}))

mock.module('./ReplayViewport', () => ({
  ReplayViewport: ({ events }: { events: readonly ReplayEvent[] }) => (
    <div
      data-player-documents={events.map((event) => event.documentId).join(',')}
      data-player-types={events.map((event) => event.type).join(',')}
    />
  ),
}))

mock.module('./PlaybackTransport', () => ({
  PlaybackTransport: ({
    playback,
  }: {
    playback: { time: number; isPlaying: boolean }
  }) => (
    <div
      data-playback-time={playback.time}
      data-playback-playing={playback.isPlaying}
    />
  ),
}))

mock.module('./EventTimeline', () => ({
  EventTimeline: ({
    frames,
    onSelectFrame,
  }: {
    frames: readonly ReplayFrame[]
    onSelectFrame: (frame: ReplayFrame) => void
  }) => (
    <div>
      {frames.map((frame) => (
        <button
          type="button"
          key={frame.dispatchId}
          data-frame-tab={frame.tabId}
          onClick={() => onSelectFrame(frame)}
        >
          {frame.caption}
        </button>
      ))}
    </div>
  ),
}))

const TargetSelectContext = createContext<(targetId: string) => void>(() => {})

mock.module('@/components/ui/tabs', () => ({
  Tabs: ({
    value,
    onValueChange,
    children,
  }: {
    value: string
    onValueChange: (targetId: string) => void
    children: ReactNode
  }) => (
    <TargetSelectContext.Provider value={onValueChange}>
      <div data-selected-target={value}>{children}</div>
    </TargetSelectContext.Provider>
  ),
  TabsList: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  TabsTrigger: ({
    value,
    children,
  }: {
    value: string
    children: ReactNode
  }) => {
    const selectTarget = useContext(TargetSelectContext)
    return (
      <button
        type="button"
        data-target-chip={value}
        onClick={() => selectTarget(value)}
      >
        {children}
      </button>
    )
  },
}))

const events: ReplayEvent[] = [
  {
    sessionId: 'session-1',
    documentId: 'document-a',
    targetId: 'target-a',
    tabId: 1,
    ts: 1_000,
    type: 4,
    data: { width: 1280, height: 720 },
  },
  {
    sessionId: 'session-1',
    documentId: 'document-a',
    targetId: 'target-a',
    tabId: 1,
    ts: 1_001,
    type: 2,
    data: {},
  },
  {
    sessionId: 'session-1',
    documentId: 'document-a',
    targetId: 'target-a',
    tabId: 1,
    ts: 5_000,
    type: 3,
    data: {},
  },
  {
    sessionId: 'session-1',
    documentId: 'document-b',
    targetId: 'target-b',
    tabId: 2,
    ts: 10_000,
    type: 4,
    data: { width: 1280, height: 720 },
  },
  {
    sessionId: 'session-1',
    documentId: 'document-b',
    targetId: 'target-b',
    tabId: 2,
    ts: 10_001,
    type: 2,
    data: {},
  },
  {
    sessionId: 'session-1',
    documentId: 'document-b',
    targetId: 'target-b',
    tabId: 2,
    ts: 15_000,
    type: 3,
    data: {},
  },
]

const frames: ReplayFrame[] = [
  {
    t: 1,
    kind: 'action',
    verb: 'read',
    node: 'A',
    caption: 'Read target A',
    tabId: 1,
    targetId: 'target-a',
    dispatchId: 1,
  },
  {
    t: 12,
    kind: 'action',
    verb: 'click',
    node: 'B',
    caption: 'Click target B',
    tabId: 2,
    targetId: 'target-b',
    dispatchId: 2,
  },
]

function replayData(replayEvents: readonly ReplayEvent[], tabOrder?: number[]) {
  const eventCatalog = buildReplayEventCatalog(replayEvents)
  const tabs = (tabOrder ?? eventCatalog.tabIds).map((tabId) => ({
    tabId,
    complete: true,
    segments: eventCatalog.documentIdsForTab(tabId).map((documentId) => {
      const documentEvents = eventCatalog.eventsForDocument(documentId)
      return {
        documentId,
        targetId: documentEvents.find((event) => event.targetId)?.targetId,
        firstEventAt: documentEvents[0]?.ts ?? 0,
        lastEventAt: documentEvents.at(-1)?.ts ?? 0,
        hasGap: false,
        legacy: false,
      }
    }),
  }))
  return {
    sessionId: 'session-1',
    agentLabel: 'Codex',
    taskTitle: 'Replay test',
    harness: 'Codex',
    status: 'done' as const,
    site: 'example.com',
    startedAt: 'Jul 18, 2026',
    startedAtMs: 0,
    duration: '0:15',
    tokens: '-',
    steps: '2',
    totalSeconds: 15,
    frames,
    complete: true,
    tabs,
    eventsForDocument: eventCatalog.eventsForDocument,
  }
}

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

let root: Root
let container: HTMLElement

beforeEach(async () => {
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  const globals = {
    window: dom.window,
    document: dom.document,
    navigator: dom.window.navigator,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    Event: dom.window.Event,
  }
  for (const [name, value] of Object.entries(globals)) {
    Object.defineProperty(globalThis, name, {
      configurable: true,
      writable: true,
      value,
    })
  }
  Object.assign(dom.window, {
    requestAnimationFrame: () => 1,
    cancelAnimationFrame: () => undefined,
  })
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })

  replayResult = {
    replay: replayData([]),
    sessionId: 'session-1',
    isLoading: false,
    navigate: mock(() => undefined) as never,
  }
  container = dom.document.getElementById('root') as unknown as HTMLElement
  const { createRoot } = await import('react-dom/client')
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
})

describe('Replay', () => {
  it('discovers logical tabs and switches tab on frame click', async () => {
    await act(async () => {
      root.render(
        <MemoryRouter initialEntries={['/audit/session-1/replay']}>
          <Replay />
        </MemoryRouter>,
      )
    })

    expect(
      [...container.querySelectorAll('[data-target-chip]')].map(
        (chip) => chip.textContent,
      ),
    ).toEqual([])
    expect(container.textContent).toContain('No visual recording for this tab')
    expect(container.querySelector('[data-player-targets]')).toBeNull()

    replayResult = { ...replayResult, replay: replayData(events) }
    await act(async () => {
      root.render(
        <MemoryRouter initialEntries={['/audit/session-1/replay']}>
          <Replay />
        </MemoryRouter>,
      )
    })

    expect(
      container
        .querySelector('[data-player-documents]')
        ?.getAttribute('data-player-documents'),
    ).toBe('document-a,document-a,document-a')
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('true')

    const targetBFrame = container.querySelector('[data-frame-tab="2"]')
    if (!targetBFrame) throw new Error('tab 2 frame missing')
    await act(async () => {
      targetBFrame.dispatchEvent(new window.Event('click', { bubbles: true }))
    })

    expect(
      container
        .querySelector('[data-selected-target]')
        ?.getAttribute('data-selected-target'),
    ).toBe('2')
    expect(
      container
        .querySelector('[data-player-documents]')
        ?.getAttribute('data-player-documents'),
    ).toBe('document-b,document-b,document-b')
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('2')

    replayResult = {
      ...replayResult,
      replay: replayData(events, [1]),
    }
    await act(async () => {
      root.render(
        <MemoryRouter initialEntries={['/audit/session-1/replay']}>
          <Replay />
        </MemoryRouter>,
      )
    })

    expect(
      container
        .querySelector('[data-player-documents]')
        ?.getAttribute('data-player-documents'),
    ).toBe('document-a,document-a,document-a')
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('0')
  })

  it('offers navigation segments without merging their rrweb lifecycles', async () => {
    const navigationEvents: ReplayEvent[] = [
      ...events.slice(0, 3),
      ...events.slice(3).map((candidate) => ({
        ...candidate,
        tabId: 1,
        documentId: 'document-b',
      })),
    ]
    replayResult = {
      ...replayResult,
      replay: replayData(navigationEvents),
    }

    await act(async () => {
      root.render(
        <MemoryRouter initialEntries={['/audit/session-1/replay']}>
          <Replay />
        </MemoryRouter>,
      )
    })

    expect(container.textContent).toContain('Navigation 1')
    expect(container.textContent).toContain('Navigation 2')
    expect(
      container
        .querySelector('[data-player-documents]')
        ?.getAttribute('data-player-documents'),
    ).toBe('document-a,document-a,document-a')

    const secondNavigation = container.querySelector(
      '[data-target-chip="document-b"]',
    )
    if (!secondNavigation) throw new Error('second navigation missing')
    await act(async () => {
      secondNavigation.dispatchEvent(
        new window.Event('click', { bubbles: true }),
      )
    })

    expect(
      container
        .querySelector('[data-player-documents]')
        ?.getAttribute('data-player-documents'),
    ).toBe('document-b,document-b,document-b')
  })

  it('slices orphan mutations and explains where visual playback starts', async () => {
    const incompleteEvents: ReplayEvent[] = [
      {
        sessionId: 'session-1',
        documentId: 'document-a',
        targetId: 'target-a',
        tabId: 1,
        ts: 1_000,
        type: 3,
        data: {},
      },
      {
        sessionId: 'session-1',
        documentId: 'document-a',
        targetId: 'target-a',
        tabId: 1,
        ts: 4_000,
        type: 2,
        data: {},
      },
      {
        sessionId: 'session-1',
        documentId: 'document-a',
        targetId: 'target-a',
        tabId: 1,
        ts: 5_000,
        type: 3,
        data: {},
      },
    ]
    replayResult = {
      ...replayResult,
      replay: replayData(incompleteEvents),
    }

    await act(async () => {
      root.render(
        <MemoryRouter initialEntries={['/audit/session-1/replay']}>
          <Replay />
        </MemoryRouter>,
      )
    })

    expect(
      container
        .querySelector('[data-player-types]')
        ?.getAttribute('data-player-types'),
    ).toBe('2,3')
    expect(container.textContent).toContain(
      'Recording incomplete — playback starts at 0:03',
    )
  })

  it('does not present a cataloged recording gap as complete', async () => {
    const partialReplay = replayData(events.slice(0, 3))
    partialReplay.complete = false
    const firstTab = partialReplay.tabs[0]
    if (firstTab) {
      firstTab.complete = false
      const firstSegment = firstTab.segments[0]
      if (firstSegment) firstSegment.hasGap = true
    }
    replayResult = { ...replayResult, replay: partialReplay }

    await act(async () => {
      root.render(
        <MemoryRouter initialEntries={['/audit/session-1/replay']}>
          <Replay />
        </MemoryRouter>,
      )
    })

    expect(container.textContent).toContain(
      'Recording incomplete — this replay contains a known gap',
    )
  })
})

const { Replay } = await import('./Replay')
