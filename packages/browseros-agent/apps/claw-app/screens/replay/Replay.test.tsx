import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import {
  act,
  createContext,
  type ReactNode,
  StrictMode,
  useContext,
} from 'react'
import type { Root } from 'react-dom/client'
import { MemoryRouter } from 'react-router'
import type { ReplayEvent, ReplayFrame } from '@/modules/api/replay.hooks'
import * as replayDataModule from './replay.data'
import { buildReplayEventCatalog } from './replay-events'
import type { ReplayAction } from './session-replay'
import type { Playback } from './use-playback'

let replayResult: replayDataModule.UseReplayDataResult

mock.module('./replay.data', () => ({
  ...replayDataModule,
  useReplayData: () => replayResult,
}))

mock.module('./ReplayViewport', () => ({
  ReplayViewport: ({
    action,
    url,
    events,
    trackTime,
    live,
    isPlaying,
    speed,
  }: {
    action: ReplayFrame | undefined
    url: string | null
    events: readonly ReplayEvent[]
    trackTime: number
    live: boolean
    isPlaying: boolean
    speed: number
  }) => (
    <div
      data-viewport-tab={events[0]?.tabId ?? -1}
      data-viewport-documents={events
        .map((event) => event.documentId)
        .join(',')}
      data-viewport-track-time={trackTime}
      data-viewport-live={live}
      data-viewport-playing={isPlaying}
      data-viewport-speed={speed}
      data-viewport-url={url ?? ''}
      data-viewport-caption={action?.caption ?? ''}
    />
  ),
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
        data-playback-total={totalSeconds}
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
        {seekTargets.map((seconds) => (
          <button
            type="button"
            key={seconds}
            data-playback-seek={seconds}
            onClick={() => onSeek(seconds)}
          >
            Seek {seconds}
          </button>
        ))}
      </div>
    )
  },
}))

mock.module('./EventTimeline', () => ({
  EventTimeline: ({
    actions,
    currentIndex,
    onSelectAction,
  }: {
    actions: readonly ReplayAction[]
    currentIndex: number
    onSelectAction: (action: ReplayAction) => void
  }) => (
    <div data-event-timeline data-current-index={currentIndex}>
      {actions.map((action, index) => (
        <button
          type="button"
          key={action.frame.dispatchId ?? action.sourceIndex}
          data-action-dispatch={action.frame.dispatchId}
          data-action-start={action.startAt}
          data-action-current={index === currentIndex}
          onClick={() => onSelectAction(action)}
        >
          {action.frame.caption}
        </button>
      ))}
    </div>
  ),
}))

/** Scrub destinations the fake transport exposes as buttons. */
const seekTargets = [0, 3, 9, 14, 16, 24.225, 26.717, 38.321]

const TabSelectContext = createContext<{
  selected: string
  select: (value: string) => void
}>({ selected: '', select: () => {} })

mock.module('@/components/ui/tabs', () => ({
  Tabs: ({
    value,
    onValueChange,
    children,
  }: {
    value: string
    onValueChange: (value: string) => void
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
  }: {
    value: string
    children: ReactNode
    onClick?: () => void
  }) => {
    const { selected, select } = useContext(TabSelectContext)
    return (
      <button
        type="button"
        data-tab-chip={value}
        onClick={() => {
          onClick?.()
          if (value !== selected) select(value)
        }}
      >
        {children}
      </button>
    )
  },
}))

const SESSION_START_MS = 0

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

function toolFrame(
  tabId: number | null,
  atSeconds: number,
  extra: Partial<ReplayFrame> = {},
): ReplayFrame {
  return {
    t: atSeconds,
    kind: 'action',
    verb: 'read',
    node: tabId === null ? 'session' : `Tab ${tabId}`,
    caption: tabId === null ? 'name_session' : `Action on Tab ${tabId}`,
    tabId,
    targetId: tabId === null ? undefined : `target-${tabId}`,
    dispatchId: Math.round(atSeconds * 1000),
    url: tabId === null ? null : `https://tab-${tabId}.example/path`,
    ...extra,
  }
}

