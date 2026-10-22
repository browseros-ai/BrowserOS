/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import {
  DEMO_CHEAPEST_ROW,
  DEMO_KEY_FRAMES,
  DEMO_LOOP_MS,
  DEMO_PROMPT,
  demoFrameAt,
} from './timeline'

describe('demoFrameAt', () => {
  it('starts on a calm desktop with neo in front, with no menu bar to drive', () => {
    const frame = demoFrameAt(0)
    expect('menu' in frame).toBe(false)
    expect(frame.fading).toBe(true)
    expect(frame.front).toBe('neo')
    expect(frame.claude.open).toBe(false)
    expect(frame.neo.up).toBe(false)
    expect(frame.neo.url).toBe('')
  })

  it('opens the Claude app from the dock in the first two seconds', () => {
    const hovering = demoFrameAt(1300)
    expect(hovering.pointer).toMatchObject({ on: true, atDock: true })
    expect(hovering.dock.tip).toBe(true)
    expect(demoFrameAt(1550).dock.claudeBounce).toBe(true)
    const opened = demoFrameAt(2300)
    expect(opened.claude.open).toBe(true)
    expect(opened.front).toBe('claude')
  })

  it('Ask: the prompt is fully typed and not yet sent', () => {
    const frame = demoFrameAt(DEMO_KEY_FRAMES.ask)
    expect(frame.claude.composer).toBe(DEMO_PROMPT)
    expect(frame.claude.sent).toBe(false)
    expect(frame.front).toBe('claude')
  })

  it('types the prompt one character at a time', () => {
    const lengths = [3000, 4000, 5000, 6000].map(
      (t) => demoFrameAt(t).claude.composer.length,
    )
    expect(lengths).toEqual([...lengths].sort((a, b) => a - b))
    expect(new Set(lengths).size).toBe(lengths.length)
    expect(DEMO_PROMPT.startsWith(demoFrameAt(4000).claude.composer)).toBe(true)
  })

  it('Watch: neo is in front, ③ is lit and "Cheapest" lands on $142', () => {
    const frame = demoFrameAt(DEMO_KEY_FRAMES.watch)
    expect(frame.front).toBe('neo')
    expect(frame.neo.up).toBe(true)
    expect(frame.neo.url).toBe('google.com/travel/flights')
    expect(frame.neo.to).toBe('New York')
    expect(frame.neo.when).toBe('Next Friday')
    expect(frame.claude.tools).toBe(3)
    expect(frame.claude.litTool).toBe(2)
    expect(frame.neo.badges).toEqual(['on', 'on', 'live'])
    expect(frame.neo.ring).toBe(DEMO_CHEAPEST_ROW)
    expect(frame.neo.tagged).toBe(true)
    expect(frame.neo.checked).toBe(5)
    expect(frame.claude.working).toBe(true)
    expect(frame.claude.seconds).toBe(12)
  })

  it('lights each tool row while it touches the page', () => {
    expect(demoFrameAt(8600).claude.litTool).toBe(0)
    expect(demoFrameAt(8600).neo.urlLit).toBe(true)
    expect(demoFrameAt(11_200).claude.litTool).toBe(1)
    expect(demoFrameAt(11_200).neo.badges[1]).toBe('live')
  })

  it('Done: Claude answers, the amber clears and the closing line shows', () => {
    const frame = demoFrameAt(DEMO_KEY_FRAMES.done)
    expect(frame.claude.answered).toBe(true)
    expect(frame.claude.working).toBe(false)
    expect(frame.neo.agentTag).toBe('done')
    expect(frame.neo.ringSettled).toBe(true)
    expect(frame.neo.agentPointer).toBe('off')
    expect(frame.front).toBe('neo')
    expect(frame.caption).toBe(true)
    expect(frame.fading).toBe(false)
  })

  it('fades out at the end of the loop and clamps outside it', () => {
    expect(demoFrameAt(29_500).fading).toBe(true)
    expect(demoFrameAt(29_500).caption).toBe(false)
    expect(demoFrameAt(DEMO_LOOP_MS + 5000)).toEqual(demoFrameAt(DEMO_LOOP_MS))
    expect(demoFrameAt(-10)).toEqual(demoFrameAt(0))
  })

  it('keeps the beats in order', () => {
    expect(DEMO_KEY_FRAMES.ask).toBeLessThan(8000)
    expect(DEMO_KEY_FRAMES.watch).toBeGreaterThan(8000)
    expect(DEMO_KEY_FRAMES.watch).toBeLessThan(22_000)
    expect(DEMO_KEY_FRAMES.done).toBeLessThan(DEMO_LOOP_MS)
  })
})
