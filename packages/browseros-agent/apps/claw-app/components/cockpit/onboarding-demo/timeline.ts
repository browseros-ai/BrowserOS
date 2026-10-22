/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The cockpit's first-run demo as one pure timeline: `demoFrameAt(t, app)`
 * returns every visual fact of the drawn desktop at `t` ms into the ~32 s
 * loop, for the app whose dock tile was clicked (Claude by default).
 *
 *   0–2 s       Desktop  your pointer opens the app from the dock
 *   2–4.5 s     Switch   Claude: Chat → Code · ChatGPT: Chat → Work ·
 *                        Terminal: type `claude`, press return
 *   4.5–10 s    Ask      the chore types in and is sent
 *   10–24 s     Watch    neo comes forward: ① navigate ② fill ③ read, "Cheapest" lands
 *   24–30 s     Done     the app answers
 *   30–32.5 s   Hold, crossfade, loop
 *
 * Only the Desktop and Switch beats differ per app, inside the same 2.5 s,
 * so the neo scene and every later beat share one clock.
 *
 * Because a frame is a function of time alone, pause, replay, the loop wrap
 * and reduced motion (which holds `DEMO_KEY_FRAMES.done`) all land on exact
 * frames; CSS transitions only smooth the steps between them. Nothing on
 * screen names the beats; they exist for these key frames and the tests.
 */

/** Round 9 inserted the Switch beat; everything after it moved by this much. */
const SWITCH_SHIFT = 2500

export const DEMO_LOOP_MS = 30_000 + SWITCH_SHIFT

export const DEMO_PROMPT = 'Find the cheapest flight to New York next Friday.'
const DEMO_URL = 'google.com/travel/flights'

export const DEMO_FLIGHTS = [
  { time: '6:00 am', stops: '1 stop', price: '$189' },
  { time: '6:40 am', stops: '1 stop', price: '$171' },
  { time: '7:05 am', stops: 'Nonstop', price: '$142' },
  { time: '9:20 am', stops: 'Nonstop', price: '$198' },
  { time: '1:15 pm', stops: '1 stop', price: '$164' },
] as const
export const DEMO_CHEAPEST_ROW = 2

interface TypedRun {
  text: string
  /** Time each character lands; `times[k]` shows `text.slice(0, k + 1)`. */
  times: number[]
}

function typed(
  text: string,
  start: number,
  gap: (index: number, char: string) => number,
): TypedRun {
  const times: number[] = []
  let t = start
  for (let k = 1; k <= text.length; k++) {
    t += gap(k, text[k - 1])
    times.push(t)
  }
  return { text, times }
}

function shownAt(run: TypedRun, t: number): string {
  let count = 0
  while (count < run.times.length && run.times[count] <= t) count++
  return run.text.slice(0, count)
}

function endOf(run: TypedRun): number {
  return run.times.at(-1) ?? 0
}

export type DemoApp = 'terminal' | 'claude' | 'chatgpt'

/** The clickable dock tiles, in dock order (owner, round 10). */
export const DEMO_APPS: readonly DemoApp[] = ['terminal', 'claude', 'chatgpt']
export const DEFAULT_DEMO_APP: DemoApp = 'claude'

// Switch: the step people miss in each app, named by a callout. Claude and
// ChatGPT open on Chat; the pointer glides to the toggle and the click flips
// it (rounds 9 and 10). In Terminal you type `claude` and press return.
const OPEN = 2200
const TO_TOGGLE = 2500
const CALLOUT = 2900
const CLICK = 3800
const CALLOUT_GONE = 4200
const COMMAND_RUN = typed('claude', 2700, () => 85)
const RETURN_KEY = 3400
const CLAUDE_CODE_STARTED = 3700

const SWITCHES: Record<
  DemoApp,
  { callout: string | null; switchedAt: number; toToggle: boolean }
> = {
  claude: { callout: 'Switch to Code', switchedAt: CLICK, toToggle: true },
  chatgpt: { callout: 'Switch to Work', switchedAt: CLICK, toToggle: true },
  // Terminal has no toggle; the callout sits beside the command instead.
  terminal: {
    callout: 'Type claude, then press ⏎',
    switchedAt: CLAUDE_CODE_STARTED,
    toToggle: false,
  },
}