function replayData(
  replayEvents: readonly ReplayEvent[],
  tabOrder?: number[],
  replayFrames: ReplayFrame[] = [],
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
    startedAtMs: SESSION_START_MS,
    duration: '0:40',
    tokens: '-',
    steps: String(replayFrames.length),
    totalSeconds: 40,
    frames: replayFrames,
    complete: true,
    tabs,
    eventsLoaded: true,
    eventsForTab: eventCatalog.eventsForTab,
  }
}

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

let root: Root
let container: HTMLElement
let nextFrameId = 1
let frames = new Map<number, FrameRequestCallback>()

beforeEach(async () => {
  nextFrameId = 1
  frames = new Map()
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
      const id = nextFrameId++
      frames.set(id, callback)
      return id
    },
    cancelAnimationFrame: (id: number) => {
      frames.delete(id)
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

/** Drives the global clock to an explicit animation-frame timestamp. */
async function advanceTo(wallMs: number) {
  const pending = [...frames.values()]
  frames.clear()
  await act(async () => {
    for (const callback of pending) callback(wallMs)
  })
}

/** Plays from a standing start to `seconds` of activity at 1x. */
async function playTo(seconds: number) {
  await act(async () => {
    click('[data-playback-speed-option="1"]')
  })
  await advanceTo(0)
  await advanceTo(seconds * 1000)
}

function click(selector: string) {
  const target = container.querySelector(selector)
  if (!target) throw new Error(`missing target: ${selector}`)
  target.dispatchEvent(new window.Event('click', { bubbles: true }))
}

async function clickAct(selector: string) {
  await act(async () => click(selector))
}

function attr(selector: string, name: string): string | null | undefined {
  return container.querySelector(selector)?.getAttribute(name)
}

function viewport(name: string): string | null | undefined {
  return attr('[data-viewport-tab]', `data-viewport-${name}`)
}

function timelineCaptions(): string[] {
  return [...container.querySelectorAll('[data-action-dispatch]')].map(
    (row) => row.textContent ?? '',
  )
}

function currentCaption(): string | undefined {
  return [...container.querySelectorAll('[data-action-dispatch]')].find(
    (row) => row.getAttribute('data-action-current') === 'true',
  )?.textContent
}

describe('Replay auto-follow', () => {
  it('starts on the first playable tab and autoplays global activity', async () => {
    replayResult.replay = replayData(
      [...visualEvents(1, 1_000), ...visualEvents(2, 10_000)],
      [1, 2],
      [toolFrame(1, 3), toolFrame(2, 12)],
    )
    await renderReplay()

    expect(attr('[data-selected-tab]', 'data-selected-tab')).toBe('1')
    expect(viewport('tab')).toBe('1')
    expect(attr('[data-playback-playing]', 'data-playback-playing')).toBe(
      'true',
    )
    // Activity spans the first checkpoint (1s) to the last rrweb event (14s).
    expect(attr('[data-playback-time]', 'data-playback-total')).toBe('13')
    expect(currentCaption()).toBeUndefined()
  })

  it('switches tabs as chronological tool activity crosses the playhead', async () => {
    replayResult.replay = replayData(
      [...visualEvents(1, 1_000), ...visualEvents(2, 10_000)],
      [1, 2],
      [toolFrame(1, 3), toolFrame(2, 12)],
    )
    await renderReplay()
    await playTo(2)

    expect(currentCaption()).toBe('Action on Tab 1')
    expect(viewport('tab')).toBe('1')
    expect(attr('[data-selected-tab]', 'data-selected-tab')).toBe('1')

    await advanceTo(11_000)

    expect(currentCaption()).toBe('Action on Tab 2')
    expect(viewport('tab')).toBe('2')
    expect(attr('[data-selected-tab]', 'data-selected-tab')).toBe('2')
    expect(viewport('url')).toBe('https://tab-2.example/path')
    // The global playhead never restarted for the switch.
    expect(attr('[data-playback-time]', 'data-playback-time')).toBe('11')
    expect(attr('[data-playback-playing]', 'data-playback-playing')).toBe(
      'true',
    )
  })

  it('keeps every tool in one global timeline regardless of tab', async () => {
    replayResult.replay = replayData(
      [...visualEvents(1, 1_000), ...visualEvents(2, 10_000)],
      [1, 2, 3],
      [toolFrame(1, 3), toolFrame(null, 6), toolFrame(3, 8), toolFrame(2, 12)],
    )
    await renderReplay()

    expect(timelineCaptions()).toEqual([
      'Action on Tab 1',
      'name_session',
      'Action on Tab 3',
      'Action on Tab 2',
    ])
  })

  it('keeps an untabbed tool current without moving the camera', async () => {
    replayResult.replay = replayData(
      [...visualEvents(1, 1_000), ...visualEvents(2, 20_000)],
      [1, 2],
      [toolFrame(1, 3), toolFrame(null, 6), toolFrame(2, 22)],
    )
    await renderReplay()
    await playTo(5)

    expect(currentCaption()).toBe('name_session')
    expect(viewport('caption')).toBe('name_session')
    expect(viewport('tab')).toBe('1')
    // An untabbed tool has no URL of its own; the tab's chrome must survive.
    expect(viewport('url')).toBe('https://tab-1.example/path')
  })

  it('keeps a no-visual tab tool current without blanking the viewport', async () => {
    replayResult.replay = replayData(
      [...visualEvents(1, 1_000), noVisualEvent(3, 8_000)],
      [1, 3],
      [toolFrame(1, 3), toolFrame(3, 8)],
    )
    await renderReplay()
    await playTo(7)

    expect(currentCaption()).toBe('Action on Tab 3')
    expect(viewport('tab')).toBe('1')
    expect(container.textContent).not.toContain('No visual recording')
  })

  it('collapses several crossed actions into one switch to the latest tab', async () => {
    replayResult.replay = replayData(
      [
        ...visualEvents(1, 1_000),
        ...visualEvents(2, 6_000),
        ...visualEvents(3, 12_000),
      ],
      [1, 2, 3],
      [toolFrame(1, 3), toolFrame(2, 7), toolFrame(null, 9), toolFrame(3, 14)],
    )
    await renderReplay()
    await playTo(13)

    expect(viewport('tab')).toBe('3')
    expect(currentCaption()).toBe('Action on Tab 3')
  })

  it('resolves the same state seeking backward as forward', async () => {
    replayResult.replay = replayData(
      [...visualEvents(1, 1_000), ...visualEvents(2, 10_000)],
      [1, 2],
      [toolFrame(1, 3), toolFrame(2, 12)],
    )
    await renderReplay()

    await clickAct('[data-playback-seek="9"]')
    const forward = {
      tab: viewport('tab'),
      caption: currentCaption(),
      trackTime: viewport('track-time'),
      live: viewport('live'),
    }

    await clickAct('[data-playback-seek="0"]')
    expect(viewport('tab')).toBe('1')
    expect(currentCaption()).toBeUndefined()

    await clickAct('[data-playback-seek="14"]')
    await clickAct('[data-playback-seek="9"]')

    expect({
      tab: viewport('tab'),
      caption: currentCaption(),
      trackTime: viewport('track-time'),
      live: viewport('live'),
    }).toEqual(forward)
  })

  it('pauses and seeks to an action start when its timeline row is clicked', async () => {
    replayResult.replay = replayData(
      [...visualEvents(1, 1_000), ...visualEvents(2, 10_000)],
      [1, 2],
      [toolFrame(1, 3), toolFrame(2, 12, { durationMs: 2_000 })],
    )
    await renderReplay()
    await clickAct('[data-action-dispatch="12000"]')

    // 12s completion minus a 2s tool, rebased on the 1s activity origin.
    expect(attr('[data-playback-time]', 'data-playback-time')).toBe('9')
    expect(attr('[data-playback-playing]', 'data-playback-playing')).toBe(
      'false',
    )
    expect(viewport('tab')).toBe('2')
    expect(currentCaption()).toBe('Action on Tab 2')
  })
})

describe('Replay manual control', () => {
  const twoTabs = () =>
    replayData(
      [...visualEvents(1, 1_000), ...visualEvents(2, 10_000)],
      [1, 2],
      [toolFrame(1, 3), toolFrame(2, 12)],
    )

  it('pins a clicked tab at the unchanged global time', async () => {
    replayResult.replay = twoTabs()
    await renderReplay()
    await playTo(3)
    expect(viewport('tab')).toBe('1')

    await clickAct('[data-tab-chip="2"]')

    expect(attr('[data-playback-time]', 'data-playback-time')).toBe('3')
    expect(attr('[data-playback-playing]', 'data-playback-playing')).toBe(
      'false',
    )
    expect(viewport('tab')).toBe('2')
    // Tab 2 has not started recording at 3s, so it holds its first frame.
    expect(viewport('track-time')).toBe('0')
    expect(viewport('live')).toBe('false')
    expect(container.querySelector('[data-resume-follow]')).not.toBeNull()
  })

  it('keeps projecting a pinned tab while scrubbing', async () => {
    replayResult.replay = twoTabs()
    await renderReplay()
    await clickAct('[data-tab-chip="2"]')

    // Tab 2 records from 9s to 13s of activity; the pin projects global time
    // into that window and clamps outside it.
    await clickAct('[data-playback-seek="9"]')
    expect(viewport('tab')).toBe('2')
    expect(viewport('track-time')).toBe('0')
    expect(viewport('live')).toBe('true')

    await clickAct('[data-playback-seek="14"]')
    expect(viewport('tab')).toBe('2')
    expect(viewport('track-time')).toBe('4')
    expect(viewport('live')).toBe('false')
    expect(currentCaption()).toBe('Action on Tab 2')
  })

  it('clears the pin on Resume follow without moving the playhead', async () => {
    replayResult.replay = twoTabs()
    await renderReplay()
    await playTo(3)
    await clickAct('[data-tab-chip="2"]')

    await clickAct('[data-resume-follow]')

    expect(attr('[data-playback-time]', 'data-playback-time')).toBe('3')
    expect(viewport('tab')).toBe('1')
    expect(container.querySelector('[data-resume-follow]')).toBeNull()
  })

  it('resumes automatic following when Play is pressed while pinned', async () => {
    replayResult.replay = twoTabs()
    await renderReplay()
    await playTo(3)
    await clickAct('[data-tab-chip="2"]')
    expect(viewport('tab')).toBe('2')

    await clickAct('[data-playback-toggle]')

    expect(attr('[data-playback-playing]', 'data-playback-playing')).toBe(
      'true',
    )
    expect(viewport('tab')).toBe('1')
    expect(container.querySelector('[data-resume-follow]')).toBeNull()
  })

  it('shows the no-recording state for a pinned tab without visuals', async () => {
    replayResult.replay = replayData(
      [...visualEvents(1, 1_000), noVisualEvent(3, 8_000)],
      [1, 3],
      [toolFrame(1, 3), toolFrame(3, 8)],
    )
    await renderReplay()
    await clickAct('[data-tab-chip="3"]')

    expect(container.textContent).toContain('No visual recording for this tab')
    expect(timelineCaptions()).toEqual(['Action on Tab 1', 'Action on Tab 3'])
    expect(container.querySelector('[data-playback-toggle]')).not.toBeNull()
  })

  it('restarts global activity from zero after completion', async () => {
    replayResult.replay = twoTabs()
    await renderReplay()
    await playTo(20)

    expect(attr('[data-playback-time]', 'data-playback-time')).toBe('13')
    expect(attr('[data-playback-toggle]', 'aria-label')).toBe(
      'Restart playback',
    )

    await clickAct('[data-playback-toggle]')

    expect(attr('[data-playback-time]', 'data-playback-time')).toBe('0')
    expect(attr('[data-playback-playing]', 'data-playback-playing')).toBe(
      'true',
    )
    expect(viewport('tab')).toBe('1')
  })

  it('preserves the chosen speed across automatic switches', async () => {
    replayResult.replay = twoTabs()
    await renderReplay()
    await clickAct('[data-playback-speed-option="4"]')
    await advanceTo(0)
    await advanceTo(3_000)

    expect(attr('[data-playback-speed]', 'data-playback-speed')).toBe('4')
    expect(viewport('tab')).toBe('2')
    expect(viewport('speed')).toBe('4')
  })
})

describe('Replay recording states', () => {
  it('merges same-tab navigations into one visual track', async () => {
    const navigationEvents = [
      ...visualEvents(1, 1_000),
      ...visualEvents(1, 10_000).map((event) => ({
        ...event,
        documentId: 'document-1b',
      })),
    ]
    replayResult.replay = replayData(navigationEvents, [1], [toolFrame(1, 12)])
    await renderReplay()

    expect(viewport('documents')).toBe(
      'document-1,document-1,document-1,document-1b,document-1b,document-1b',
    )
    expect(container.querySelector('[data-tab-chip="document-1b"]')).toBeNull()
  })

  it('keeps the incomplete-prefix warning for the visible tab', async () => {
    replayResult.replay = replayData(
      [
        noVisualEvent(1, 1_000),
        ...visualEvents(1, 4_000).slice(1),
        ...visualEvents(2, 20_000),
      ],
      [1, 2],
      [toolFrame(1, 5)],
    )
    await renderReplay()

    expect(container.textContent).toContain(
      'Recording incomplete — playback starts at 0:03',
    )
  })

  it("does not leak another tab's gap into the visible tab", async () => {
    const withGap = replayData(
      [...visualEvents(1, 1_000), ...visualEvents(2, 10_000)],
      [1, 2],
      [toolFrame(1, 3), toolFrame(2, 12)],
    )
    const secondTab = withGap.tabs[1]
    if (!secondTab) throw new Error('tab 2 metadata missing')
    secondTab.complete = false
    const segment = secondTab.segments[0]
    if (segment) segment.hasGap = true
    replayResult.replay = withGap
    await renderReplay()

    expect(container.textContent).not.toContain('Recording incomplete')

    await clickAct('[data-tab-chip="2"]')
    expect(container.textContent).toContain(
      'Recording incomplete — this replay contains a known gap',
    )
  })

  it('offers no transport until a playable recording resolves', async () => {
    const loading = replayData([], [1], [toolFrame(1, 3)])
    loading.eventsLoaded = false
    replayResult.replay = loading
    await renderReplay()

    expect(container.textContent).toContain('No visual recording for this tab')
    expect(container.querySelector('[data-playback-toggle]')).toBeNull()
    expect(timelineCaptions()).toEqual(['Action on Tab 1'])

    replayResult = {
      ...replayResult,
      replay: replayData(visualEvents(1, 1_000), [1], [toolFrame(1, 3)]),
    }
    await renderReplay()

    expect(attr('[data-playback-playing]', 'data-playback-playing')).toBe(
      'true',
    )
    expect(attr('[data-playback-time]', 'data-playback-time')).toBe('0')
  })

  it('extends a live replay without rewinding accepted activity', async () => {
    replayResult.replay = replayData(
      visualEvents(1, 1_000),
      [1],
      [toolFrame(1, 3)],
    )
    await renderReplay()
    await playTo(9)

    expect(attr('[data-playback-time]', 'data-playback-time')).toBe('4')
    expect(attr('[data-playback-playing]', 'data-playback-playing')).toBe(
      'false',
    )

    replayResult = {
      ...replayResult,
      replay: replayData(
        visualEvents(1, 1_000, 10_000),
        [1],
        [toolFrame(1, 3)],
      ),
    }
    await renderReplay()

    expect(attr('[data-playback-time]', 'data-playback-time')).toBe('4')
    expect(attr('[data-playback-total]', 'data-playback-total')).toBe('10')
    expect(attr('[data-playback-playing]', 'data-playback-playing')).toBe(
      'true',
    )
  })

  it('resets the pin and the clock when the session changes', async () => {
    replayResult.replay = replayData(
      [...visualEvents(1, 1_000), ...visualEvents(2, 10_000)],
      [1, 2],
      [toolFrame(1, 3), toolFrame(2, 12)],
    )
    await renderReplay()
    await playTo(3)
    await clickAct('[data-tab-chip="2"]')

    const nextSession = replayData(
      [...visualEvents(2, 1_000), ...visualEvents(3, 10_000)],
      [2, 3],
      [toolFrame(2, 3), toolFrame(3, 12)],
    )
    nextSession.sessionId = 'session-2'
    replayResult = {
      ...replayResult,
      sessionId: 'session-2',
      replay: nextSession,
    }
    await renderReplay()

    expect(container.querySelector('[data-resume-follow]')).toBeNull()
    expect(attr('[data-playback-time]', 'data-playback-time')).toBe('0')
    expect(attr('[data-playback-playing]', 'data-playback-playing')).toBe(
      'true',
    )
    expect(viewport('tab')).toBe('2')
  })

  it('keeps a single-tab session on the same global model', async () => {
    replayResult.replay = replayData(
      visualEvents(1, 1_000),
      [1],
      [toolFrame(1, 3)],
    )
    await renderReplay()

    expect(container.querySelector('[data-selected-tab]')).toBeNull()
    expect(attr('[data-playback-playing]', 'data-playback-playing')).toBe(
      'true',
    )

    await playTo(9)
    expect(attr('[data-playback-time]', 'data-playback-time')).toBe('4')
    expect(viewport('tab')).toBe('1')
  })

  it('cancels the animation loop when the replay unmounts', async () => {
    replayResult.replay = replayData(
      visualEvents(1, 1_000),
      [1],
      [toolFrame(1, 3)],
    )
    await renderReplay()
    await advanceTo(0)
    expect(frames.size).toBe(1)

    await act(async () => root.render(<div data-unmounted />))

    expect(frames.size).toBe(0)
    expect(container.querySelector('[data-unmounted]')).not.toBeNull()
  })
})

describe('Replay Trendshift regression', () => {
  // Tab 1 stops emitting DOM events at 9.003s but keeps receiving tool calls
  // out to 38.321s. Chapter playback ended at 9.003s and could never reach them.
  const TAB_1_VISUAL_END_MS = 9_003
  const trendshift = () =>
    replayData(
      [
        ...visualEvents(1, 0, TAB_1_VISUAL_END_MS),
        ...visualEvents(2, 12_000, 6_000),
      ],
      [1, 2],
      [
        toolFrame(1, 2.053),
        toolFrame(null, 14),
        toolFrame(2, 16),
        toolFrame(1, 24.225),
        toolFrame(1, 26.717),
        toolFrame(1, 38.321),
      ],
    )

  it('reaches every late tool while holding the final recorded frame', async () => {
    replayResult.replay = trendshift()
    await renderReplay()

    expect(attr('[data-playback-time]', 'data-playback-total')).toBe('38.321')

    for (const seconds of [24.225, 26.717, 38.321]) {
      await clickAct(`[data-playback-seek="${seconds}"]`)
      expect(currentCaption()).toBe('Action on Tab 1')
      expect(viewport('tab')).toBe('1')
      expect(viewport('track-time')).toBe(String(TAB_1_VISUAL_END_MS / 1000))
      expect(viewport('live')).toBe('false')
    }
  })

  it('still visits tab 2 in tool order between tab 1 tools', async () => {
    replayResult.replay = trendshift()
    await renderReplay()

    await clickAct('[data-playback-seek="16"]')
    expect(viewport('tab')).toBe('2')
    expect(currentCaption()).toBe('Action on Tab 2')

    await clickAct('[data-playback-seek="24.225"]')
    expect(viewport('tab')).toBe('1')
  })

  it('keeps the untabbed name_session visible and current', async () => {
    replayResult.replay = trendshift()
    await renderReplay()
    await clickAct('[data-playback-seek="14"]')

    expect(currentCaption()).toBe('name_session')
    expect(viewport('caption')).toBe('name_session')
    expect(viewport('tab')).toBe('1')
    expect(timelineCaptions()).toContain('name_session')
  })
})

const { Replay } = await import('./Replay')
