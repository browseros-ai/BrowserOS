import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act, StrictMode } from 'react'
import type { Root } from 'react-dom/client'
import type { ReplayEvent } from '@/modules/api/replay.hooks'
import type { ReplayPlayerHandle } from './ReplayViewport'

interface ReplayerConfig {
  root: HTMLElement
}

class FakeReplayer {
  readonly marker: HTMLElement
  destroyed = false

  constructor(_events: unknown[], config: ReplayerConfig) {
    this.marker = document.createElement('div')
    this.marker.dataset.fakeReplayer = String(fakeReplayers.length + 1)
    config.root.append(this.marker)
    fakeReplayers.push(this)
  }

  pause(_ms?: number): void {}

  play(_ms?: number): void {}

  setConfig(_config: { speed: number }): void {}

  getCurrentTime(): number {
    return 0
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
  it('owns one live Replayer across Strict Mode setup, replacement, and unmount', async () => {
    const readyValues: Array<ReplayPlayerHandle | null> = []
    const onPlayerReady = (handle: ReplayPlayerHandle | null): void => {
      readyValues.push(handle)
    }
    const { ReplayViewport } = await import('./ReplayViewport')

    await act(async () => {
      root?.render(
        <StrictMode>
          <ReplayViewport
            site="example.com"
            frame={undefined}
            events={events}
            onPlayerReady={onPlayerReady}
          />
        </StrictMode>,
      )
    })

    const canvas = container.querySelector('[data-replay-canvas]')
    expect(canvas?.querySelectorAll('[data-fake-replayer]')).toHaveLength(1)
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
      targetId: 'target-b',
    }))
    await act(async () => {
      root?.render(
        <StrictMode>
          <ReplayViewport
            site="example.com"
            frame={undefined}
            events={replacementEvents}
            onPlayerReady={onPlayerReady}
          />
        </StrictMode>,
      )
    })

    expect(firstLiveReplayer?.destroyed).toBe(true)
    expect(canvas?.querySelectorAll('[data-fake-replayer]')).toHaveLength(1)
    expect(
      fakeReplayers.filter((replayer) => !replayer.destroyed),
    ).toHaveLength(1)

    await act(async () => root?.unmount())
    root = null

    expect(fakeReplayers.every((replayer) => replayer.destroyed)).toBe(true)
    expect(readyValues.at(-1)).toBeNull()
  })
})