// Ask: ~70 ms a character with a little jitter, so it reads as a person typing.
const PROMPT_RUN = typed(
  DEMO_PROMPT,
  2800 + SWITCH_SHIFT,
  (k, char) => 58 + ((k * 37) % 23) + (char === ' ' ? 18 : 0),
)
const SUBMIT = Math.max(endOf(PROMPT_RUN) + 450, 6900 + SWITCH_SHIFT)

// Watch: neo comes forward; each numbered tool row lights the part of the
// page it touches.
const WATCH = 8000 + SWITCH_SHIFT
const NAVIGATE = WATCH + 500
const URL_RUN = typed(DEMO_URL, NAVIGATE + 60, () => 22)
const FILL = WATCH + 3000
const TO_RUN = typed('New York', FILL + 700, () => 60)
const WHEN_RUN = typed('Next Friday', endOf(TO_RUN) + 850, () => 55)
const FILLED = endOf(WHEN_RUN)
const READ = WATCH + 6400
const RING_STEP = 750
const DONE = 22_000 + SWITCH_SHIFT
const FADE_OUT = 29_400 + SWITCH_SHIFT

export const DEMO_KEY_FRAMES = {
  /** Before the switch: the callout up (Claude, ChatGPT), or `claude` typed. */
  switch: CLICK - 150,
  /** The prompt fully typed in the composer, not yet sent. */
  ask: endOf(PROMPT_RUN) + 200,
  /** ③ read lit and "Cheapest" landing. */
  watch: READ + 5000,
  /** The app's answer. Reduced motion holds this frame. */
  done: 28_500 + SWITCH_SHIFT,
} as const

const between = (t: number, from: number, to = Number.POSITIVE_INFINITY) =>
  t >= from && t < to

export interface DemoFrame {
  /** The whole stage fades out at the loop end and back in at its start. */
  fading: boolean
  pointer: { on: boolean; at: 'start' | 'dock' | 'toggle'; pressed: boolean }
  /** The callout beside the app's toggle ("Switch to Code"), if any. */
  callout: string | null
  /** The playing app's dock tile: its hover label, bounce and running dot. */
  dock: { tip: boolean; bounce: boolean; running: boolean }
  /** Frontmost window: it sits on top and shows coloured traffic lights. */
  front: 'neo' | 'agent'
  /** The AI app the person asks: Claude, Terminal or ChatGPT. */
  agent: {
    app: DemoApp
    open: boolean
    /** Past the Switch beat: Code (Claude), Work (ChatGPT), Claude Code running (Terminal). */
    switched: boolean
    /** The shell command typed before Claude Code starts (Terminal only). */
    command: string
    composer: string
    sent: boolean
    /** The session is running: header renamed, running dot in Recents. */
    working: boolean
    seconds: number
    /** The tool rows shown so far, and which one is lit. */
    tools: number
    litTool: number | null
    answered: boolean
  }
  neo: {
    up: boolean
    agentTag: 'off' | 'working' | 'done'
    url: string
    urlLit: boolean
    loading: 'idle' | 'loading' | 'loaded'
    pageShown: boolean
    to: string
    toLit: boolean
    when: string
    whenLit: boolean
    results: boolean
    resultsLit: boolean
    badges: [BadgeState, BadgeState, BadgeState]
    /** Row the focus ring sits on, or null when hidden. */
    ring: number | null
    ringSettled: boolean
    checked: number
    tagged: boolean
    agentPointer: 'off' | 'to' | 'when' | 'answer'
  }
}

export type BadgeState = 'off' | 'live' | 'on'

function badge(t: number, from: number, liveUntil: number): BadgeState {
  if (t < from) return 'off'
  return t < liveUntil ? 'live' : 'on'
}

