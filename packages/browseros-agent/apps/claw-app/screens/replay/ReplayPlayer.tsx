/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * One rrweb stream and its imperative lifetime. ReplayViewport composes two of
 * these primitives, but this component knows nothing about active/standby
 * roles so every constructed Replayer still has one owner and one cleanup.
 */

import { useEffect, useRef } from 'react'
import type { ReplayEvent } from '@/modules/api/replay.hooks'

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

export interface ReplayPlayerProps {
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
  const meta = events.find((event) => event.type === 4)
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
export function ReplayPlayer({ events, onReady }: ReplayPlayerProps) {
  const mountRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const mount = mountRef.current
    if (!mount || events.length < 2) return

    const rrwebEvents = events.map((event) => ({
      type: event.type,
      data: event.data,
      timestamp: event.ts,
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
    } catch (error) {
      // eslint-disable-next-line no-console
      console.warn('[browseros-claw replay] Replayer ctor threw', error)
      return
    }

    const { width: recordedWidth, height: recordedHeight } =
      readRecordedSize(events)
    const wrapper = mount.querySelector<HTMLElement>('.replayer-wrapper')
    let observer: ResizeObserver | null = null
    if (wrapper) {
      wrapper.style.position = 'absolute'
      wrapper.style.transformOrigin = 'top left'
      const applyScale = (): void => {
        const rect = mount.getBoundingClientRect()
        if (rect.width === 0 || rect.height === 0) return
        const scale = Math.min(
          rect.width / recordedWidth,
          rect.height / recordedHeight,
        )
        const scaledWidth = recordedWidth * scale
        const scaledHeight = recordedHeight * scale
        wrapper.style.transform = `scale(${scale})`
        wrapper.style.left = `${Math.max(0, (rect.width - scaledWidth) / 2)}px`
        wrapper.style.top = `${Math.max(0, (rect.height - scaledHeight) / 2)}px`
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
      className="relative h-full w-full overflow-hidden"
      data-replay-canvas
    />
  )
}
