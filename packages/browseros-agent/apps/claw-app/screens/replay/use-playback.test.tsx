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
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

let root: Root
let container: HTMLElement
let latest: Playback
let nextFrameId = 1
let frames = new Map<number, FrameRequestCallback>()
let cancelledFrameIds: number[] = []

function Harness({ totalSeconds }: { totalSeconds: number }) {
  latest = usePlayback(totalSeconds)
  return <div data-time={latest.time} data-playing={latest.isPlaying} />
}

async function render(totalSeconds: number) {
  await act(async () => root.render(<Harness totalSeconds={totalSeconds} />))
}

/** Fires every pending animation frame with an explicit wall-clock stamp. */
async function advanceTo(wallMs: number) {
  const pending = [...frames.values()]
  frames.clear()
  await act(async () => {
    for (const callback of pending) callback(wallMs)
  })
}

beforeEach(async () => {
  nextFrameId = 1
  frames = new Map()
  cancelledFrameIds = []
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
      cancelledFrameIds.push(id)
      frames.delete(id)
    },
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
  await act(async () => root.unmount())
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
})

describe('usePlayback', () => {
  it('advances from animation-frame time at the selected speed', async () => {
    await render(100)
    await act(async () => latest.setSpeed(1))

    await advanceTo(1_000)
    expect(latest.time).toBe(0)

    await advanceTo(3_000)
    expect(latest.time).toBe(2)

    await act(async () => latest.setSpeed(4))
    await advanceTo(5_000)
    await advanceTo(6_000)
    expect(latest.time).toBe(6)
  })

  it('defaults to 2x without consulting a player', async () => {
    await render(100)

    expect(latest.speed).toBe(2)
    await advanceTo(1_000)
    await advanceTo(2_000)
    expect(latest.time).toBe(2)
  })

  it('excludes paused wall time when playback resumes', async () => {
    await render(100)
    await act(async () => latest.setSpeed(1))
    await advanceTo(0)
    await advanceTo(2_000)
    expect(latest.time).toBe(2)

    await act(async () => latest.pause())
    expect(latest.isPlaying).toBe(false)

    await act(async () => latest.play())
    await advanceTo(60_000)
    await advanceTo(61_000)
    expect(latest.time).toBe(3)
  })

  it('stops once at activity end and restarts from zero on play', async () => {
    await render(10)
    await act(async () => latest.setSpeed(1))
    await advanceTo(0)
    await advanceTo(20_000)

    expect(latest.time).toBe(10)
    expect(latest.isPlaying).toBe(false)
    expect(frames.size).toBe(0)

    await act(async () => latest.play())
    expect(latest.time).toBe(0)
    expect(latest.isPlaying).toBe(true)
  })

  it('clamps a seek to the activity window and pauses', async () => {
    await render(10)

    let applied = 0
    await act(async () => {
      applied = latest.seek(99)
    })
    expect(applied).toBe(10)
    expect(latest.time).toBe(10)
    expect(latest.isPlaying).toBe(false)

    await act(async () => {
      applied = latest.seek(-5)
    })
    expect(applied).toBe(0)
    expect(latest.time).toBe(0)
  })

  it('resumes when a live recording grows past where activity ended', async () => {
    await render(10)
    await act(async () => latest.setSpeed(1))
    await advanceTo(0)
    await advanceTo(20_000)
    expect(latest.isPlaying).toBe(false)

    await render(20)

    expect(latest.time).toBe(10)
    expect(latest.isPlaying).toBe(true)
  })

  it('does not resume growth over an operator pause', async () => {
    await render(10)
    await act(async () => latest.setSpeed(1))
    await advanceTo(0)
    await advanceTo(4_000)
    await act(async () => latest.pause())

    await render(20)

    expect(latest.time).toBe(4)
    expect(latest.isPlaying).toBe(false)
  })

  it('clamps the playhead when activity shrinks', async () => {
    await render(20)
    await act(async () => latest.seek(18))

    await render(6)

    expect(latest.time).toBe(6)
  })

  it('runs no clock while there is nothing replayable', async () => {
    await render(0)

    expect(latest.isPlaying).toBe(true)
    expect(frames.size).toBe(0)

    // Autoplay survives until visuals resolve, then starts from zero.
    await render(10)
    await advanceTo(0)
    await advanceTo(1_000)
    expect(latest.time).toBe(2)
  })

  it('keeps exactly one animation loop across control changes', async () => {
    await render(100)
    await act(async () => latest.setSpeed(1))
    await advanceTo(0)
    expect(frames.size).toBe(1)

    await act(async () => latest.setSpeed(4))
    expect(frames.size).toBe(1)

    await advanceTo(1_000)
    expect(frames.size).toBe(1)
  })

  it('cancels its animation frame on unmount', async () => {
    await render(100)
    await advanceTo(0)
    expect(frames.size).toBe(1)

    await act(async () => root.unmount())

    expect(frames.size).toBe(0)
    expect(cancelledFrameIds.length).toBeGreaterThan(0)
  })
})
