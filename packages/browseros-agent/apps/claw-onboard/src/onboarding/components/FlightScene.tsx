/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Check, Lock } from 'lucide-react'
import { useEffect, useState } from 'react'
import { cn } from '@/lib/utils'
import { usePrefersReducedMotion } from '../use-chat-reveal'
import { NeoMark } from './NeoMark'

/** Same rows and prices as the cockpit demo: one chore, told twice. */
const FLIGHTS = [
  ['6:00 am', '1 stop', '$189'],
  ['6:40 am', '1 stop', '$171'],
  ['7:05 am', 'Nonstop', '$142'],
  ['9:20 am', 'Nonstop', '$198'],
  ['1:15 pm', '1 stop', '$164'],
] as const
const CHEAPEST_ROW = 2

/** The cockpit's Try one pill and the demo type the same words. */
const HI_PROMPT =
  'Use neo, find me the cheapest one-way flight to NYC next Friday on Google Flights.'

const LOOP_MS = 9900
const ROW_STEP_MS = 700

export interface SceneFrame {
  asked: boolean
  status: 'idle' | 'working' | 'done'
  seconds: number
  rows: boolean
  /** Row the focus ring sits on, or -1 when hidden. */
  ring: number
  tagged: boolean
  checked: number
}

const FINAL_FRAME: SceneFrame = {
  asked: true,
  status: 'done',
  seconds: 6,
  rows: true,
  ring: -1,
  tagged: true,
  checked: 5,
}

/** One ~10 s loop: Claude Code asks, neo checks five flights, tags $142. */
export function flightSceneFrameAt(t: number): SceneFrame {
  const rowIndex = Math.floor((t - 2500) / ROW_STEP_MS)
  return {
    asked: t >= 700,
    status: t >= 6900 ? 'done' : t >= 1500 ? 'working' : 'idle',
    // Stops counting when the answer lands, like the cockpit demo.
    seconds: t >= 1500 ? 1 + Math.floor((Math.min(t, 6899) - 1500) / 1000) : 0,
    rows: t >= 1900,
    ring:
      t >= 6900
        ? -1
        : t >= 6000
          ? CHEAPEST_ROW
          : t >= 2500
            ? Math.min(rowIndex, 4)
            : -1,
    tagged: t >= 6000,
    checked: t >= 2500 ? Math.min(rowIndex + 1, 5) : 0,
  }
}

/** Loop time quantised to 100 ms, so the scene re-renders ~10× a second. */
function useLoopTime(enabled: boolean): number {
  const [t, setT] = useState(0)
  useEffect(() => {
    if (!enabled) return
    const start = performance.now()
    let frame = requestAnimationFrame(function tick(now) {
      setT(Math.floor(((now - start) % LOOP_MS) / 100) * 100)
      frame = requestAnimationFrame(tick)
    })
    return () => cancelAnimationFrame(frame)
  }, [enabled])
  return t
}

export interface FlightSceneProps {
  /** Plays while Hi is the live step; history shows the finished frame. */
  live: boolean
}