function ringAt(t: number): { ring: number | null; checked: number } {
  if (t < READ + 400) return { ring: null, checked: 0 }
  const step = Math.min(
    DEMO_FLIGHTS.length - 1,
    Math.floor((t - READ - 400) / RING_STEP),
  )
  const checked = step + 1
  if (t >= READ + 4300) return { ring: DEMO_CHEAPEST_ROW, checked }
  return { ring: step, checked }
}

function litToolAt(t: number): number | null {
  if (between(t, NAVIGATE, NAVIGATE + 1900)) return 0
  if (between(t, FILL, FILLED + 650)) return 1
  if (between(t, READ, READ + 6000)) return 2
  return null
}

function agentPointerAt(t: number): DemoFrame['neo']['agentPointer'] {
  if (!between(t, FILL, DONE)) return 'off'
  if (t < endOf(TO_RUN) + 250) return 'to'
  if (t < READ + 4300) return 'when'
  return 'answer'
}

function pointerAt(t: number, app: DemoApp): DemoFrame['pointer'] {
  const { toToggle } = SWITCHES[app]
  const switchPress = toToggle
    ? between(t, CLICK, CLICK + 150)
    : between(t, RETURN_KEY, RETURN_KEY + 150)
  return {
    on: between(t, 250, WATCH),
    at: toToggle && t >= TO_TOGGLE ? 'toggle' : t >= 350 ? 'dock' : 'start',
    // Terminal's press is the return key; the pointer rests on the dock.
    pressed: between(t, 1500, 1650) || (toToggle && switchPress),
  }
}

export function demoFrameAt(
  rawT: number,
  app: DemoApp = DEFAULT_DEMO_APP,
): DemoFrame {
  const t = Math.min(Math.max(rawT, 0), DEMO_LOOP_MS)
  const { callout, switchedAt, toToggle } = SWITCHES[app]
  const sent = t >= SUBMIT
  const done = t >= DONE
  const { ring, checked } = ringAt(t)
  return {
    fading: t < 60 || t >= FADE_OUT,
    pointer: pointerAt(t, app),
    callout: callout && between(t, CALLOUT, CALLOUT_GONE) ? callout : null,
    dock: {
      tip: between(t, 1250, toToggle ? TO_TOGGLE : WATCH),
      bounce: between(t, 1500, 2350),
      running: t >= 1750,
    },
    front: between(t, OPEN, WATCH) ? 'agent' : 'neo',
    agent: {
      app,
      open: t >= OPEN,
      switched: t >= switchedAt,
      command: app === 'terminal' ? shownAt(COMMAND_RUN, t) : '',
      composer: sent ? '' : shownAt(PROMPT_RUN, t),
      sent,
      working: sent && !done,
      seconds: sent ? Math.floor((Math.min(t, DONE - 1) - SUBMIT) / 1000) : 0,
      tools: [NAVIGATE, FILL, READ].filter((start) => t >= start).length,
      litTool: litToolAt(t),
      answered: done,
    },
    neo: {
      up: t >= WATCH,
      agentTag: done ? 'done' : sent ? 'working' : 'off',
      url: shownAt(URL_RUN, t),
      urlLit: between(t, NAVIGATE, NAVIGATE + 1900),
      loading: between(t, NAVIGATE + 800, NAVIGATE + 1250)
        ? 'loading'
        : t >= NAVIGATE + 1250
          ? 'loaded'
          : 'idle',
      pageShown: t >= NAVIGATE + 1050,
      to: shownAt(TO_RUN, t),
      toLit: between(t, FILL + 600, endOf(TO_RUN) + 250),
      when: shownAt(WHEN_RUN, t),
      whenLit: between(t, endOf(TO_RUN) + 750, FILLED + 350),
      results: t >= FILLED + 350,
      resultsLit: between(t, READ, READ + 6000),
      badges: [
        badge(t, NAVIGATE, NAVIGATE + 1900),
        badge(t, FILL, FILLED + 650),
        badge(t, READ, READ + 6000),
      ],
      ring,
      ringSettled: done,
      checked,
      tagged: t >= READ + 4700,
      agentPointer: agentPointerAt(t),
    },
  }
}
