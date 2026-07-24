import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import {
  act,
  createContext,
  type ReactNode,
  StrictMode,
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
let playerTimeMs = 0
let activePlayers = 0
let maxActivePlayers = 0
const playerCalls: Array<{
  kind: 'seek' | 'play' | 'pause' | 'speed'
  tabId: number
  value?: number
}> = []

mock.module('./replay.data', () => ({
  ...replayDataModule,
  useReplayData: () => replayResult,
}))

mock.module('./ReplayViewport', () => ({
  ReplayViewport: ({
    events,
    onPlayerReady,
  }: {
    events: readonly ReplayEvent[]
    onPlayerReady: (
      handle: {
        seek: (ms: number) => void
        play: (ms: number) => void
        pause: () => void
        setSpeed: (speed: number) => void
        getCurrentTime: () => number
      } | null,
    ) => void
  }) => {
    const tabId = events[0]?.tabId ?? -1
    // biome-ignore lint/correctness/useExhaustiveDependencies: event-array replacement remounts the real ReplayViewport player.
    useEffect(() => {
      activePlayers += 1
      maxActivePlayers = Math.max(maxActivePlayers, activePlayers)
      onPlayerReady({
        seek: (ms) => {
          playerTimeMs = ms
          playerCalls.push({ kind: 'seek', tabId, value: ms })
        },
        play: (ms) => {
          playerTimeMs = ms
          playerCalls.push({ kind: 'play', tabId, value: ms })
        },
        pause: () => {
          playerCalls.push({ kind: 'pause', tabId })
        },
        setSpeed: (speed) => {
          playerCalls.push({ kind: 'speed', tabId, value: speed })
        },
        getCurrentTime: () => playerTimeMs,
      })
      return () => {
        activePlayers -= 1
        onPlayerReady(null)
      }
    }, [events, onPlayerReady, tabId])
    return (
      <div
        data-player-documents={events
          .map((event) => event.documentId)
          .join(',')}
        data-player-types={events.map((event) => event.type).join(',')}
      />
    )
  },
}))

mock.module('./PlaybackTransport', () => ({
  PlaybackTransport: ({
    playback,
    totalSeconds,
    onSeek,
  }: {
    playback: Playback
    totalSeconds: number
    onSeek: (seconds: number) => void
  }) => {
    const finished = playback.time >= totalSeconds
    return (
      <div
        data-playback-time={playback.time}
        data-playback-playing={playback.isPlaying}
        data-playback-speed={playback.speed}
      >
        <button
          type="button"
          data-playback-toggle
          aria-label={
            finished
              ? 'Restart playback'
              : playback.isPlaying
                ? 'Pause'
                : 'Play'
          }
          onClick={playback.togglePlay}
        >
          Toggle
        </button>
        {[1, 2, 4].map((speed) => (
          <button
            type="button"
            key={speed}
            data-playback-speed-option={speed}
            onClick={() => playback.setSpeed(speed)}
          >
            {speed}×
          </button>
        ))}
        <button
          type="button"
          data-playback-seek-middle
          onClick={() => onSeek(totalSeconds / 2)}
        >
          Seek middle
        </button>
      </div>
    )
  },
}))

mock.module('./EventTimeline', () => ({
  EventTimeline: ({
    frames,
    onSelectFrame,
  }: {
    frames: readonly ReplayFrame[]
    onSelectFrame: (frame: ReplayFrame) => void
  }) => (
    <div data-event-timeline>
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

const TargetSelectContext = createContext<{
  selectedTarget: string
  selectTarget: (targetId: string) => void
}>({
  selectedTarget: '',
  selectTarget: () => {},
})

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
    <TargetSelectContext.Provider
      value={{ selectedTarget: value, selectTarget: onValueChange }}
    >
      <div data-selected-target={value}>{children}</div>
    </TargetSelectContext.Provider>
  ),
  TabsList: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  TabsTrigger: ({
    value,
    children,
    onClick,
  }: {
    value: string
    children: ReactNode
    onClick?: () => void
  }) => {
    const { selectedTarget, selectTarget } = useContext(TargetSelectContext)
    return (
      <button
        type="button"
        data-target-chip={value}
        onClick={() => {
          onClick?.()
          if (value !== selectedTarget) selectTarget(value)
        }}
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
    url: 'https://first.example/products',
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
    url: 'https://second.example/checkout',
  },
]

function replayData(
  replayEvents: readonly ReplayEvent[],
  tabOrder?: number[],
  replayFrames: ReplayFrame[] = frames,
): replayDataModule.ReplayData {
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
    frames: replayFrames,
    complete: true,
    tabs,
    eventsLoaded: true,
    eventsForTab: eventCatalog.eventsForTab,
  }
}

function visualEvents(
  tabId: number,
  startMs: number,
  durationMs = 4_000,
): ReplayEvent[] {
  const documentId = `document-${tabId}`
  return [
    {
      sessionId: 'session-1',
      documentId,
      targetId: `target-${tabId}`,
      tabId,
      ts: startMs,
      type: 4,
      data: { width: 1280, height: 720 },
    },
    {
      sessionId: 'session-1',
      documentId,
      targetId: `target-${tabId}`,
      tabId,
      ts: startMs + 1,
      type: 2,
      data: {},
    },
    {
      sessionId: 'session-1',
      documentId,
      targetId: `target-${tabId}`,
      tabId,
      ts: startMs + durationMs,
      type: 3,
      data: {},
    },
  ]
}

function staticVisualEvents(tabId: number, atMs: number): ReplayEvent[] {
  const documentId = `document-${tabId}`
  return [
    {
      sessionId: 'session-1',
      documentId,
      targetId: `target-${tabId}`,
      tabId,
      ts: atMs,
      type: 4,
      data: { width: 1280, height: 720 },
    },
    {
      sessionId: 'session-1',
      documentId,
      targetId: `target-${tabId}`,
      tabId,
      ts: atMs,
      type: 2,
      data: {},
    },
  ]
}

function noVisualEvent(tabId: number, atMs: number): ReplayEvent {
  return {
    sessionId: 'session-1',
    documentId: `document-${tabId}`,
    targetId: `target-${tabId}`,
    tabId,
    ts: atMs,
    type: 3,
    data: {},
  }
}

function chapterFrame(
  tabId: number,
  atSeconds: number,
  domain = `tab-${tabId}.example`,
): ReplayFrame {
  return {
    t: atSeconds,
    kind: 'action',
    verb: 'read',
    node: `Tab ${tabId}`,
    caption: `Action on Tab ${tabId}`,
    tabId,
    targetId: `target-${tabId}`,
    dispatchId: tabId,
    url: `https://${domain}/path`,
  }
}

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

let root: Root
let container: HTMLElement
let nextAnimationFrameId = 1
let nextTimerId = 1
const animationFrames = new Map<number, FrameRequestCallback>()
const transitionTimers = new Map<
  number,
  { callback: TimerHandler; delay: number | undefined }
>()
const clearedTimerIds: number[] = []

beforeEach(async () => {
  playerTimeMs = 0
  activePlayers = 0
  maxActivePlayers = 0
  playerCalls.length = 0
  nextAnimationFrameId = 1
  nextTimerId = 1
  animationFrames.clear()
  transitionTimers.clear()
  clearedTimerIds.length = 0
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
    requestAnimationFrame: (callback: FrameRequestCallback) => {
      const id = nextAnimationFrameId++
      animationFrames.set(id, callback)
      return id
    },
    cancelAnimationFrame: (id: number) => {
      animationFrames.delete(id)
    },
    setTimeout: (callback: TimerHandler, delay?: number) => {
      const id = nextTimerId++
      transitionTimers.set(id, { callback, delay })
      return id
    },
    clearTimeout: (id: number) => {
      clearedTimerIds.push(id)
      transitionTimers.delete(id)
    },
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

async function renderReplay() {
  await act(async () => {
    root.render(
      <StrictMode>
        <MemoryRouter initialEntries={['/audit/session-1/replay']}>
          <Replay />
        </MemoryRouter>
      </StrictMode>,
    )
  })
}

async function completeCurrentChapter(totalMs: number) {
  playerTimeMs = totalMs
  const callbacks = [...animationFrames.values()]
  animationFrames.clear()
  await act(async () => {
    for (const callback of callbacks) callback(performance.now())
  })
}

async function fireNextTransition() {
  const entry = transitionTimers.entries().next().value as
    | [number, { callback: TimerHandler; delay: number | undefined }]
    | undefined
  if (!entry) throw new Error('transition timer missing')
  transitionTimers.delete(entry[0])
  await act(async () => {
    if (typeof entry[1].callback === 'function') entry[1].callback()
  })
}

async function click(selector: string) {
  const target = container.querySelector(selector)
  if (!target) throw new Error(`missing target: ${selector}`)
  await act(async () => {
    target.dispatchEvent(new window.Event('click', { bubbles: true }))
  })
}

describe('Replay', () => {
  it('discovers logical tabs and keeps frame selection tab-local', async () => {
    await renderReplay()

    expect(
      [...container.querySelectorAll('[data-target-chip]')].map(
        (chip) => chip.textContent,
      ),
    ).toEqual([])
    expect(container.textContent).toContain('No visual recording for this tab')
    expect(container.querySelector('[data-player-targets]')).toBeNull()

    replayResult = { ...replayResult, replay: replayData(events) }
    await renderReplay()

    expect(
      container
        .querySelector('[data-player-documents]')
        ?.getAttribute('data-player-documents'),
    ).toBe('document-a,document-a,document-a')
    expect(
      [...container.querySelectorAll('[data-target-chip]')].map(
        (chip) => chip.textContent,
      ),
    ).toEqual(['Tab 1', 'Tab 2'])
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('true')

    expect(container.querySelector('[data-frame-tab="2"]')).toBeNull()
    await click('[data-target-chip="2"]')

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
    ).toBe('0')

    const targetBFrame = container.querySelector('[data-frame-tab="2"]')
    if (!targetBFrame) throw new Error('tab 2 frame missing')
    await act(async () => {
      targetBFrame.dispatchEvent(new window.Event('click', { bubbles: true }))
    })
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('2')

    replayResult = {
      ...replayResult,
      replay: replayData(events, [1]),
    }
    await renderReplay()

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

  it('merges navigation lifecycles into one tab-level replay', async () => {
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

    await renderReplay()

    expect(container.textContent).not.toContain('Navigation 1')
    expect(container.textContent).not.toContain('Navigation 2')
    expect(
      container
        .querySelector('[data-player-documents]')
        ?.getAttribute('data-player-documents'),
    ).toBe('document-a,document-a,document-a,document-b,document-b,document-b')
    expect(
      container.querySelector('[data-target-chip="document-b"]'),
    ).toBeNull()
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

    await renderReplay()

    expect(
      container
        .querySelector('[data-player-types]')
        ?.getAttribute('data-player-types'),
    ).toBe('2,3')
    expect(container.textContent).toContain(
      'Recording incomplete — playback starts at 0:03',
    )
  })

  it("keeps the selected tab's prefix warning when another tab has a gap", async () => {
    const partialReplay = replayData([
      { ...events[0], ts: 0, type: 3 },
      ...events,
    ])
    partialReplay.complete = false
    const secondTab = partialReplay.tabs[1]
    if (secondTab) {
      secondTab.complete = false
      const firstSegment = secondTab.segments[0]
      if (firstSegment) firstSegment.hasGap = true
    }
    replayResult = { ...replayResult, replay: partialReplay }

    await renderReplay()

    expect(container.textContent).toContain(
      'Recording incomplete — playback starts at 0:01',
    )
    expect(container.textContent).not.toContain(
      'this replay contains a known gap',
    )
  })

  it("does not leak another tab's gap into the selected tab", async () => {
    const partialReplay = replayData(events)
    partialReplay.complete = false
    const secondTab = partialReplay.tabs[1]
    if (secondTab) {
      secondTab.complete = false
      const firstSegment = secondTab.segments[0]
      if (firstSegment) firstSegment.hasGap = true
    }
    replayResult = { ...replayResult, replay: partialReplay }

    await renderReplay()

    expect(container.textContent).not.toContain('Recording incomplete')
  })

  it('plays each chapter fully, waits one wall-clock second, and preserves speed', async () => {
    const chapterEvents = [
      ...visualEvents(1, 1_000),
      ...visualEvents(2, 10_000, 5_000),
    ]
    replayResult = {
      ...replayResult,
      replay: replayData(
        chapterEvents,
        [1, 2],
        [
          chapterFrame(1, 2, 'first.example'),
          chapterFrame(2, 12, 'second.example'),
        ],
      ),
    }
    await renderReplay()

    expect(container.textContent).toContain('Tab 1 of 2')
    expect(container.querySelector('[data-event-timeline]')?.textContent).toBe(
      'Action on Tab 1',
    )
    await click('[data-playback-speed-option="4"]')
    await completeCurrentChapter(4_000)

    expect(container.textContent).toContain(
      'Tab 1 complete → Opening Tab 2 · second.example',
    )
    expect(
      container
        .querySelector('[data-selected-target]')
        ?.getAttribute('data-selected-target'),
    ).toBe('2')
    expect(container.textContent).toContain('Tab 2 of 2')
    expect(container.querySelector('[data-event-timeline]')?.textContent).toBe(
      'Action on Tab 2',
    )
    expect([...transitionTimers.values()].map(({ delay }) => delay)).toEqual([
      1_000,
    ])
    expect(
      playerCalls.some(
        (call) => call.kind === 'play' && call.tabId === 2 && call.value === 0,
      ),
    ).toBe(false)

    await fireNextTransition()

    expect(container.textContent).not.toContain('Tab 1 complete')
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('true')
    expect(
      container
        .querySelector('[data-playback-speed]')
        ?.getAttribute('data-playback-speed'),
    ).toBe('4')
    expect(
      playerCalls.some(
        (call) => call.kind === 'play' && call.tabId === 2 && call.value === 0,
      ),
    ).toBe(true)
    expect(maxActivePlayers).toBe(1)
  })

  it('changes the warning, viewport, timeline, and local clock as one chapter', async () => {
    const chapterEvents = [
      ...visualEvents(1, 1_000),
      ...visualEvents(2, 10_000),
    ]
    const chapterReplay = replayData(
      chapterEvents,
      [1, 2],
      [chapterFrame(1, 2), chapterFrame(2, 11)],
    )
    const secondTab = chapterReplay.tabs[1]
    if (!secondTab) throw new Error('tab 2 metadata missing')
    secondTab.complete = false
    const secondSegment = secondTab.segments[0]
    if (!secondSegment) throw new Error('tab 2 segment missing')
    secondSegment.hasGap = true
    replayResult = { ...replayResult, replay: chapterReplay }
    await renderReplay()

    expect(container.textContent).not.toContain('Recording incomplete')
    await completeCurrentChapter(4_000)

    expect(container.textContent).toContain('Tab 2 of 2')
    expect(container.textContent).toContain(
      'Recording incomplete — this replay contains a known gap',
    )
    expect(
      container
        .querySelector('[data-player-documents]')
        ?.getAttribute('data-player-documents'),
    ).toBe('document-2,document-2,document-2')
    expect(container.querySelector('[data-event-timeline]')?.textContent).toBe(
      'Action on Tab 2',
    )
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('0')
  })

  it('summarizes consecutive middle and trailing no-visual chapters', async () => {
    const chapterEvents = [
      ...visualEvents(1, 1_000),
      noVisualEvent(2, 6_000),
      noVisualEvent(3, 7_000),
      ...visualEvents(4, 10_000),
      noVisualEvent(5, 16_000),
      noVisualEvent(6, 17_000),
    ]
    replayResult = {
      ...replayResult,
      replay: replayData(
        chapterEvents,
        [1, 2, 3, 4, 5, 6],
        [
          chapterFrame(1, 2),
          chapterFrame(2, 6),
          chapterFrame(3, 7),
          chapterFrame(4, 11, 'fourth.example'),
          chapterFrame(5, 16),
          chapterFrame(6, 17),
        ],
      ),
    }
    await renderReplay()

    await completeCurrentChapter(4_000)
    expect(container.textContent).toContain(
      'Tab 1 complete → Opening Tab 4 · fourth.example',
    )
    expect(container.textContent).toContain(
      'Skipped Tabs 2–3 — no visual recordings',
    )
    expect(container.textContent).toContain('Tab 4 of 6')

    await fireNextTransition()
    await completeCurrentChapter(4_000)
    expect(container.textContent).toContain('Tab 4 complete → Replay complete')
    expect(container.textContent).toContain(
      'Skipped Tabs 5–6 — no visual recordings',
    )
    expect(container.textContent).not.toContain(
      'Replay completeReplay complete',
    )

    await fireNextTransition()
    expect(container.textContent).toContain('Replay complete')
    expect(transitionTimers.size).toBe(0)
  })

  it('explains leading skipped chapters before autoplaying the first playable tab', async () => {
    const chapterEvents = [
      noVisualEvent(1, 1_000),
      noVisualEvent(2, 2_000),
      ...visualEvents(3, 3_000),
    ]
    replayResult = {
      ...replayResult,
      replay: replayData(
        chapterEvents,
        [1, 2, 3],
        [
          chapterFrame(1, 1),
          chapterFrame(2, 2),
          chapterFrame(3, 3, 'playable.example'),
        ],
      ),
    }
    await renderReplay()

    expect(container.textContent).toContain(
      'Starting replay → Opening Tab 3 · playable.example',
    )
    expect(container.textContent).toContain(
      'Skipped Tabs 1–2 — no visual recordings',
    )
    expect(container.textContent).toContain('Tab 3 of 3')
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('false')

    await fireNextTransition()
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('true')
  })

  it('waits for event loading before completing an all-no-visual replay', async () => {
    const noVisualReplay = replayData(
      [noVisualEvent(1, 1_000), noVisualEvent(2, 2_000)],
      [1, 2],
      [chapterFrame(1, 1), chapterFrame(2, 2)],
    )
    noVisualReplay.eventsLoaded = false
    replayResult = { ...replayResult, replay: noVisualReplay }
    await renderReplay()

    expect(container.textContent).toContain('Tab 1 of 2')
    expect(container.textContent).not.toContain('No visual recordings')
    expect(transitionTimers.size).toBe(0)

    noVisualReplay.eventsLoaded = true
    replayResult = { ...replayResult, replay: { ...noVisualReplay } }
    await renderReplay()

    expect(container.textContent).toContain(
      'No visual recordings → Replay complete',
    )
    expect(container.textContent).toContain(
      'Skipped Tabs 1–2 — no visual recordings',
    )
    expect([...transitionTimers.values()][0]?.delay).toBe(1_000)

    await fireNextTransition()
    expect(container.textContent).toContain('Replay complete')
    expect(container.querySelector('[data-playback-toggle]')).toBeNull()
  })

  it('does not complete an all-no-visual replay while the session is live', async () => {
    const liveReplay = replayData(
      [noVisualEvent(1, 1_000)],
      [1],
      [chapterFrame(1, 1)],
    )
    liveReplay.status = 'running'
    replayResult = { ...replayResult, replay: liveReplay }
    await renderReplay()

    expect(container.textContent).not.toContain(
      'No visual recordings → Replay complete',
    )
    expect(container.textContent).not.toContain('Replay complete')
    expect(transitionTimers.size).toBe(0)
  })

  it('autoplays when the first playable snapshot arrives after metadata', async () => {
    const loadingReplay = replayData(
      [],
      [1],
      [chapterFrame(1, 1, 'first.example')],
    )
    loadingReplay.eventsLoaded = false
    replayResult = { ...replayResult, replay: loadingReplay }
    await renderReplay()

    expect(container.textContent).toContain('No visual recording for this tab')

    replayResult = {
      ...replayResult,
      replay: replayData(
        visualEvents(1, 1_000),
        [1],
        [chapterFrame(1, 1, 'first.example')],
      ),
    }
    await renderReplay()

    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('true')
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('0')
  })

  it('preserves a manual chapter choice while visual events are loading', async () => {
    const loadingReplay = replayData(
      [],
      [1, 2],
      [chapterFrame(1, 1), chapterFrame(2, 2)],
    )
    loadingReplay.eventsLoaded = false
    replayResult = { ...replayResult, replay: loadingReplay }
    await renderReplay()
    await click('[data-target-chip="2"]')

    replayResult = {
      ...replayResult,
      replay: replayData(
        [...visualEvents(1, 1_000), ...visualEvents(2, 10_000)],
        [1, 2],
        [chapterFrame(1, 1), chapterFrame(2, 11)],
      ),
    }
    await renderReplay()

    expect(
      container
        .querySelector('[data-selected-target]')
        ?.getAttribute('data-selected-target'),
    ).toBe('2')
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('false')
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('0')
  })

  it('cancels an empty-replay transition when a playable snapshot arrives', async () => {
    const emptyReplay = replayData(
      [noVisualEvent(1, 1_000)],
      [1],
      [chapterFrame(1, 1, 'first.example')],
    )
    replayResult = { ...replayResult, replay: emptyReplay }
    await renderReplay()

    const staleTransition = [...transitionTimers.values()][0]?.callback
    expect(container.textContent).toContain(
      'No visual recordings → Replay complete',
    )

    replayResult = {
      ...replayResult,
      replay: replayData(
        visualEvents(1, 1_000),
        [1],
        [chapterFrame(1, 1, 'first.example')],
      ),
    }
    await renderReplay()

    expect(transitionTimers.size).toBe(0)
    expect(container.textContent).not.toContain('No visual recordings')
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('true')

    await act(async () => {
      if (typeof staleTransition === 'function') staleTransition()
    })
    expect(container.textContent).not.toContain('Replay complete')
  })

  it('starts a playable snapshot that arrives after empty-replay completion', async () => {
    replayResult = {
      ...replayResult,
      replay: replayData(
        [noVisualEvent(1, 1_000), noVisualEvent(2, 2_000)],
        [1, 2],
        [chapterFrame(1, 1), chapterFrame(2, 2, 'second.example')],
      ),
    }
    await renderReplay()
    await fireNextTransition()
    expect(container.textContent).toContain('Replay complete')

    replayResult = {
      ...replayResult,
      replay: replayData(
        [noVisualEvent(1, 1_000), ...visualEvents(2, 2_000)],
        [1, 2],
        [chapterFrame(1, 1), chapterFrame(2, 2, 'second.example')],
      ),
    }
    await renderReplay()

    expect(container.textContent).not.toContain('Replay complete')
    expect(container.textContent).toContain(
      'Starting replay → Opening Tab 2 · second.example',
    )
    expect(
      container
        .querySelector('[data-selected-target]')
        ?.getAttribute('data-selected-target'),
    ).toBe('2')
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('false')

    await fireNextTransition()
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('true')
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('0')
  })

  it('continues into a later chapter that becomes playable after completion', async () => {
    replayResult = {
      ...replayResult,
      replay: replayData(
        [...visualEvents(1, 1_000), noVisualEvent(2, 7_000)],
        [1, 2],
        [chapterFrame(1, 2), chapterFrame(2, 7, 'second.example')],
      ),
    }
    await renderReplay()
    await completeCurrentChapter(4_000)
    await fireNextTransition()
    expect(container.textContent).toContain('Replay complete')

    replayResult = {
      ...replayResult,
      replay: replayData(
        [...visualEvents(1, 1_000), ...visualEvents(2, 10_000)],
        [1, 2],
        [chapterFrame(1, 2), chapterFrame(2, 11, 'second.example')],
      ),
    }
    await renderReplay()

    expect(container.textContent).not.toContain('Replay complete')
    expect(container.textContent).toContain(
      'Tab 1 complete → Opening Tab 2 · second.example',
    )
    expect(
      container
        .querySelector('[data-selected-target]')
        ?.getAttribute('data-selected-target'),
    ).toBe('2')

    await fireNextTransition()
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('true')
  })

  it("resumes when the completed chapter's recording grows", async () => {
    replayResult = {
      ...replayResult,
      replay: replayData(visualEvents(1, 1_000), [1], [chapterFrame(1, 2)]),
    }
    await renderReplay()
    await completeCurrentChapter(4_000)
    expect(container.textContent).toContain('Replay complete')

    replayResult = {
      ...replayResult,
      replay: replayData(
        visualEvents(1, 1_000, 6_000),
        [1],
        [chapterFrame(1, 2)],
      ),
    }
    await renderReplay()

    expect(container.textContent).not.toContain('Replay complete')
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('4')
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('true')

    await completeCurrentChapter(6_000)
    expect(container.textContent).toContain('Replay complete')
  })

  it('waits for the current event revision before finalizing the sequence', async () => {
    const refreshingReplay = replayData(
      visualEvents(1, 1_000),
      [1],
      [chapterFrame(1, 2)],
    )
    refreshingReplay.eventsLoaded = false
    replayResult = { ...replayResult, replay: refreshingReplay }
    await renderReplay()
    await completeCurrentChapter(4_000)

    expect(container.textContent).not.toContain('Replay complete')

    replayResult = {
      ...replayResult,
      replay: { ...refreshingReplay, eventsLoaded: true },
    }
    await renderReplay()

    expect(container.textContent).toContain('Replay complete')
  })

  it('plays a static visual snapshot before advancing', async () => {
    replayResult = {
      ...replayResult,
      replay: replayData(
        [...staticVisualEvents(1, 1_000), ...visualEvents(2, 10_000)],
        [1, 2],
        [chapterFrame(1, 1), chapterFrame(2, 11)],
      ),
    }
    await renderReplay()

    expect(container.querySelector('[data-player-types]')).not.toBeNull()
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('true')

    await completeCurrentChapter(0)
    expect(container.textContent).toContain(
      'Tab 1 complete → Opening Tab 2 · tab-2.example',
    )
  })

  it('resets selection, transport, and stale transitions when the session changes', async () => {
    replayResult = {
      ...replayResult,
      replay: replayData(
        [...visualEvents(1, 1_000), ...visualEvents(2, 10_000)],
        [1, 2],
        [chapterFrame(1, 2), chapterFrame(2, 11)],
      ),
    }
    await renderReplay()
    await completeCurrentChapter(4_000)
    const staleTransition = [...transitionTimers.values()][0]?.callback

    const nextSession = replayData(
      [...visualEvents(2, 1_000), ...visualEvents(3, 10_000)],
      [2, 3],
      [chapterFrame(2, 2), chapterFrame(3, 11)],
    )
    nextSession.sessionId = 'session-2'
    replayResult = {
      ...replayResult,
      sessionId: 'session-2',
      replay: nextSession,
    }
    await renderReplay()

    expect(transitionTimers.size).toBe(0)
    expect(container.textContent).not.toContain('Tab 1 complete')
    expect(container.textContent).not.toContain('Replay complete')
    expect(container.textContent).toContain('Tab 1 of 2')
    expect(
      container
        .querySelector('[data-selected-target]')
        ?.getAttribute('data-selected-target'),
    ).toBe('2')
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('0')
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('true')

    await act(async () => {
      if (typeof staleTransition === 'function') staleTransition()
    })
    expect(
      container
        .querySelector('[data-selected-target]')
        ?.getAttribute('data-selected-target'),
    ).toBe('2')
  })

  it('cancels a pending transition and invalidates its callback on manual selection', async () => {
    const chapterEvents = [
      ...visualEvents(1, 1_000),
      ...visualEvents(2, 10_000),
      ...visualEvents(3, 20_000),
    ]
    replayResult = {
      ...replayResult,
      replay: replayData(
        chapterEvents,
        [1, 2, 3],
        [chapterFrame(1, 2), chapterFrame(2, 11), chapterFrame(3, 21)],
      ),
    }
    await renderReplay()
    await completeCurrentChapter(4_000)

    const pending = [...transitionTimers.values()][0]?.callback
    await click('[data-target-chip="3"]')

    expect(transitionTimers.size).toBe(0)
    expect(clearedTimerIds).toHaveLength(1)
    expect(container.textContent).not.toContain('Tab 1 complete')
    expect(
      container
        .querySelector('[data-selected-target]')
        ?.getAttribute('data-selected-target'),
    ).toBe('3')
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('0')
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('false')

    await act(async () => {
      if (typeof pending === 'function') pending()
    })
    expect(
      container
        .querySelector('[data-selected-target]')
        ?.getAttribute('data-selected-target'),
    ).toBe('3')
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('false')
  })

  it('lets the active destination chip cancel its transition and continue forward', async () => {
    replayResult = {
      ...replayResult,
      replay: replayData(
        [
          ...visualEvents(1, 1_000),
          ...visualEvents(2, 10_000),
          ...visualEvents(3, 20_000),
        ],
        [1, 2, 3],
        [chapterFrame(1, 2), chapterFrame(2, 11), chapterFrame(3, 21)],
      ),
    }
    await renderReplay()
    await completeCurrentChapter(4_000)

    expect(container.textContent).toContain('Tab 1 complete')
    await click('[data-target-chip="2"]')

    expect(transitionTimers.size).toBe(0)
    expect(container.textContent).not.toContain('Tab 1 complete')
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('false')
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('0')

    await click('[data-playback-toggle]')
    await completeCurrentChapter(4_000)
    expect(container.textContent).toContain(
      'Tab 2 complete → Opening Tab 3 · tab-3.example',
    )
  })

  it('keeps a manually selected no-visual chapter paused and tab-local', async () => {
    const chapterEvents = [
      ...visualEvents(1, 1_000),
      noVisualEvent(2, 7_000),
      ...visualEvents(3, 10_000),
    ]
    replayResult = {
      ...replayResult,
      replay: replayData(
        chapterEvents,
        [1, 2, 3],
        [chapterFrame(1, 2), chapterFrame(2, 7), chapterFrame(3, 11)],
      ),
    }
    await renderReplay()
    await click('[data-target-chip="2"]')

    expect(container.textContent).toContain('No visual recording for this tab')
    expect(container.textContent).toContain('Tab 2 of 3')
    expect(container.querySelector('[data-event-timeline]')?.textContent).toBe(
      'Action on Tab 2',
    )
    expect(container.querySelector('[data-playback-toggle]')).toBeNull()
    expect(transitionTimers.size).toBe(0)
  })

  it('restarts a completed sequence from the first playable tab', async () => {
    const chapterEvents = [
      ...visualEvents(1, 1_000),
      ...visualEvents(2, 10_000),
    ]
    replayResult = {
      ...replayResult,
      replay: replayData(
        chapterEvents,
        [1, 2],
        [chapterFrame(1, 2), chapterFrame(2, 11)],
      ),
    }
    await renderReplay()
    await click('[data-playback-speed-option="4"]')
    await completeCurrentChapter(4_000)
    await fireNextTransition()
    await completeCurrentChapter(4_000)

    expect(container.textContent).toContain('Replay complete')
    expect(
      container
        .querySelector('[data-playback-toggle]')
        ?.getAttribute('aria-label'),
    ).toBe('Restart playback')

    await click('[data-playback-toggle]')

    expect(
      container
        .querySelector('[data-selected-target]')
        ?.getAttribute('data-selected-target'),
    ).toBe('1')
    expect(container.textContent).toContain('Tab 1 of 2')
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('0')
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('true')
    expect(
      container
        .querySelector('[data-playback-speed]')
        ?.getAttribute('data-playback-speed'),
    ).toBe('4')
  })

  it('keeps a single playable tab unchanged apart from chapter progress', async () => {
    replayResult = {
      ...replayResult,
      replay: replayData(visualEvents(1, 1_000), [1], [chapterFrame(1, 2)]),
    }
    await renderReplay()

    expect(container.textContent).toContain('Tab 1 of 1')
    expect(container.querySelector('[data-selected-target]')).toBeNull()
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('true')

    await completeCurrentChapter(4_000)
    expect(container.textContent).toContain('Replay complete')
    expect(transitionTimers.size).toBe(0)
  })

  it('resumes from a manual seek after sequence completion', async () => {
    replayResult = {
      ...replayResult,
      replay: replayData(visualEvents(1, 1_000), [1], [chapterFrame(1, 2)]),
    }
    await renderReplay()
    await completeCurrentChapter(4_000)
    expect(container.textContent).toContain('Replay complete')

    await click('[data-playback-seek-middle]')

    expect(container.textContent).not.toContain('Replay complete')
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('2')
    expect(
      container
        .querySelector('[data-playback-toggle]')
        ?.getAttribute('aria-label'),
    ).toBe('Play')

    await click('[data-playback-toggle]')
    expect(
      container
        .querySelector('[data-playback-time]')
        ?.getAttribute('data-playback-time'),
    ).toBe('2')
    expect(
      container
        .querySelector('[data-playback-playing]')
        ?.getAttribute('data-playback-playing'),
    ).toBe('true')
  })

  it('clears the transition timer when the replay unmounts', async () => {
    replayResult = {
      ...replayResult,
      replay: replayData(
        [...visualEvents(1, 1_000), ...visualEvents(2, 10_000)],
        [1, 2],
        [chapterFrame(1, 2), chapterFrame(2, 11)],
      ),
    }
    await renderReplay()
    await completeCurrentChapter(4_000)
    const pending = [...transitionTimers.values()][0]?.callback

    await act(async () => root.render(<div data-unmounted />))

    expect(transitionTimers.size).toBe(0)
    expect(clearedTimerIds).toHaveLength(1)
    await act(async () => {
      if (typeof pending === 'function') pending()
    })
    expect(container.querySelector('[data-unmounted]')).not.toBeNull()
  })
})

const { Replay } = await import('./Replay')
