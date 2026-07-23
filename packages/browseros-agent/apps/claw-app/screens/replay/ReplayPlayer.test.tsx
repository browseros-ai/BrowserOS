/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act, StrictMode } from 'react'
import type { Root } from 'react-dom/client'
import type { ReplayEvent } from '@/modules/api/replay.hooks'
import type { ReplayPlayerHandle } from './ReplayPlayer'

interface ReplayerConfig {
  root: HTMLElement
}

class FakeResizeObserver {
  disconnected = false

  constructor(_callback: ResizeObserverCallback) {
    fakeResizeObservers.push(this)
  }

  observe(_target: Element): void {}

  disconnect(): void {
    this.disconnected = true
  }
}

class FakeReplayer {
  readonly marker: HTMLElement
  readonly pauseCalls: number[] = []
  readonly playCalls: number[] = []
  readonly speeds: number[] = []
  destroyed = false

  constructor(_events: unknown[], config: ReplayerConfig) {
    this.marker = document.createElement('div')
    this.marker.dataset.fakeReplayer = String(fakeReplayers.length + 1)
    this.marker.className = 'replayer-wrapper'
    config.root.append(this.marker)
    fakeReplayers.push(this)
  }

  pause(ms?: number): void {
    if (ms !== undefined) this.pauseCalls.push(ms)
  }

  play(ms = 0): void {
    this.playCalls.push(ms)
  }

  setConfig({ speed }: { speed: number }): void {
    this.speeds.push(speed)
  }

  getCurrentTime(): number {
    return 321
  }

  destroy(): void {
    this.destroyed = true
    this.marker.remove()
  }
}

const fakeReplayers: FakeReplayer[] = []
const fakeResizeObservers: FakeResizeObserver[] = []

mock.module('rrweb', () => ({ Replayer: FakeReplayer }))

const globalDescriptors = new Map(
  [
    'window',
    'document',
    'navigator',
    'HTMLElement',
    'Node',
    'Event',
    'ResizeObserver',
  ].map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]),
)

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
]

let root: Root | null
let container: HTMLElement

beforeEach(async () => {
  fakeReplayers.length = 0
  fakeResizeObservers.length = 0
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
    ResizeObserver: FakeResizeObserver,
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

describe('ReplayPlayer', () => {
  it('owns one live Replayer across Strict Mode setup, replacement, and unmount', async () => {
    const readyValues: Array<ReplayPlayerHandle | null> = []
    const { ReplayPlayer } = await import('./ReplayPlayer')

    await act(async () => {
      root?.render(
        <StrictMode>
          <ReplayPlayer
            events={events}
            onReady={(handle) => readyValues.push(handle)}
          />
        </StrictMode>,
      )
    })

    expect(fakeReplayers).toHaveLength(2)
    expect(
      fakeReplayers.filter((replayer) => !replayer.destroyed),
    ).toHaveLength(1)
    expect(readyValues.at(-1)).not.toBeNull()

    const firstLiveReplayer = fakeReplayers.find(
      (replayer) => !replayer.destroyed,
    )
    const replacementEvents = events.map((event) => ({
      ...event,
      documentId: 'document-b',
    }))
    await act(async () => {
      root?.render(
        <StrictMode>
          <ReplayPlayer
            events={replacementEvents}
            onReady={(handle) => readyValues.push(handle)}
          />
        </StrictMode>,
      )
    })

    expect(firstLiveReplayer?.destroyed).toBe(true)
    expect(
      fakeReplayers.filter((replayer) => !replayer.destroyed),
    ).toHaveLength(1)

    await act(async () => root?.unmount())
    root = null
    expect(fakeReplayers.every((replayer) => replayer.destroyed)).toBe(true)
    expect(fakeResizeObservers.every((observer) => observer.disconnected)).toBe(
      true,
    )
    expect(readyValues.at(-1)).toBeNull()
  })

  it('exposes clamped seek, play, pause, speed, and current-time controls', async () => {
    let handle: ReplayPlayerHandle | null = null
    const { ReplayPlayer } = await import('./ReplayPlayer')
    await act(async () => {
      root?.render(
        <ReplayPlayer
          events={events}
          onReady={(nextHandle) => {
            handle = nextHandle
          }}
        />,
      )
    })
    if (!handle) throw new Error('expected ReplayPlayer handle')

    handle.seek(0)
    handle.play(50)
    handle.pause()
    handle.setSpeed(4)

    const replayer = fakeReplayers.at(-1)
    expect(replayer?.pauseCalls).toEqual([1])
    expect(replayer?.playCalls).toEqual([50])
    expect(replayer?.speeds).toEqual([4])
    expect(handle.getCurrentTime()).toBe(321)
  })
})
