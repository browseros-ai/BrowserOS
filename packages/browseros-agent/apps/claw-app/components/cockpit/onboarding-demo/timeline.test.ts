/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import {
  DEFAULT_DEMO_APP,
  DEMO_APPS,
  DEMO_CHEAPEST_ROW,
  DEMO_KEY_FRAMES,
  DEMO_LOOP_MS,
  DEMO_PROMPT,
  demoFrameAt,
} from './timeline'

describe('demoFrameAt', () => {
  it('starts on a calm desktop with neo in front and its new tab waiting', () => {
    const frame = demoFrameAt(0)
    expect(frame.fading).toBe(true)
    expect(frame.front).toBe('neo')
    expect(frame.agent.open).toBe(false)
    expect(frame.neo.pageShown).toBe(false)
    expect(frame.neo.url).toBe('')
  })

  it('opens the Claude app from the dock in the first two seconds', () => {
    const hovering = demoFrameAt(1300)
    expect(hovering.pointer).toMatchObject({ on: true, at: 'dock' })
    expect(hovering.dock.tip).toBe(true)
    expect(demoFrameAt(1550).dock.bounce).toBe(true)
    expect(demoFrameAt(1999).agent.open).toBe(false)
    const opened = demoFrameAt(2000)
    expect(opened.agent.open).toBe(true)
    expect(opened.front).toBe('agent')
  })

  it('Switch: Claude opens on Chat and the person clicks Code', () => {
    const opened = demoFrameAt(2300)
    expect(opened.agent.switched).toBe(false)
    expect(opened.callout).toBeNull()
    const frame = demoFrameAt(DEMO_KEY_FRAMES.switch)
    expect(frame.agent.switched).toBe(false)
    expect(frame.callout).toBe('Switch to Code')
    expect(frame.pointer).toMatchObject({ on: true, at: 'toggle' })
    expect(frame.dock.tip).toBe(false)
    expect(frame.agent.composer).toBe('')
    const clicked = demoFrameAt(3850)
    expect(clicked.agent.switched).toBe(true)
    expect(clicked.pointer.pressed).toBe(true)
    expect(demoFrameAt(4300).callout).toBeNull()
  })

  it('Ask: the pointer rests on the Claude tile again, its label showing', () => {
    expect(demoFrameAt(4400).pointer.at).toBe('toggle')
    const frame = demoFrameAt(DEMO_KEY_FRAMES.ask)
    expect(frame.pointer).toMatchObject({ on: true, at: 'dock' })
    expect(frame.dock.tip).toBe(true)
    expect(frame.agent.composer).toBe(DEMO_PROMPT)
    expect(frame.agent.sent).toBe(false)
    expect(frame.front).toBe('agent')
  })

  it('types the cockpit’s own prompt, and sends it before neo takes over', () => {
    expect(DEMO_PROMPT).toBe(
      'Use neo, find me the cheapest one-way flight to NYC next Friday on Google Flights.',
    )
    expect(demoFrameAt(4500).agent.composer).toBe('')
    expect(demoFrameAt(9999).agent.sent).toBe(true)
  })

  it('types the prompt one character at a time', () => {
    const lengths = [5000, 6000, 7000, 8000].map(
      (t) => demoFrameAt(t).agent.composer.length,
    )
    expect(lengths).toEqual([...lengths].sort((a, b) => a - b))
    expect(new Set(lengths).size).toBe(lengths.length)
    expect(DEMO_PROMPT.startsWith(demoFrameAt(6500).agent.composer)).toBe(true)
  })

  it('Watch: neo is in front, ③ is lit and "Cheapest" lands on $142', () => {
    const frame = demoFrameAt(DEMO_KEY_FRAMES.watch)
    expect(frame.front).toBe('neo')
    expect(frame.pointer.on).toBe(false)
    expect(frame.neo.pageShown).toBe(true)
    expect(frame.neo.url).toBe('google.com/travel/flights')
    expect(frame.neo.to).toBe('New York')
    expect(frame.neo.when).toBe('Next Friday')
    expect(frame.agent.tools).toBe(3)
    expect(frame.agent.litTool).toBe(2)
    expect(frame.neo.badges).toEqual(['on', 'on', 'live'])
    expect(frame.neo.ring).toBe(DEMO_CHEAPEST_ROW)
    expect(frame.neo.tagged).toBe(true)
    expect(frame.neo.checked).toBe(5)
    expect(frame.agent.working).toBe(true)
    expect(frame.agent.seconds).toBe(12)
  })

  it('lights each tool row while it touches the page', () => {
    expect(demoFrameAt(10_600).agent.litTool).toBe(0)
    expect(demoFrameAt(10_600).neo.urlLit).toBe(true)
    expect(demoFrameAt(13_200).agent.litTool).toBe(1)
    expect(demoFrameAt(13_200).neo.badges[1]).toBe('live')
  })

  it('Done: Claude answers and the ring settles on $142', () => {
    const frame = demoFrameAt(DEMO_KEY_FRAMES.done)
    expect(frame.agent.answered).toBe(true)
    expect(frame.agent.working).toBe(false)
    expect(frame.neo.agentTag).toBe('done')
    expect(frame.neo.ringSettled).toBe(true)
    expect(frame.neo.agentPointer).toBe('off')
    expect(frame.front).toBe('neo')
    expect(frame.fading).toBe(false)
    // Reduced motion holds this frame, so it must show Code selected.
    expect(frame.agent.switched).toBe(true)
    expect(frame.agent.app).toBe('claude')
  })

  it('fades out at the end of the loop and clamps outside it', () => {
    expect(demoFrameAt(31_500).fading).toBe(true)
    expect(DEMO_LOOP_MS).toBe(32_000)
    expect(demoFrameAt(DEMO_LOOP_MS + 5000)).toEqual(demoFrameAt(DEMO_LOOP_MS))
    expect(demoFrameAt(-10)).toEqual(demoFrameAt(0))
  })

  it('keeps Paper’s beat table: Switch 2–4.5 s, Ask to 10 s, Watch to 24 s', () => {
    expect(DEMO_KEY_FRAMES.switch).toBeGreaterThan(2000)
    expect(DEMO_KEY_FRAMES.switch).toBeLessThan(4500)
    expect(DEMO_KEY_FRAMES.ask).toBeGreaterThan(4500)
    expect(DEMO_KEY_FRAMES.ask).toBeLessThan(10_000)
    expect(demoFrameAt(9999).front).toBe('agent')
    expect(demoFrameAt(10_000).front).toBe('neo')
    expect(DEMO_KEY_FRAMES.watch).toBeLessThan(24_000)
    expect(demoFrameAt(23_999).agent.answered).toBe(false)
    expect(demoFrameAt(24_000).agent.answered).toBe(true)
    expect(DEMO_KEY_FRAMES.done).toBeGreaterThan(24_000)
    expect(DEMO_KEY_FRAMES.done).toBeLessThan(30_000)
  })
})

