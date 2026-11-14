import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act, StrictMode } from 'react'
import type { Root } from 'react-dom/client'
import type { ReplayEvent, ReplayFrame } from '@/modules/api/replay.hooks'
import { buildSessionReplayPlan } from './session-replay'
import { type Playback, usePlayback } from './use-playback'

interface ReplayerConfig {
  root: HTMLElement
}

type PlayerCall =
  | { kind: 'play' | 'pause'; ms: number | undefined }
  | { kind: 'speed'; speed: number }

interface RecordedEvent {
  timestamp: number
  type: number
  data: { adds?: { node: { textContent: string } }[] }
}

class FakeReplayer {
  readonly marker: HTMLElement
  readonly calls: PlayerCall[] = []
  destroyed = false
  currentTime = 0

  constructor(
    readonly events: RecordedEvent[],
    config: ReplayerConfig,
  ) {
    this.marker = document.createElement('div')
    this.marker.dataset.fakeReplayer = String(fakeReplayers.length + 1)
    config.root.append(this.marker)
    fakeReplayers.push(this)
  }

  pause(ms?: number): void {
    if (ms !== undefined) this.position(ms)
    this.calls.push({ kind: 'pause', ms })
  }

  play(ms?: number): void {
    if (ms !== undefined) this.position(ms)
    this.calls.push({ kind: 'play', ms })
  }

  setConfig(config: { speed: number }): void {
    this.calls.push({ kind: 'speed', speed: config.speed })
  }

  getCurrentTime(): number {
    return this.currentTime
  }

  getMetaData(): { totalTime: number } {
    return {
      totalTime:
        (this.events.at(-1)?.timestamp ?? 0) - (this.events[0]?.timestamp ?? 0),
    }
  }

  // rrweb rebuilds a seek from events strictly before its target timestamp.
  // Reflect DOM additions so endpoint tests assert the rendered state.
  private position(ms: number): void {
    this.currentTime = ms
    this.marker.replaceChildren()
    const baseline = (this.events[0]?.timestamp ?? 0) + ms
    for (const event of this.events) {
      if (event.timestamp >= baseline || event.type !== 3) continue
      for (const addition of event.data.adds ?? []) {
        const node = document.createElement('span')
        node.textContent = addition.node.textContent
        this.marker.append(node)
      }
    }
  }

  destroy(): void {
    this.destroyed = true
    this.marker.remove()
  }
}

const fakeReplayers: FakeReplayer[] = []

mock.module('rrweb', () => ({ Replayer: FakeReplayer }))

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

function tabEvents(tabId: number, documentId: string): ReplayEvent[] {
  return [
    {
      sessionId: 'session-1',
      documentId,
      targetId: `target-${documentId}`,
      tabId,
      ts: 1_000,
      type: 4,
      data: { width: 1280, height: 720 },
    },
    {
      sessionId: 'session-1',
      documentId,
      targetId: `target-${documentId}`,
      tabId,
      ts: 1_001,
      type: 2,
      data: {},
    },
    {
      sessionId: 'session-1',
      documentId,
      tabId,
      ts: 11_000,
      type: 3,
      data: {},
    },
  ]
}

const events = tabEvents(1, 'document-a')

function action(caption: string, url: string | null = null): ReplayFrame {
  return {
    t: 1,
    kind: 'action',
    verb: 'read',
    node: 'node',
    caption,
    url,
  }
}

let root: Root | null
let container: HTMLElement
let playback: Playback
let nextFrameId: number
let animationFrames: Map<number, FrameRequestCallback>

function mutation(ts: number, textContent: string): ReplayEvent {
  return {
    ...events[0],
    ts,
    type: 3,
    data: { source: 0, adds: [{ node: { type: 3, textContent } }] },
  }
}

const finalChangeEvents = [
  ...events.slice(0, 2),
  mutation(1_250, 'Filled'),
  mutation(1_500, 'Middle'),
  mutation(2_001, 'Submitted'),
  mutation(2_001, 'Confirmed'),
]
const finalChangePlan = buildSessionReplayPlan({
  startedAtMs: 1_000,
  frames: [],
  tabs: [{ tabId: 1, complete: true, segments: [] }],
  eventsForTab: () => finalChangeEvents,
})

async function renderPlayback() {
  const { ReplayViewport } = await import('./ReplayViewport')
  function Harness() {
    playback = usePlayback(finalChangePlan.durationSeconds)
    return (
      <ReplayViewport
        site="example.com"
        action={undefined}
        url={null}
        events={finalChangeEvents}
        {...finalChangePlan.project(1, playback.time)}
        isPlaying={playback.isPlaying}
        speed={playback.speed}
      />
    )
  }
  await act(async () => root?.render(<Harness />))
}

async function advanceTo(wallMs: number) {
  const pending = [...animationFrames.values()]
  animationFrames.clear()
  await act(async () => {
    for (const callback of pending) callback(wallMs)
  })
}

