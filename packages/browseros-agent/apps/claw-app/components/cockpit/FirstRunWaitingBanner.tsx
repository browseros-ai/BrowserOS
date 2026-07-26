/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Live "waiting for your first run" indicator. Shown when the
 * cockpit is in the `waiting` state OR when the operator has just
 * copied the starter prompt (armed state). Pulses a small accent
 * dot to signal that the surface is listening.
 */

import { motion } from 'motion/react'

interface FirstRunWaitingBannerProps {
  message: string
}

export function FirstRunWaitingBanner({ message }: FirstRunWaitingBannerProps) {
  // role="status" gives this container an implicit `aria-live="polite"`,
  // so assistive tech announces the message the moment the banner mounts
  // (e.g. right after the operator copies the starter prompt).
  return (
    <div
      role="status"
      className="flex items-center gap-3 rounded-xl border border-border-2 bg-bg-sunken px-4 py-3 text-[13px] text-ink-2"
    >
      <PulsingDot />
      <span>{message}</span>
    </div>
  )
}

function PulsingDot() {
  return (
    <div className="relative flex size-3 shrink-0 items-center justify-center">
      <motion.span
        aria-hidden
        className="absolute inline-flex size-3 rounded-full bg-accent"
        animate={{ scale: [1, 1.9, 1], opacity: [0.55, 0, 0.55] }}
        transition={{
          duration: 1.6,
          repeat: Number.POSITIVE_INFINITY,
          ease: 'easeOut',
        }}
      />
      <span className="relative inline-flex size-2 rounded-full bg-accent" />
    </div>
  )
}
