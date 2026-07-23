/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import { type Playback, usePlayback } from './use-playback'

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
let playback: Playback
let nowMs = 0
let nextAnimationFrameId = 1
let cancelledAnimationFrames: number[]
let animationFrames: Map<number, FrameRequestCallback>

function PlaybackHarness({ totalSeconds }: { totalSeconds: number }) {
  playback = usePlayback(totalSeconds)
  return <div data-time={playback.time} data-playing={playback.isPlaying} />
}

async function render(totalSeconds: number): Promise<void> {
  await act(async () =>
    root?.render(<PlaybackHarness totalSeconds={totalSeconds} />),
  )
}

async function advanceAnimationFrame(toMs: number): Promise<void> {
  nowMs = toMs
  const callbacks = [...animationFrames.values()]
  animationFrames.clear()
  await act(async () => {
    for (const callback of callbacks) callback(toMs)
  })
}

beforeEach(async () => {
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  nowMs = 0
  nextAnimationFrameId = 1
  cancelledAnimationFrames = []
  animationFrames = new Map()
  const requestAnimationFrame = (callback: FrameRequestCallback): number => {
    const id = nextAnimationFrameId++
    animationFrames.set(id, callback)
    return id
  }
  const cancelAnimationFrame = (id: number): void => {
    cancelledAnimationFrames.push(id)
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

describe('usePlayback', () => {
  it('advances from the monotonic app clock at 1x, 2x, and 4x', async () => {
    await render(100)

    await act(async () => playback.setSpeed(1))
    await advanceAnimationFrame(1_000)
    expect(playback.time).toBe(1)

    await act(async () => playback.setSpeed(2))
    await advanceAnimationFrame(2_000)
    expect(playback.time).toBe(3)

    await act(async () => playback.setSpeed(4))
    await advanceAnimationFrame(3_000)
    expect(playback.time).toBe(7)
  })

  it('accounts for elapsed time at the old speed before changing speed', async () => {
    await render(100)
    await advanceAnimationFrame(1_000)
    expect(playback.time).toBe(2)

    nowMs = 1_500
    await act(async () => playback.setSpeed(4))
    expect(playback.time).toBe(3)

    await advanceAnimationFrame(2_000)
    expect(playback.time).toBe(5)
  })

  it('freezes while paused and excludes paused wall time after resume', async () => {
    await render(100)
    await advanceAnimationFrame(1_000)
    expect(playback.time).toBe(2)

    await act(async () => playback.togglePlay())
    nowMs = 5_000
    await act(async () => playback.togglePlay())
    await advanceAnimationFrame(6_000)

    expect(playback.time).toBe(4)
  })

  it('clamps seeks, pauses, and returns the applied destination', async () => {
    await render(10)

    let applied = -1
    await act(async () => {
      applied = playback.seek(50)
    })
    expect(applied).toBe(10)
    expect(playback.time).toBe(10)
    expect(playback.isPlaying).toBe(false)

    await act(async () => {
      applied = playback.seek(-5)
    })
    expect(applied).toBe(0)
    expect(playback.time).toBe(0)
  })

  it('stops exactly at the end and restarts from zero on the next Play', async () => {
    await render(10)
    await advanceAnimationFrame(6_000)

    expect(playback.time).toBe(10)
    expect(playback.isPlaying).toBe(false)

    await act(async () => playback.togglePlay())
    expect(playback.time).toBe(0)
    expect(playback.isPlaying).toBe(true)

    await advanceAnimationFrame(6_500)
    expect(playback.time).toBe(1)
  })

  it('clamps a changed duration and never schedules two clock loops', async () => {
    await render(100)
    expect(animationFrames.size).toBe(1)

    await advanceAnimationFrame(2_000)
    expect(playback.time).toBe(4)
    expect(animationFrames.size).toBe(1)

    await render(3)
    expect(playback.time).toBe(3)
    expect(playback.isPlaying).toBe(false)
    expect(animationFrames.size).toBe(0)
  })

  it('cancels the only scheduled animation frame on unmount', async () => {
    await render(100)
    const scheduledId = [...animationFrames.keys()][0]
    expect(scheduledId).toBeDefined()

    await act(async () => root?.unmount())
    root = null

    if (scheduledId === undefined) throw new Error('expected a scheduled frame')
    expect(cancelledAnimationFrames).toContain(scheduledId)
    expect(animationFrames.size).toBe(0)
  })
})
