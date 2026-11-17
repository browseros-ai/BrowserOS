/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { cn } from '@/lib/utils'

/**
 * The margin rule along the left edge of an ultrafast task's card. Between two ultrafast rows it
 * reaches over the divider, so a run of them reads as one rule.
 */
export function UltrafastRail({ className }: { className?: string }) {
  return (
    <span
      aria-hidden
      data-ultrafast-rail
      className={cn(
        'pointer-events-none absolute inset-y-0 left-0 w-0.5 bg-primary [[data-ultrafast]+[data-ultrafast]>&]:-top-px',
        className,
      )}
    />
  )
}

/** "ultrafast" in its bold italic, then what ultrafast did, in words. */
export function UltrafastNote({
  children,
  className,
}: {
  children: string
  className?: string
}) {
  return (
    <span className={cn('text-[13.5px] text-ink-2', className)}>
      <span className="mr-1.5 font-extrabold text-accent-ink italic tracking-[-0.01em]">
        ultrafast
      </span>
      {children}
    </span>
  )
}