function renderedChanges(): (string | null)[] {
  return Array.from(livePlayer().marker.children, (node) => node.textContent)
}

/** Live player, i.e. the one that survived Strict Mode's double invoke. */
function livePlayer(): FakeReplayer {
  const live = fakeReplayers.filter((replayer) => !replayer.destroyed)
  if (live.length !== 1)
    throw new Error(`expected 1 player, got ${live.length}`)
  return live[0] as FakeReplayer
}

interface ViewportProps {
  events?: readonly ReplayEvent[]
  action?: ReplayFrame
  url?: string | null
  trackTime?: number
  live?: boolean
  isPlaying?: boolean
  speed?: number
}

async function render(props: ViewportProps = {}) {
  const { ReplayViewport } = await import('./ReplayViewport')
  await act(async () => {
    root?.render(
      <StrictMode>
        <ReplayViewport
          site="example.com"
          action={props.action}
          url={props.url ?? null}
          events={props.events ?? events}
          trackTime={props.trackTime ?? 0}
          live={props.live ?? true}
          isPlaying={props.isPlaying ?? true}
          speed={props.speed ?? 2}
        />
      </StrictMode>,
    )
  })
}

beforeEach(async () => {
  fakeReplayers.length = 0
  nextFrameId = 1
  animationFrames = new Map()
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
      animationFrames.set(id, callback)
      return id
    },
    cancelAnimationFrame: (id: number) => animationFrames.delete(id),
  })
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })

  container = dom.document.getElementById('root') as unknown as HTMLElement
  const { createRoot } = await import('react-dom/client')
  root = createRoot(container)
})

afterEach(async () => {
  if (root) await act(async () => root?.unmount())
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
})