describe('per-app flows', () => {
  it('plays Claude by default and offers the three dock apps', () => {
    expect(DEFAULT_DEMO_APP).toBe('claude')
    expect(DEMO_APPS).toEqual(['terminal', 'claude', 'chatgpt'])
    expect(demoFrameAt(DEMO_KEY_FRAMES.ask)).toEqual(
      demoFrameAt(DEMO_KEY_FRAMES.ask, 'claude'),
    )
  })

  it('Terminal: types claude, presses return, then Claude Code starts', () => {
    const typing = demoFrameAt(3000, 'terminal')
    expect(typing.agent.command.length).toBeGreaterThan(0)
    expect('claude'.startsWith(typing.agent.command)).toBe(true)
    expect(typing.agent.switched).toBe(false)
    expect(typing.callout).toBe('Type claude, then press ⏎')
    expect(typing.agent.commandRun).toBe(false)
    const started = demoFrameAt(3800, 'terminal')
    expect(started.agent.command).toBe('claude')
    expect(started.agent.commandRun).toBe(true)
    expect(started.agent.switched).toBe(true)
    expect(started.callout).toBe('Type claude, then press ⏎')
    // Timed like the other two callouts: gone shortly after the switch.
    expect(demoFrameAt(4300, 'terminal').callout).toBeNull()
  })

  it('Terminal: your hands go to the keyboard, so the pointer and label leave', () => {
    const opening = demoFrameAt(2200, 'terminal')
    expect(opening.pointer).toMatchObject({ on: true, at: 'dock' })
    expect(opening.dock.tip).toBe(true)
    for (const t of [3000, 3800, DEMO_KEY_FRAMES.ask]) {
      const frame = demoFrameAt(t, 'terminal')
      expect(frame.pointer.on).toBe(false)
      expect(frame.dock.tip).toBe(false)
    }
  })

  it('ChatGPT: opens on Chat, a callout says Switch to Work, the click flips it', () => {
    const frame = demoFrameAt(DEMO_KEY_FRAMES.switch, 'chatgpt')
    expect(frame.callout).toBe('Switch to Work')
    expect(frame.agent.switched).toBe(false)
    expect(frame.pointer.at).toBe('toggle')
    expect(demoFrameAt(3850, 'chatgpt').agent.switched).toBe(true)
  })

  it('shares the ask and the neo scene across every app', () => {
    for (const app of DEMO_APPS) {
      const watch = demoFrameAt(DEMO_KEY_FRAMES.watch, app)
      expect(watch.agent.app).toBe(app)
      expect(watch.neo).toEqual(demoFrameAt(DEMO_KEY_FRAMES.watch).neo)
      expect(demoFrameAt(DEMO_KEY_FRAMES.ask, app).agent.composer).toBe(
        DEMO_PROMPT,
      )
    }
  })

  it('every app’s Done frame is switched, answered and calm (reduced motion)', () => {
    for (const app of DEMO_APPS) {
      const done = demoFrameAt(DEMO_KEY_FRAMES.done, app)
      expect(done.agent).toMatchObject({ app, switched: true, answered: true })
      expect(done.callout).toBeNull()
      expect(done.pointer.on).toBe(false)
    }
  })
})
