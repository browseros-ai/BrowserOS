/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
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
  seek(ms: number): void
  play(ms: number): void
  pause(): void
  setSpeed(speed: number): void
  getCurrentTime(): number
}

interface ReplayViewportProps {
  site: string
  /** The frame whose caption is currently displayed in the overlay. */
  frame: ReplayFrame | undefined
  /** rrweb events for the currently selected document segment. */
  events: readonly ReplayEvent[]
  /** Called when the rrweb Replayer mounts or is destroyed. */
  onPlayerReady: (handle: ReplayPlayerHandle | null) => void
}

/** Displays the selected tab's rrweb replay inside the audit browser chrome. */
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
  events: readonly ReplayEvent[]
  onReady: (handle: ReplayPlayerHandle | null) => void
}

/**
 * Fallback DOM viewport for pages that never emitted a meta event.
 * rrweb ALWAYS emits type 4 as its first event under normal
 * conditions, so this is defensive.
 */
const DEFAULT_RECORDED_SIZE = { width: 1280, height: 720 }
// rrweb casts events strictly before the target; 0ms can leave the first
// snapshot blank while paused.
const MIN_RENDER_SEEK_MS = 1

function readRecordedSize(events: readonly ReplayEvent[]): {
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

/** Mounts rrweb's imperative Replayer and exposes the narrow playback handle. */
function PlayerCanvas({ events, onReady }: PlayerCanvasProps) {
  const mountRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const mount = mountRef.current
    if (!mount) return
    if (events.length < 2) return

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
      seek: (ms) => replayer.pause(Math.max(MIN_RENDER_SEEK_MS, ms)),
      play: (ms) => replayer.play(ms),
      pause: () => replayer.pause(),
      setSpeed: (speed) => replayer.setConfig({ speed }),
      getCurrentTime: () => replayer.getCurrentTime(),
    })
    return () => {
      onReady(null)
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
      <span className="truncate font-semibold text-white/90 text-xs">
        {frame.caption}
      </span>
    </div>
  )
}
