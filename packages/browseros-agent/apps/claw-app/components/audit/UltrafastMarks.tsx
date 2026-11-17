/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Zap } from 'lucide-react'
import { cn } from '@/lib/utils'

/** The glowing ink rail along the left edge of an ultrafast task's card. */
export function UltrafastRail({ className }: { className?: string }) {
  return (
    <span
      aria-hidden
      data-ultrafast-rail
      className={cn(
        'pointer-events-none absolute top-2.5 bottom-2.5 left-0 w-1 rounded-r-full bg-[linear-gradient(180deg,#9cc0ff,#2f6fe0_60%,#020d33)] shadow-[0_0_12px_rgb(82_144_255/0.7)]',
        className,
      )}
    />
  )
}

/** The lightning medallion that marks an ultrafast task. */
export function UltrafastMark({
  size = 'sm',
  className,
}: {
  size?: 'sm' | 'md'
  className?: string
}) {
  return (
    <span
      aria-hidden
      data-ultrafast-mark
      className={cn(
        'grid shrink-0 place-items-center rounded-full bg-[radial-gradient(circle_at_35%_30%,#2f6fe0,#06205f_70%)] text-white shadow-[0_0_0_1.5px_rgb(255_255_255/0.85),0_0_14px_rgb(82_144_255/0.9)]',
        // Under sm the card's picture is 72px wide; a smaller mark keeps clear of the site icon.
        size === 'md' ? 'size-6' : 'size-4 sm:size-5',
        className,
      )}
    >
      <Zap className={size === 'md' ? 'size-3' : 'size-2 sm:size-2.5'} />
    </span>
  )
}
