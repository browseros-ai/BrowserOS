/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { useEffect, useId, useRef, useState } from 'react'
import { HarnessIcon } from '@/components/harness/HarnessIcon'
import type { Harness } from '@/components/harness/harness.types'
import { cn } from '@/lib/utils'
import { FIRST_RUN_COPY } from '@/screens/cockpit/cockpit-onboarding.helpers'

/** At most this many marks show; the rest collapse into "+N". */
const VISIBLE_MARKS = 3

const MARK =
  'flex size-[30px] items-center justify-center rounded-full border-2 border-bg-canvas shadow-[0_0_0_1px_var(--color-border-2)]'

export interface ConnectedAppsProps {
  harnesses: readonly Harness[]
}

/**
 * The connected AI apps as overlapping marks, so seven apps read as calmly as
 * two (owner, round 7), then "Connected". The group expands into a popover
 * listing every app, with the restart hint as its footer (final Paper):
 * hovering or focusing it peeks, clicking pins it open, and Escape or a click
 * outside closes it. The list is not interactive, so focus stays on the
 * trigger, which names every app for screen readers.
 */
export function ConnectedApps({ harnesses }: ConnectedAppsProps) {
  const [peeking, setPeeking] = useState(false)
  const [pinned, setPinned] = useState(false)
  const open = peeking || pinned
  const wrapRef = useRef<HTMLDivElement>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)
  // Escape hands focus back to the trigger; that focus must not peek again.
  const refocusing = useRef(false)
  const popoverId = useId()
  const shown = harnesses.slice(0, VISIBLE_MARKS)
  const more = harnesses.length - shown.length

  useEffect(() => {
    if (!open) return
    const close = () => {
      setPeeking(false)
      setPinned(false)
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      close()
      refocusing.current = true
      triggerRef.current?.focus()
      refocusing.current = false
    }
    const onPointerDown = (event: Event) => {
      if (!wrapRef.current?.contains(event.target as Node)) close()
    }
    document.addEventListener('keydown', onKeyDown)
    document.addEventListener('pointerdown', onPointerDown)
    return () => {
      document.removeEventListener('keydown', onKeyDown)
      document.removeEventListener('pointerdown', onPointerDown)
    }
  }, [open])

  return (
    <div className="flex min-w-0 items-center gap-2.5">
      {/* biome-ignore lint/a11y/noStaticElementInteractions: hover only peeks; the button inside is the accessible control */}
      <div
        ref={wrapRef}
        className="relative shrink-0"
        onMouseEnter={() => setPeeking(true)}
        onMouseLeave={() => setPeeking(false)}
        onFocus={() => {
          if (!refocusing.current) setPeeking(true)
        }}
        onBlur={(event) => {
          if (!wrapRef.current?.contains(event.relatedTarget as Node)) {
            setPeeking(false)
          }
        }}
      >
        <button
          ref={triggerRef}
          type="button"
          aria-expanded={open}
          aria-controls={open ? popoverId : undefined}
          aria-label={`${FIRST_RUN_COPY.connectedApps}: ${harnesses.join(', ')}`}
          onClick={() => setPinned(!pinned)}
          className={cn(
            'flex cursor-pointer items-center rounded-2xl outline-none transition-shadow focus-visible:ring-4 focus-visible:ring-accent-tint-2',
            open && 'ring-4 ring-accent-tint-2',
          )}
        >
          {/* The leftmost mark sits on top, as in Paper. */}
          {shown.map((harness, index) => (
            <span
              key={harness}
              style={{ zIndex: shown.length - index + 1 }}
              className={cn(MARK, 'relative bg-card', index > 0 && '-ml-1.5')}
            >
              <HarnessIcon harness={harness} className="size-4" />
            </span>
          ))}
          {more > 0 && (
            <span
              className={cn(
                MARK,
                'relative -ml-1.5 bg-accent-tint font-bold text-[11.5px] text-accent leading-[14px]',
              )}
            >
              +{more}
            </span>
          )}
        </button>
        {open && (
          <div
            id={popoverId}
            className="absolute top-[calc(100%+15px)] -left-2 z-30 flex w-[252px] animate-fade-in flex-col rounded-[14px] border border-border-2 bg-card p-2 shadow-[0_1px_2px_rgb(1_18_63/0.06),0_22px_44px_-18px_rgb(1_18_63/0.38)]"
          >
            <div className="flex items-baseline justify-between px-2.5 pt-1.5 pb-2 font-semibold text-[13px] text-cyanotype-ink leading-[18px]">
              {FIRST_RUN_COPY.connectedApps}
              <span className="font-medium text-ink-3">{harnesses.length}</span>
            </div>
            <ul aria-label={FIRST_RUN_COPY.connectedApps} className="m-0 p-0">
              {harnesses.map((harness) => (
                <li
                  key={harness}
                  className="flex h-9 items-center gap-2.5 rounded-[9px] px-2.5 font-medium text-[14px] text-cyanotype-ink leading-5"
                >
                  <span className="flex size-5 shrink-0 items-center justify-center">
                    <HarnessIcon harness={harness} className="size-4" />
                  </span>
                  {harness}
                </li>
              ))}
            </ul>
            <p className="m-0 border-border-2 border-t px-2.5 pt-2.5 pb-1 text-[12.5px] text-ink-3 leading-4">
              {FIRST_RUN_COPY.restartHint}
            </p>
          </div>
        )}
      </div>
      <span className="font-semibold text-[14px] text-cyanotype-ink leading-[18px]">
        {FIRST_RUN_COPY.connected}
      </span>
    </div>
  )
}
