/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The first-run demo that replaced the onboarding video: a drawn macOS
 * desktop (Paper's 976×520 stage, wallpaper and slim menu bar) where an AI app
 * asks and neo finds the $142 flight. Claude plays by
 * default; the Terminal, Claude and ChatGPT dock tiles switch to that app's
 * flow (round 10). One requestAnimationFrame clock drives `demoFrameAt`
 * (onboarding-demo/timeline), so pause, replay, the loop wrap, a flow switch
 * and reduced motion all land on exact frames.
 */

import {
  type ReactNode,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { DemoDesktop } from './onboarding-demo/DemoDesktop'
import {
  DEFAULT_DEMO_APP,
  DEMO_KEY_FRAMES,
  DEMO_LOOP_MS,
  type DemoApp,
  type DemoFrame,
  demoFrameAt,
} from './onboarding-demo/timeline'

/** Paper's cockpit stage; the column is 976 px wide at 1280. */
const STAGE_WIDTH = 976
const STAGE_HEIGHT = 520
/** A hidden tab resumes where it paused instead of jumping ahead. */
const MAX_STEP_MS = 64
/** Claude Code's working spinner, one glyph per ~110 ms. */
const SPIN_GLYPHS = ['·', '✢', '✳', '✶', '✻', '✽', '✻', '✶', '✳', '✢']
const SPIN_STEP_MS = 110
/** The demo stands in for the old video, so it reports the video events. */
const ANALYTICS = { surface: 'demo' } as const

const REDUCED_MOTION_QUERY = '(prefers-reduced-motion: reduce)'

const FLIGHT_STORY =
  'ask it to use neo to find the cheapest one-way flight to NYC next Friday on Google Flights. neo opens Google Flights, fills it in, reads five flights and tags the cheapest, $142 at 7:05 am, and the answer comes back.'

/** What the drawn desktop shows, per flow, for screen readers. */
const STAGE_LABELS: Record<DemoApp, string> = {
  claude: `Demo on a Mac: you open the Claude app, switch it to Code and ${FLIGHT_STORY}`,
  terminal: `Demo on a Mac: you open Terminal, start Claude Code with claude and ${FLIGHT_STORY}`,
  chatgpt: `Demo on a Mac: you open ChatGPT, switch it to Work and ${FLIGHT_STORY}`,
}

function prefersReducedMotion(): boolean {
  return (
    typeof window !== 'undefined' &&
    Boolean(window.matchMedia?.(REDUCED_MOTION_QUERY).matches)
  )
}

interface Playback {
  frame: DemoFrame
  spinGlyph: string
  playing: boolean
  /** Bumps on every jump (loop wrap, Replay) so that frame skips transitions. */
  jumpId: number
  togglePlaying: () => void
  replay: () => void
  /** Plays an app's flow from the start (its Done frame under reduced motion). */
  pickApp: (app: DemoApp) => void
}

/**
 * Owns the clock. Re-renders only when the frame (or the spinner, while it
 * shows) changes, which is at most a few times a second outside typing.
 */
function useDemoPlayback(): Playback {
  const [startStill] = useState(prefersReducedMotion)
  const [view, setView] = useState(() => ({
    t: startStill ? DEMO_KEY_FRAMES.done : 0,
    spin: 0,
    jumpId: 0,
    app: DEFAULT_DEMO_APP,
  }))
  const [playing, setPlaying] = useState(!startStill)
  // The rAF loop reads the clock (and the playing app) from this ref, so a
  // flow switch takes effect on the very next frame without restarting it.
  const clock = useRef({ t: view.t, key: '', spin: 0, app: view.app })
  const reported = useRef({ played: false, completed: false })

  useEffect(() => {
    if (!playing) return
    if (!reported.current.played) {
      reported.current.played = true
      track(AnalyticsEvent.OnboardingVideoPlayed, ANALYTICS)
    }
    let last = performance.now()
    let raf = requestAnimationFrame(function tick(now) {
      const c = clock.current
      c.t += Math.min(now - last, MAX_STEP_MS)
      last = now
      let jumped = false
      if (c.t >= DEMO_LOOP_MS) {
        c.t = 0
        jumped = true
        if (!reported.current.completed) {
          reported.current.completed = true
          track(AnalyticsEvent.OnboardingVideoCompleted, ANALYTICS)
        }
      }
      const frame = demoFrameAt(c.t, c.app)
      // Only Terminal draws the spinner; the apps' windows need no re-render.
      const spin =
        frame.agent.working && c.app === 'terminal'
          ? Math.floor(now / SPIN_STEP_MS) % SPIN_GLYPHS.length
          : 0
      const key = JSON.stringify(frame)
      if (jumped || key !== c.key || spin !== c.spin) {
        c.key = key
        c.spin = spin
        const { t, app } = c
        setView((prev) => ({
          t,
          spin,
          app,
          jumpId: jumped ? prev.jumpId + 1 : prev.jumpId,
        }))
      }
      raf = requestAnimationFrame(tick)
    })
    return () => cancelAnimationFrame(raf)
  }, [playing])

  return {
    frame: demoFrameAt(view.t, view.app),
    spinGlyph: SPIN_GLYPHS[view.spin],
    playing,
    jumpId: view.jumpId,
    togglePlaying() {
      track(
        playing
          ? AnalyticsEvent.OnboardingVideoPaused
          : AnalyticsEvent.OnboardingVideoResumed,
        ANALYTICS,
      )
      setPlaying(!playing)
    },
    replay() {
      clock.current.t = 0
      clock.current.key = ''
      setView((prev) => ({ ...prev, t: 0, spin: 0, jumpId: prev.jumpId + 1 }))
      if (!playing) {
        track(AnalyticsEvent.OnboardingVideoResumed, ANALYTICS)
        setPlaying(true)
      }
    },
    pickApp(app) {
      track(AnalyticsEvent.OnboardingLinkClicked, { link: `demo-${app}` })
      const still = prefersReducedMotion()
      const t = still ? DEMO_KEY_FRAMES.done : 0
      clock.current = { ...clock.current, t, key: '', app }
      setView((prev) => ({ t, spin: 0, app, jumpId: prev.jumpId + 1 }))
      setPlaying(!still)
    },
  }
}

/**
 * Freezes every CSS transition and animation running in the stage, mid-move
 * (the pointer's 1.1 s glide, a fade, the badge halo), or resumes the ones it
 * froze. Stopping the clock alone would let in-flight transitions finish, and
 * killing transitions would snap them to their end instead of holding them.
 */
export function setStageMotionPaused(
  stage: Pick<Element, 'getAnimations'> | null,
  paused: boolean,
) {
  for (const animation of stage?.getAnimations?.({ subtree: true }) ?? []) {
    if (paused) {
      // pause() only takes hold on the next frame; pinning currentTime holds
      // the animation exactly where it was when Pause was pressed.
      const heldAt = animation.currentTime
      animation.pause()
      animation.currentTime = heldAt
    }
    // Only resume what Pause froze: play() would replay a finished animation.
    else if (animation.playState === 'paused') animation.play()
  }
}

/** Lays the stage out at 976×520 and scales it to the player's width. */
function useStageScale() {
  const boxRef = useRef<HTMLDivElement>(null)
  const [scale, setScale] = useState(1)
  useLayoutEffect(() => {
    const box = boxRef.current
    if (!box) return
    const measure = () => {
      if (box.clientWidth) setScale(box.clientWidth / STAGE_WIDTH)
    }
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(box)
    return () => observer.disconnect()
  }, [])
  return { boxRef, scale }
}

export function OnboardingDemo() {
  const playback = useDemoPlayback()
  const { boxRef, scale } = useStageScale()
  const stageRef = useRef<HTMLDivElement>(null)
  const [instant, setInstant] = useState(true)
  const { playing } = playback

  useLayoutEffect(() => {
    setStageMotionPaused(stageRef.current, !playing)
  }, [playing])

  // A jump (loop wrap, Replay) paints its frame with transitions off, then
  // turns them back on a frame later, so nothing animates backwards. The
  // layout-effect update lands before paint, cancelling any started transition.
  const { jumpId } = playback
  // biome-ignore lint/correctness/useExhaustiveDependencies: jumpId is the trigger; each jump re-arms the instant frame.
  useLayoutEffect(() => {
    setInstant(true)
    const raf = requestAnimationFrame(() => setInstant(false))
    return () => cancelAnimationFrame(raf)
  }, [jumpId])

  return (
    <figure className="m-0">
      {/* Not role="img": the dock's app tiles inside the stage are buttons,
          so the scene is described in text and its drawing is aria-hidden. */}
      <p className="sr-only">{STAGE_LABELS[playback.frame.agent.app]}</p>
      <div
        ref={boxRef}
        className="relative aspect-[976/520] w-full overflow-hidden rounded-2xl bg-[#EAF1FF] shadow-[0_0_0_1px_#CFDBF0,0_30px_60px_-30px_rgb(1_18_63/0.35)]"
      >
        <div
          ref={stageRef}
          data-instant={instant || undefined}
          data-paused={!playback.playing || undefined}
          style={{
            width: STAGE_WIDTH,
            height: STAGE_HEIGHT,
            transform: `scale(${scale})`,
          }}
          className="absolute top-0 left-0 origin-top-left font-sans text-[#01123F] data-instant:[&_*]:animate-none! data-instant:[&_*]:transition-none! motion-reduce:[&_*]:animate-none! motion-reduce:[&_*]:transition-none! data-paused:[&_*]:[animation-play-state:paused]"
        >
          <DemoDesktop
            frame={playback.frame}
            spinGlyph={playback.spinGlyph}
            onPickApp={playback.pickApp}
          />
          {/* S from the right edge, centred on the dock (Paper). */}
          <div className="absolute top-[451px] right-5 z-10 flex gap-2">
            <DemoControl
              label={playback.playing ? 'Pause the demo' : 'Play the demo'}
              onClick={playback.togglePlaying}
            >
              {playback.playing ? (
                <svg aria-hidden width="12" height="12" viewBox="0 0 12 12">
                  <rect x="2.5" y="2" width="2.4" height="8" rx="0.8" />
                  <rect x="7.1" y="2" width="2.4" height="8" rx="0.8" />
                </svg>
              ) : (
                <svg aria-hidden width="12" height="12" viewBox="0 0 12 12">
                  <path d="M3.5 2.2v7.6a.6.6 0 0 0 .9.5l6-3.8a.6.6 0 0 0 0-1L4.4 1.7a.6.6 0 0 0-.9.5z" />
                </svg>
              )}
            </DemoControl>
            <DemoControl label="Replay the demo" onClick={playback.replay}>
              <svg aria-hidden width="14" height="14" viewBox="0 0 14 14">
                <path
                  d="M11.5 7a4.5 4.5 0 1 1-1.4-3.3M11.4 1.8v2.6H8.8"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.6"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
            </DemoControl>
          </div>
        </div>
      </div>
    </figure>
  )
}

function DemoControl({
  label,
  onClick,
  children,
}: {
  label: string
  onClick: () => void
  children: ReactNode
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className="grid size-8 cursor-pointer place-items-center rounded-full border border-[#DCE4F2] bg-white/88 fill-[#575D69] text-[#575D69] shadow-[0_2px_6px_rgb(1_18_63/0.08)] transition-colors hover:bg-white hover:fill-[#01123F] hover:text-[#01123F] focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-[#0254EC]/25"
    >
      {children}
    </button>
  )
}
