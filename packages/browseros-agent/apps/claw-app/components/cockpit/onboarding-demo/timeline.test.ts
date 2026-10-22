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
    expect(hovering.pointer).toMatchObject({ on: true, at: 'dock' })
    expect(hovering.dock.tip).toBe(true)
    expect(demoFrameAt(1550).dock.claudeBounce).toBe(true)
    const opened = demoFrameAt(2300)
    expect(opened.claude.open).toBe(true)
    expect(opened.front).toBe('claude')
  })

  it('Switch: Claude opens on Chat and the person clicks Code', () => {
    const opened = demoFrameAt(2300)
    expect(opened.claude.mode).toBe('chat')
    expect(opened.switchCallout).toBe(false)
    const frame = demoFrameAt(DEMO_KEY_FRAMES.switch)
    expect(frame.claude.mode).toBe('chat')
    expect(frame.switchCallout).toBe(true)
    expect(frame.pointer).toMatchObject({ on: true, at: 'toggle' })
    expect(frame.dock.tip).toBe(false)
    expect(frame.claude.composer).toBe('')
    const clicked = demoFrameAt(3850)
    expect(clicked.claude.mode).toBe('code')
    expect(clicked.pointer.pressed).toBe(true)
    expect(demoFrameAt(4300).switchCallout).toBe(false)
  })

  it('Ask: the prompt is fully typed and not yet sent', () => {
    const frame = demoFrameAt(DEMO_KEY_FRAMES.ask)
    expect(frame.claude.composer).toBe(DEMO_PROMPT)
    expect(frame.claude.sent).toBe(false)
    expect(frame.front).toBe('claude')
  })

  it('types the prompt one character at a time', () => {
    const lengths = [5500, 6500, 7500, 8500].map(
      (t) => demoFrameAt(t).claude.composer.length,
    )
    expect(lengths).toEqual([...lengths].sort((a, b) => a - b))
    expect(new Set(lengths).size).toBe(lengths.length)
    expect(DEMO_PROMPT.startsWith(demoFrameAt(6500).claude.composer)).toBe(true)
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
    expect(demoFrameAt(11_100).claude.litTool).toBe(0)
    expect(demoFrameAt(11_100).neo.urlLit).toBe(true)
    expect(demoFrameAt(13_700).claude.litTool).toBe(1)
    expect(demoFrameAt(13_700).neo.badges[1]).toBe('live')
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
    // Reduced motion holds this frame, so it must show Code selected.
    expect(frame.claude.mode).toBe('code')
  })

  it('fades out at the end of the loop and clamps outside it', () => {
    expect(demoFrameAt(32_000).fading).toBe(true)
    expect(demoFrameAt(32_000).caption).toBe(false)
    expect(DEMO_LOOP_MS).toBe(32_500)
    expect(demoFrameAt(DEMO_LOOP_MS + 5000)).toEqual(demoFrameAt(DEMO_LOOP_MS))
    expect(demoFrameAt(-10)).toEqual(demoFrameAt(0))
  })

  it('keeps the beats in order', () => {
    expect(DEMO_KEY_FRAMES.switch).toBeLessThan(4500)
    expect(DEMO_KEY_FRAMES.ask).toBeGreaterThan(4500)
    expect(DEMO_KEY_FRAMES.ask).toBeLessThan(10_500)
    expect(DEMO_KEY_FRAMES.watch).toBeGreaterThan(10_500)
    expect(DEMO_KEY_FRAMES.watch).toBeLessThan(24_500)
    expect(DEMO_KEY_FRAMES.done).toBeLessThan(DEMO_LOOP_MS)
  })
})
