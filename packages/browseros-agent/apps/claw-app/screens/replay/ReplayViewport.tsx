/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Replay viewport for the audit page. The original scaffold drew a
 * fake browser chrome with a tinted page region and a caption pill.
 * This version mounts rrweb-player inside that chrome so the actual
 * recorded DOM mutations play back. Tab selection lives at the
 * page level (see `Replay.tsx`) as a prominent shadcn Tabs bar; the
 * viewport itself just renders the currently-selected tab's events.
 *
 * The player's built-in controller is hidden via `showController:
 * false`; PlaybackTransport (see use-playback wiring) is the single
 * source of UI truth. Time sync between this player and the
 * scaffold's `usePlayback` clock is set up imperatively via the
 * `onPlayerReady` callback so the page-level component can drive
 * the player from the same scrub events the timeline already
 * dispatches.
 */

import { Lock } from 'lucide-react'
import { useEffect, useRef } from 'react'
import { cn } from '@/lib/utils'
import type { ReplayEvent, ReplayFrame } from '@/modules/api/replay.hooks'
import { KIND_STYLE, VERB_META } from './replay.helpers'

import 'rrweb-player/dist/style.css'
// We use rrweb's Replayer directly. The rrweb-player wrapper at v2.x
// publishes a broken bundle: its built JS has no `new Replayer(...)`
// call AND no import statement for @rrweb/replay, so the wrapper's
// Player.svelte never instantiates a Replayer (the `replayer` Svelte
// state stays undefined, the Controller `{#if replayer}` block never
// renders, the player-frame div stays empty). The rrweb package
// itself bundles Replayer cleanly; we mount it ourselves and skip
// the wrapper. rrweb-player's CSS is still imported for the
// `.replayer-wrapper` styling.
import { Replayer } from 'rrweb'

export interface ReplayPlayerHandle {
  goto(ms: number): void
  play(): void
  pause(): void
}

interface ReplayViewportProps {
  site: string
  /** The frame whose caption is currently displayed in the overlay. */
  frame: ReplayFrame | undefined
  /** rrweb events for the currently-selected tabPageId. */
  events: ReplayEvent[]
  /** Called once the rrweb-player has mounted with usable controls. */
  onPlayerReady: (handle: ReplayPlayerHandle) => void
}

export function ReplayViewport({
  site,
  frame,
  events,
  onPlayerReady,
}: ReplayViewportProps) {
  // Prefer the current frame's full URL so the address bar shows
  // exactly where the agent was at this instant. Falls back to the
  // task-level site (a hostname) when the frame carries no url
  // (e.g. `run`, `windows`, `tab_groups` dispatches).
  const addressBar = frame?.url ?? site
  return (
    <div className="relative flex flex-1 flex-col overflow-hidden rounded-2xl border border-border-2 bg-card shadow-sm">
      <Chrome url={addressBar} />
      <div className="relative flex flex-1 items-stretch justify-center overflow-hidden bg-bg-sunken">
        <PlayerCanvas events={events} onReady={onPlayerReady} />
        {frame && <Caption frame={frame} />}
      </div>
    </div>
  )
}

function Chrome({ url }: { url: string }) {
  return (
    <div className="flex h-9 shrink-0 items-center gap-2 border-border border-b bg-bg-sunken px-3">
      <span className="flex gap-1.5">
        <span className="size-2.5 rounded-full bg-[#FF5F57]" />
        <span className="size-2.5 rounded-full bg-[#FEBC2E]" />
        <span className="size-2.5 rounded-full bg-[#28C840]" />
      </span>
      <div className="ml-3 flex h-6 flex-1 items-center gap-2 rounded-md border border-border-2 bg-card px-3 font-mono text-ink-2 text-xs">
        <Lock className="size-3 text-ink-3" />
        <span className="truncate">{url}</span>
      </div>
    </div>
  )
}

interface PlayerCanvasProps {
  events: ReplayEvent[]
  onReady: (handle: ReplayPlayerHandle) => void
}

/**
 * Fallback DOM viewport for pages that never emitted a meta event.
 * rrweb ALWAYS emits type 4 as its first event under normal
 * conditions, so this is defensive.
 */
const DEFAULT_RECORDED_SIZE = { width: 1280, height: 720 }

function readRecordedSize(events: ReplayEvent[]): {
  width: number
  height: number
} {
  const meta = events.find((e) => e.type === 4)
  const data = meta?.data as { width?: unknown; height?: unknown } | undefined
  const width =
    typeof data?.width === 'number' && data.width > 0
      ? data.width
      : DEFAULT_RECORDED_SIZE.width
  const height =
    typeof data?.height === 'number' && data.height > 0
      ? data.height
      : DEFAULT_RECORDED_SIZE.height
  return { width, height }
}

function PlayerCanvas({ events, onReady }: PlayerCanvasProps) {
  const mountRef = useRef<HTMLDivElement>(null)
  // We deliberately use useEffect rather than deriving during render
  // because Replayer mounts into the DOM imperatively and its
  // cleanup needs to happen on unmount + on events-array swap (tab
  // change). Re-renders without a swap should NOT re-mount; the
  // ref-comparison guard below handles that.
  const lastEventsRef = useRef<ReplayEvent[] | null>(null)
  useEffect(() => {
    const mount = mountRef.current
    if (!mount) return
    if (events.length < 2) return
    if (lastEventsRef.current === events) return
    lastEventsRef.current = events

    // Strip the cockpit annotations so rrweb sees its canonical
    // {type, data, timestamp} shape.
    const rrwebEvents = events.map((e) => ({
      type: e.type,
      data: e.data,
      timestamp: e.ts,
      // biome-ignore lint/suspicious/noExplicitAny: rrweb's event union is wide; we trust the recorder's output shape.
    })) as any[]

    mount.replaceChildren()
    let replayer: Replayer
    try {
      replayer = new Replayer(rrwebEvents, {
        root: mount,
        speed: 1,
        skipInactive: false,
        showWarning: false,
      })
    } catch (err) {
      // eslint-disable-next-line no-console
      console.warn('[browseros-claw replay] Replayer ctor threw', err)
      return
    }

    // rrweb-player normally handles fit-to-container scaling; we
    // mount the raw Replayer (see notes at the top of the file) so
    // we do it ourselves. Read the recorded viewport from the meta
    // event, absolute-position the .replayer-wrapper at 0,0 of the
    // mount, and apply a uniform transform so it fits regardless of
    // the player pane's size. A ResizeObserver keeps the scale in
    // sync with layout changes (window resize, split-pane drags,
    // tab switch narrowing the viewport, etc.).
    const { width: recordedW, height: recordedH } = readRecordedSize(events)
    const wrapper = mount.querySelector<HTMLElement>('.replayer-wrapper')
    let observer: ResizeObserver | null = null
    if (wrapper) {
      wrapper.style.position = 'absolute'
      wrapper.style.transformOrigin = 'top left'
      const applyScale = (): void => {
        const rect = mount.getBoundingClientRect()
        if (rect.width === 0 || rect.height === 0) return
        const scale = Math.min(rect.width / recordedW, rect.height / recordedH)
        const scaledW = recordedW * scale
        const scaledH = recordedH * scale
        wrapper.style.transform = `scale(${scale})`
        wrapper.style.left = `${Math.max(0, (rect.width - scaledW) / 2)}px`
        wrapper.style.top = `${Math.max(0, (rect.height - scaledH) / 2)}px`
      }
      applyScale()
      observer = new ResizeObserver(applyScale)
      observer.observe(mount)
    }

    onReady({
      // `pause(timeOffset)` jumps to that time and pauses. We pause
      // rather than play so our scaffold's playback clock stays the
      // source of truth.
      goto: (ms) => replayer.pause(ms),
      play: () => replayer.play(replayer.getCurrentTime()),
      pause: () => replayer.pause(replayer.getCurrentTime()),
    })
    return () => {
      observer?.disconnect()
      try {
        replayer.destroy()
      } catch {
        // ignore; we're tearing down anyway
      }
      mount.replaceChildren()
    }
  }, [events, onReady])

  return (
    <div
      ref={mountRef}
      className="relative flex-1 overflow-hidden"
      data-replay-canvas
    />
  )
}

function Caption({ frame }: { frame: ReplayFrame }) {
  const verb = VERB_META[frame.verb]
  const kind = KIND_STYLE[frame.kind]
  return (
    <div className="absolute bottom-5 left-1/2 z-10 flex max-w-[82%] -translate-x-1/2 items-center gap-2.5 rounded-full bg-ink-deep/90 px-4 py-2 shadow-xl backdrop-blur">
      <span
        className={cn(
          'flex size-5 items-center justify-center rounded-md text-white',
          kind.dotClass,
        )}
      >
        <verb.Icon className="size-3" />
      </span>
      <span className="truncate font-semibold text-[#e9f2ea] text-xs">
        {frame.caption}
      </span>
    </div>
  )
}
