/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Replay viewport for the audit page. The original scaffold drew a
 * fake browser chrome with a tinted page region and a caption pill.
 * This version mounts rrweb-player inside that chrome so the actual
 * recorded DOM mutations play back. A tab selector above the chrome
 * lets the operator switch between tabs the agent drove in the same
 * session.
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
  /** Distinct tabPageIds the operator can pick from. */
  tabPageIds: number[]
  selectedTabPageId: number | null
  onTabPageIdChange: (id: number) => void
  /** Called once the rrweb-player has mounted with usable controls. */
  onPlayerReady: (handle: ReplayPlayerHandle) => void
}

export function ReplayViewport({
  site,
  frame,
  events,
  tabPageIds,
  selectedTabPageId,
  onTabPageIdChange,
  onPlayerReady,
}: ReplayViewportProps) {
  return (
    <div className="relative flex flex-1 flex-col overflow-hidden rounded-2xl border border-border-2 bg-card shadow-sm">
      <Chrome
        site={site}
        tabPageIds={tabPageIds}
        selectedTabPageId={selectedTabPageId}
        onTabPageIdChange={onTabPageIdChange}
      />
      <div className="relative flex flex-1 items-stretch justify-center overflow-hidden bg-bg-sunken">
        <PlayerCanvas events={events} onReady={onPlayerReady} />
        {frame && <Caption frame={frame} />}
      </div>
    </div>
  )
}

interface ChromeProps {
  site: string
  tabPageIds: number[]
  selectedTabPageId: number | null
  onTabPageIdChange: (id: number) => void
}

function Chrome({
  site,
  tabPageIds,
  selectedTabPageId,
  onTabPageIdChange,
}: ChromeProps) {
  return (
    <div className="flex h-9 shrink-0 items-center gap-2 border-border border-b bg-bg-sunken px-3">
      <span className="flex gap-1.5">
        <span className="size-2.5 rounded-full bg-[#FF5F57]" />
        <span className="size-2.5 rounded-full bg-[#FEBC2E]" />
        <span className="size-2.5 rounded-full bg-[#28C840]" />
      </span>
      <div className="ml-3 flex h-6 flex-1 items-center gap-2 rounded-md border border-border-2 bg-card px-3 font-mono text-ink-2 text-xs">
        <Lock className="size-3 text-ink-3" />
        <span className="truncate">{site}</span>
      </div>
      {tabPageIds.length > 1 && (
        <select
          aria-label="Tab to replay"
          value={selectedTabPageId ?? ''}
          onChange={(e) => onTabPageIdChange(Number(e.target.value))}
          className="h-6 rounded-md border border-border-2 bg-card px-2 font-mono text-[11px] text-ink-2"
        >
          {tabPageIds.map((id) => (
            <option key={id} value={id}>
              Tab {id}
            </option>
          ))}
        </select>
      )}
      <span className="rounded-full bg-bg-sunken px-2 py-0.5 font-bold text-[10px] text-ink-3 uppercase tracking-wide">
        rrweb
      </span>
    </div>
  )
}

interface PlayerCanvasProps {
  events: ReplayEvent[]
  onReady: (handle: ReplayPlayerHandle) => void
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
      // biome-ignore lint/suspicious/noExplicitAny: rrweb's event
      // union is wide; we trust the recorder's output shape.
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
    onReady({
      // `pause(timeOffset)` jumps to that time and pauses. We pause
      // rather than play so our scaffold's playback clock stays the
      // source of truth.
      goto: (ms) => replayer.pause(ms),
      play: () => replayer.play(replayer.getCurrentTime()),
      pause: () => replayer.pause(replayer.getCurrentTime()),
    })
    return () => {
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
      className="flex flex-1 items-center justify-center"
      data-replay-canvas
    />
  )
}

function Caption({ frame }: { frame: ReplayFrame }) {
  const verb = VERB_META[frame.verb]
  const kind = KIND_STYLE[frame.kind]
  return (
    <div className="absolute bottom-5 left-1/2 z-10 flex max-w-[82%] -translate-x-1/2 items-center gap-2.5 rounded-full bg-[#1B1A17]/90 px-4 py-2 shadow-xl backdrop-blur">
      <span
        className={cn(
          'flex size-5 items-center justify-center rounded-md text-white',
          kind.dotClass,
        )}
      >
        <verb.Icon className="size-3" />
      </span>
      <span className="truncate font-semibold text-[#EDEAE2] text-xs">
        {frame.caption}
      </span>
    </div>
  )
}