/** The Hi bubble's picture of the product: an AI app asks, neo does it. */
export function FlightScene({ live }: FlightSceneProps) {
  const reducedMotion = usePrefersReducedMotion()
  const playing = live && !reducedMotion
  const t = useLoopTime(playing)
  const frame = playing ? flightSceneFrameAt(t) : FINAL_FRAME
  const fade = 'transition-opacity duration-400 ease-out'

  return (
    <div
      role="img"
      aria-label="Claude Code asks neo for the cheapest one-way flight to NYC next Friday; neo checks five results on Google Flights and tags $142."
      className="relative h-[306px] w-[544px] max-w-full overflow-hidden rounded-[18px] border border-accent-tint-2 bg-secondary"
    >
      <div className="absolute top-[18px] left-[182px] flex h-[270px] w-[344px] flex-col overflow-hidden rounded-[10px] border border-[#CBD5E1] bg-card shadow-[0_22px_44px_-24px_rgb(1_18_63/0.35)]">
        <div className="flex h-[30px] shrink-0 items-end border-[#DAE5FF] border-b bg-[#F1F5FD] pr-2.5 pl-[46px]">
          <div className="flex h-6 w-[190px] items-center gap-1.5 rounded-t-md border border-[#DAE5FF] border-b-0 bg-card pr-1.5 pl-2 font-medium text-[11px] text-ink-deep">
            <i className="size-3 shrink-0 rounded-[3px] bg-ink-deep" />
            <span className="flex-1">Flights</span>
            <span className="flex h-[18px] items-center gap-1 rounded-[9px] bg-secondary px-1.5 font-mono text-[11px] text-primary">
              <i
                className={cn(
                  'size-[5px] rounded-full transition-colors duration-400',
                  frame.status === 'done' ? 'bg-primary' : 'bg-[#F5B957]',
                )}
              />
              claude
            </span>
          </div>
        </div>
        <div className="flex h-8 shrink-0 items-center gap-2 border-[#EEF2F8] border-b pr-2.5 pl-[46px]">
          <div className="flex h-[22px] flex-1 items-center gap-1.5 rounded-md bg-[#F5F8FF] px-2 font-mono text-[11px] text-ink-deep">
            <Lock aria-hidden className="size-[9px] text-ink-3" />
            google.com/travel/flights
          </div>
          <NeoMark className="size-4" />
        </div>
        <div className="flex-1 pt-2.5 pr-3.5 pl-[50px]">
          <div
            className={cn(
              'flex h-[26px] items-center justify-between pb-1',
              fade,
              frame.rows ? 'opacity-100' : 'opacity-0',
            )}
          >
            <b className="font-bold text-[13px] text-ink-deep">
              To New York · Fri
            </b>
            <span className="flex h-5 items-center gap-1 rounded-[10px] bg-secondary px-2 font-mono text-[11px] text-primary">
              Checked <b>{frame.checked}/5</b>
            </span>
          </div>
          <div className="relative">
            {FLIGHTS.map(([time, stops, price], index) => (
              <div
                key={time}
                className={cn(
                  'flex h-8 items-center gap-2.5 border-[#EEF2F8] border-b px-1.5 text-ink-deep last:border-b-0',
                  fade,
                  frame.rows ? 'opacity-100' : 'opacity-0',
                )}
              >
                <span className="w-[50px] shrink-0 font-semibold text-[12px]">
                  {time}
                </span>
                <span className="w-12 shrink-0 text-[11px] text-ink-3">
                  {stops}
                </span>
                <i className="h-1.5 flex-1 rounded-[3px] bg-[#E8EEF8]" />
                <span className="flex w-[70px] shrink-0 justify-end">
                  {index === CHEAPEST_ROW && (
                    <span
                      className={cn(
                        'flex h-[18px] items-center rounded-[9px] bg-primary px-1.5 font-mono font-semibold text-[11px] text-primary-foreground',
                        fade,
                        frame.tagged ? 'opacity-100' : 'opacity-0',
                      )}
                    >
                      Cheapest
                    </span>
                  )}
                </span>
                <span className="w-[34px] shrink-0 text-right font-bold text-[12px]">
                  {price}
                </span>
              </div>
            ))}
            <div
              aria-hidden
              style={{ top: 1 + Math.max(frame.ring, 0) * 32 }}
              className={cn(
                'pointer-events-none absolute -left-1 h-[30px] w-[286px] rounded-lg border-[1.5px] border-primary shadow-[0_0_0_3px_rgb(2_84_236/0.12)] transition-[top,opacity] duration-400 ease-out',
                frame.ring >= 0 ? 'opacity-100' : 'opacity-0',
              )}
            />
          </div>
        </div>
      </div>

      {/* Claude Code in a terminal asks; neo does it (owner, round 9: the
          terminal reads better here than a drawing of the Claude app). */}
      <div className="absolute top-[60px] left-[18px] flex h-[200px] w-[206px] flex-col overflow-hidden rounded-[10px] bg-ink-deep shadow-[0_26px_50px_-24px_rgb(1_18_63/0.6)]">
        <div className="relative flex h-[26px] shrink-0 items-center border-[#12285F] border-b px-2.5">
          <span className="flex gap-[5px]">
            <i className="size-2 rounded-full bg-[#EAF1FF]/18" />
            <i className="size-2 rounded-full bg-[#EAF1FF]/18" />
            <i className="size-2 rounded-full bg-[#EAF1FF]/18" />
          </span>
          <span className="absolute inset-x-0 top-[7px] text-center font-medium font-mono text-[#7E8FB8] text-[11px] leading-3 tracking-[0.02em]">
            Claude Code
          </span>
        </div>
        <div className="flex flex-col gap-2.5 p-3">
          <div
            className={cn(
              'ml-3.5 rounded-[10px_10px_3px_10px] bg-[#EAF1FF]/10 px-2.5 py-2 font-medium text-[12px] text-white leading-4 transition-[opacity,transform] duration-500 ease-out',
              frame.asked ? 'opacity-100' : 'translate-y-1.5 opacity-0',
            )}
          >
            {HI_PROMPT}
          </div>
          {/* Paper keeps "Working in neo" above the answer once it lands. */}
          <div
            className={cn(
              'flex items-center gap-1.5 font-mono text-[#7E8FB8] text-[11px] leading-4',
              fade,
              frame.status === 'idle' ? 'opacity-0' : 'opacity-100',
            )}
          >
            <i
              className={cn(
                'size-1.5 shrink-0 rounded-full bg-[#F5B957]',
                frame.status === 'working' && 'animate-pulse-dot',
              )}
            />
            Working in neo · {frame.seconds}s
          </div>
          <div
            className={cn(
              'flex items-start gap-[7px] font-semibold text-[12.5px] text-white leading-[17px]',
              fade,
              frame.status === 'done' ? 'opacity-100' : 'opacity-0',
            )}
          >
            <span className="mt-0.5 grid size-3.5 shrink-0 place-items-center rounded-full bg-white/14">
              <Check aria-hidden className="size-[9px]" strokeWidth={3.5} />
            </span>
            Cheapest is $142, nonstop, Fri 7:05 am.
          </div>
        </div>
      </div>
    </div>
  )
}
