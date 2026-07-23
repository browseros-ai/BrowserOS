/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act, useEffect, useMemo } from 'react'
import type { Root } from 'react-dom/client'
import type { ReplayEvent } from '@/modules/api/replay.hooks'
import type { ReplayPlayerHandle, ReplayPlayerProps } from './ReplayPlayer'
import type { ReplayViewportTrack } from './ReplayViewport'

interface FakeHandle extends ReplayPlayerHandle {
  running: boolean
  currentTime: number
  destroyed: boolean
  pauseCalls: number
  playCalls: number[]
  seekCalls: number[]
  speedCalls: number[]
}

interface FakePlayer {
  tabId: number
  handle: FakeHandle
  destroyed: boolean
  ready: () => void
}

const fakePlayers: FakePlayer[] = []
const immediatelyReadyTabs = new Set<number>()

function createHandle(): FakeHandle {
  return {
    running: false,
    currentTime: 0,
    destroyed: false,
    pauseCalls: 0,
    playCalls: [],
    seekCalls: [],
    speedCalls: [],
    seek(ms) {
      this.currentTime = ms
      this.running = false
      this.seekCalls.push(ms)
    },
    play(ms) {
      this.currentTime = ms
      this.running = true
      this.playCalls.push(ms)
    },
    pause() {
      this.running = false
      this.pauseCalls += 1
    },
    setSpeed(speed) {
      this.speedCalls.push(speed)
    },
    getCurrentTime() {
      return this.currentTime
    },
  }
}

function FakeReplayPlayer({ events, onReady }: ReplayPlayerProps) {
  const tabId = events[0]?.tabId ?? -1
  const player = useMemo<FakePlayer>(() => {
    const handle = createHandle()
    const empty = tabId === -1
    handle.destroyed = empty
    const instance = {
      tabId,
      handle,
      destroyed: empty,
      ready: () => onReady(handle),
    }
    fakePlayers.push(instance)
    return instance
  }, [onReady, tabId])

  useEffect(() => {
    if (events.length < 2) return
    if (immediatelyReadyTabs.has(tabId)) player.ready()
    return () => {
      player.destroyed = true
      player.handle.destroyed = true
      player.handle.pause()
      onReady(null)
    }
  }, [events.length, onReady, player, tabId])

  return <div data-fake-player={tabId} />
}

mock.module('./ReplayPlayer', () => ({
  ReplayPlayer: FakeReplayPlayer,
}))

mock.module('@/lib/utils', () => ({
  cn: (...values: Array<string | false | null | undefined>) =>
    values.filter(Boolean).join(' '),
}))

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

function eventsForTab(tabId: number): ReplayEvent[] {
  return [
    {
      sessionId: 'session-1',
      documentId: `document-${tabId}`,
      targetId: `target-${tabId}`,
      tabId,
      ts: tabId * 1_000,
      type: 4,
      data: { width: 1280, height: 720 },
    },
    {
      sessionId: 'session-1',
      documentId: `document-${tabId}`,
      targetId: `target-${tabId}`,
      tabId,
      ts: tabId * 1_000 + 1,
      type: 2,
      data: {},
    },
  ]
}

function track(tabId: number): ReplayViewportTrack {
  return { tabId, events: eventsForTab(tabId) }
}

const tabOne = track(1)
const tabTwo = track(2)
const tabThree = track(3)