describe('ReplayViewport', () => {
  for (const speed of [1, 2, 4]) {
    it(`applies the final DOM changes once when playing through and restarting at ${speed}x`, async () => {
      await renderPlayback()
      await act(async () => playback.setSpeed(speed))

      for (let run = 0; run < 2; run++) {
        await act(async () => playback.play())
        expect(renderedChanges()).toEqual([])
        await advanceTo(0)
        await advanceTo(2_000 / speed)

        expect(playback.isPlaying).toBe(false)
        expect(playback.time).toBe(finalChangePlan.durationSeconds)
        expect(renderedChanges()).toEqual([
          'Filled',
          'Middle',
          'Submitted',
          'Confirmed',
        ])
      }
    })
  }

  it('keeps middle seeks strictly before the target event', async () => {
    await renderPlayback()
    await act(async () => playback.seek(0.5))
    expect(renderedChanges()).toEqual(['Filled'])
  })

  it('includes the last DOM changes on an end seek and a return from the middle', async () => {
    await renderPlayback()
    await act(async () => playback.seek(finalChangePlan.durationSeconds))
    expect(renderedChanges()).toEqual([
      'Filled',
      'Middle',
      'Submitted',
      'Confirmed',
    ])

    await act(async () => playback.seek(0.5))
    expect(renderedChanges()).toEqual(['Filled'])

    await act(async () => playback.seek(finalChangePlan.durationSeconds))
    expect(renderedChanges()).toEqual([
      'Filled',
      'Middle',
      'Submitted',
      'Confirmed',
    ])
  })

  it('holds the final DOM changes while later session activity keeps playing', async () => {
    await render({
      events: finalChangeEvents,
      ...finalChangePlan.project(1, 10),
      isPlaying: true,
    })

    expect(renderedChanges()).toEqual([
      'Filled',
      'Middle',
      'Submitted',
      'Confirmed',
    ])
  })

  it('shows final DOM changes when mounting at the end of a zero-duration track', async () => {
    await render({
      events: finalChangeEvents.map((event) => ({ ...event, ts: 1_000 })),
      trackTime: 0,
      live: false,
      isPlaying: false,
    })

    expect(renderedChanges()).toEqual([
      'Filled',
      'Middle',
      'Submitted',
      'Confirmed',
    ])
  })

  it('owns one live Replayer across Strict Mode setup, replacement, and unmount', async () => {
    await render()

    const canvas = container.querySelector('[data-replay-canvas]')
    expect(canvas?.querySelectorAll('[data-fake-replayer]')).toHaveLength(1)
    expect(fakeReplayers).toHaveLength(2)
    expect(
      fakeReplayers.filter((replayer) => !replayer.destroyed),
    ).toHaveLength(1)

    const firstLiveReplayer = livePlayer()
    await render({ events: tabEvents(2, 'document-b') })

    expect(firstLiveReplayer.destroyed).toBe(true)
    expect(canvas?.querySelectorAll('[data-fake-replayer]')).toHaveLength(1)
    expect(
      fakeReplayers.filter((replayer) => !replayer.destroyed),
    ).toHaveLength(1)

    await act(async () => root?.unmount())
    root = null

    expect(fakeReplayers.every((replayer) => replayer.destroyed)).toBe(true)
  })

  it('positions a new player at the supplied time, speed, and play state', async () => {
    await render({ trackTime: 3, speed: 4, isPlaying: true, live: true })

    expect(livePlayer().calls).toEqual([
      { kind: 'speed', speed: 4 },
      { kind: 'play', ms: 3_000 },
    ])
  })

  it('builds a replacement player from the latest projection, not the old one', async () => {
    await render({ trackTime: 1, speed: 2 })
    await render({ trackTime: 8, speed: 4 })

    const before = livePlayer()
    await render({ events: tabEvents(2, 'document-b'), trackTime: 8, speed: 4 })

    expect(before.destroyed).toBe(true)
    expect(livePlayer().calls).toEqual([
      { kind: 'speed', speed: 4 },
      { kind: 'play', ms: 8_000 },
    ])
  })

  it('holds a boundary frame paused while global playback continues', async () => {
    await render({ trackTime: 9.003, live: false, isPlaying: true })

    expect(livePlayer().calls.at(-1)).toEqual({ kind: 'pause', ms: 9_003 })
  })

  it('keeps a clamped track quiet as later global actions arrive', async () => {
    await render({ trackTime: 9.003, live: false, isPlaying: true })
    const player = livePlayer()
    const settled = player.calls.length

    await render({
      trackTime: 9.003,
      live: false,
      isPlaying: true,
      action: action('later tool on this tab'),
    })

    expect(player.calls).toHaveLength(settled)
    expect(container.textContent).toContain('later tool on this tab')
  })

  it('resumes the player when global time re-enters the recorded range', async () => {
    await render({ trackTime: 0, live: false, isPlaying: true })
    const player = livePlayer()
    player.calls.length = 0

    await render({ trackTime: 0.5, live: true, isPlaying: true })

    expect(player.calls).toEqual([
      { kind: 'speed', speed: 2 },
      { kind: 'play', ms: 500 },
    ])
  })

  it('reaches the player with pause, resume, seek, and speed changes', async () => {
    await render({ trackTime: 2 })
    const player = livePlayer()

    player.calls.length = 0
    await render({ trackTime: 2, isPlaying: false })
    expect(player.calls.at(-1)).toEqual({ kind: 'pause', ms: 2_000 })

    player.calls.length = 0
    await render({ trackTime: 6, isPlaying: false })
    expect(player.calls.at(-1)).toEqual({ kind: 'pause', ms: 6_000 })

    player.calls.length = 0
    await render({ trackTime: 6, isPlaying: true })
    expect(player.calls.at(-1)).toEqual({ kind: 'play', ms: 6_000 })

    player.calls.length = 0
    await render({ trackTime: 6, isPlaying: true, speed: 4 })
    expect(player.calls).toEqual([
      { kind: 'speed', speed: 4 },
      { kind: 'play', ms: 6_000 },
    ])
  })

  it('ignores small drift and corrects material drift once', async () => {
    await render({ trackTime: 4, isPlaying: true, live: true })
    const player = livePlayer()

    player.currentTime = 4_100
    player.calls.length = 0
    await render({ trackTime: 4.2, isPlaying: true, live: true })
    expect(player.calls).toEqual([])

    player.currentTime = 1_000
    await render({ trackTime: 4.4, isPlaying: true, live: true })
    expect(player.calls).toEqual([
      { kind: 'speed', speed: 2 },
      { kind: 'play', ms: 4_400 },
    ])

    player.calls.length = 0
    await render({ trackTime: 4.5, isPlaying: true, live: true })
    expect(player.calls).toEqual([])
  })

  it('separates the caption from the address bar', async () => {
    await render({
      action: action('act: click Sign in', null),
      url: 'https://trendshift.io/repositories',
    })

    expect(container.textContent).toContain('act: click Sign in')
    expect(container.textContent).toContain(
      'https://trendshift.io/repositories',
    )

    await render({
      action: action('name_session', null),
      url: 'https://trendshift.io/repositories',
    })

    expect(container.textContent).toContain('name_session')
    expect(container.textContent).toContain(
      'https://trendshift.io/repositories',
    )
  })

  it('falls back to the task site when the tab has no known URL', async () => {
    await render({ url: null })

    expect(container.textContent).toContain('example.com')
  })

  it('shows no caption before the first action becomes current', async () => {
    await render({ action: undefined })

    expect(container.querySelector('[data-replay-canvas]')).not.toBeNull()
    expect(container.querySelector('[data-replay-caption]')).toBeNull()

    await render({ action: action('read: Trendshift') })
    expect(
      container.querySelector('[data-replay-caption]')?.textContent,
    ).toContain('read: Trendshift')
  })
})