beforeEach(async () => {
  fakePlayers.length = 0
  immediatelyReadyTabs.clear()
  nowMs = 0
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

async function renderViewport(
  overrides: Partial<React.ComponentProps<typeof ReplayViewport>> = {},
): Promise<void> {
  const props: React.ComponentProps<typeof ReplayViewport> = {
    site: 'example.com',
    frame: undefined,
    activeTrack: tabOne,
    standbyTrack: tabTwo,
    activeTimeMs: 1_000,
    standbyTimeMs: 2_000,
    isPlaying: true,
    speed: 2,
    syncKey: 0,
    mode: 'follow',
    ...overrides,
  }
  await act(async () => root?.render(<ReplayViewport {...props} />))
}

describe('ReplayViewport', () => {
  it('prepares one paused standby while only the visible slot runs', async () => {
    immediatelyReadyTabs.add(1)
    immediatelyReadyTabs.add(2)
    await renderViewport()

    const livePlayers = fakePlayers.filter(({ destroyed }) => !destroyed)
    expect(livePlayers).toHaveLength(2)
    const active = livePlayers.find(({ tabId }) => tabId === 1)
    const standby = livePlayers.find(({ tabId }) => tabId === 2)
    expect(active?.handle.playCalls).toContain(1_000)
    expect(active?.handle.running).toBe(true)
    expect(standby?.handle.seekCalls).toContain(2_000)
    expect(standby?.handle.running).toBe(false)
    expect(livePlayers.filter(({ handle }) => handle.running)).toHaveLength(1)
  })

  it('pauses the old active, re-seeks the standby, then promotes without an empty slot', async () => {
    immediatelyReadyTabs.add(1)
    immediatelyReadyTabs.add(2)
    await renderViewport()
    const tabOnePlayer = fakePlayers.find(
      ({ tabId, destroyed }) => tabId === 1 && !destroyed,
    )
    const tabTwoPlayer = fakePlayers.find(
      ({ tabId, destroyed }) => tabId === 2 && !destroyed,
    )

    await renderViewport({
      activeTrack: tabTwo,
      standbyTrack: null,
      activeTimeMs: 5_000,
      standbyTimeMs: 0,
      syncKey: 1,
    })

    expect(tabOnePlayer?.handle.running).toBe(false)
    expect(tabOnePlayer?.handle.seekCalls).not.toContain(5_000)
    expect(tabTwoPlayer?.handle.seekCalls).toContain(5_000)
    expect(tabTwoPlayer?.handle.running).toBe(true)
    expect(
      container
        .querySelector('[data-replay-slot-active="true"]')
        ?.getAttribute('data-replay-tab-id'),
    ).toBe('2')
  })

  it('reuses the paused prior active when the camera returns to that tab', async () => {
    immediatelyReadyTabs.add(1)
    immediatelyReadyTabs.add(2)
    await renderViewport()
    await renderViewport({
      activeTrack: tabTwo,
      standbyTrack: null,
      activeTimeMs: 5_000,
    })
    const playerCountAfterFirstPromotion = fakePlayers.length

    await renderViewport({
      activeTrack: tabOne,
      standbyTrack: null,
      activeTimeMs: 6_000,
    })

    expect(fakePlayers).toHaveLength(playerCountAfterFirstPromotion)
    const tabOnePlayer = fakePlayers.find(
      ({ tabId, destroyed }) => tabId === 1 && !destroyed,
    )
    const tabTwoPlayer = fakePlayers.find(
      ({ tabId, destroyed }) => tabId === 2 && !destroyed,
    )
    expect(tabOnePlayer?.handle.running).toBe(true)
    expect(tabOnePlayer?.handle.seekCalls).toContain(6_000)
    expect(tabTwoPlayer?.handle.running).toBe(false)
  })

  it('rejects stale readiness after rapid standby replacement and waits for the current generation', async () => {
    immediatelyReadyTabs.add(1)
    await renderViewport()
    const staleTabTwo = fakePlayers.find(({ tabId }) => tabId === 2)
    if (!staleTabTwo) throw new Error('expected tab 2 standby')

    await renderViewport({ standbyTrack: tabThree, standbyTimeMs: 3_000 })
    const tabThreePlayer = fakePlayers.find(
      ({ tabId, destroyed }) => tabId === 3 && !destroyed,
    )
    if (!tabThreePlayer) throw new Error('expected tab 3 standby')

    await renderViewport({
      activeTrack: tabThree,
      standbyTrack: null,
      activeTimeMs: 4_000,
    })
    await act(async () => staleTabTwo.ready())
    expect(staleTabTwo.handle.running).toBe(false)
    expect(
      container
        .querySelector('[data-replay-slot-active="true"]')
        ?.getAttribute('data-replay-tab-id'),
    ).toBe('1')

    await act(async () => tabThreePlayer.ready())
    expect(tabThreePlayer.handle.running).toBe(true)
    expect(
      container
        .querySelector('[data-replay-slot-active="true"]')
        ?.getAttribute('data-replay-tab-id'),
    ).toBe('3')
  })

  it('keeps rapid pending churn within two live player instances', async () => {
    immediatelyReadyTabs.add(1)
    for (let tabId = 2; tabId <= 12; tabId += 1) {
      immediatelyReadyTabs.add(tabId)
    }
    await renderViewport()

    for (let tabId = 3; tabId <= 12; tabId += 1) {
      await renderViewport({
        standbyTrack: track(tabId),
        standbyTimeMs: tabId * 1_000,
      })
      expect(
        fakePlayers.filter(({ destroyed }) => !destroyed).length,
      ).toBeLessThanOrEqual(2)
      expect(
        fakePlayers.filter(({ handle }) => handle.running).length,
      ).toBeLessThanOrEqual(1)
    }
  })

  it('checks drift on a throttle and corrects only beyond 250 ms', async () => {
    immediatelyReadyTabs.add(1)
    await renderViewport({ standbyTrack: null, activeTimeMs: 0 })
    const active = fakePlayers.find(
      ({ tabId, destroyed }) => tabId === 1 && !destroyed,
    )
    if (!active) throw new Error('expected active player')
    const initialSeekCount = active.handle.seekCalls.length
    active.handle.currentTime = 0

    nowMs = 500
    await renderViewport({ standbyTrack: null, activeTimeMs: 500 })
    expect(active.handle.seekCalls).toHaveLength(initialSeekCount)

    nowMs = 1_000
    await renderViewport({ standbyTrack: null, activeTimeMs: 200 })
    expect(active.handle.seekCalls).toHaveLength(initialSeekCount)

    nowMs = 2_000
    await renderViewport({ standbyTrack: null, activeTimeMs: 400 })
    expect(active.handle.seekCalls).toHaveLength(initialSeekCount + 1)
    expect(active.handle.seekCalls.at(-1)).toBe(400)
  })

  it('removes the inactive player for inspection and stops everything on cleanup', async () => {
    immediatelyReadyTabs.add(1)
    immediatelyReadyTabs.add(2)
    await renderViewport()
    await renderViewport({
      mode: 'inspect',
      standbyTrack: null,
      isPlaying: false,
    })

    expect(fakePlayers.filter(({ destroyed }) => !destroyed)).toHaveLength(1)

    await act(async () => root?.unmount())
    root = null
    expect(fakePlayers.every(({ destroyed }) => destroyed)).toBe(true)
    expect(fakePlayers.every(({ handle }) => !handle.running)).toBe(true)
  })
})

const { ReplayViewport } = await import('./ReplayViewport')
