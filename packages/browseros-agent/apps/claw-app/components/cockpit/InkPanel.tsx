/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The flat ink surface the ultrafast meter and its Details header share, and the pieces set on it.
 */

import { cn } from '@/lib/utils'
import { meterParts } from './energy-meter.helpers'

/** Cyanotype ink with off-white type; Signal Blue is the one accent on it. */
export const INK = {
  panel: 'bg-[#0c2742] text-[#eef3ff] dark:bg-[#101d33]',
  ink: 'text-[#eef3ff]',
  ink2: 'text-[rgb(238_243_255/0.74)]',
  ink3: 'text-[rgb(238_243_255/0.6)]',
  line: 'border-[rgb(238_243_255/0.14)]',
  accent: 'text-[#9cc0ff]',
} as const

export const PANEL_SHADOW =
  'shadow-[0_1px_2px_rgb(1_18_63/0.12),0_12px_28px_-18px_rgb(1_18_63/0.45)] dark:shadow-[inset_0_0_0_1px_rgb(238_243_255/0.08)]'

/** The ultrafast word in its bold italic, the only mark ultrafast carries. */
export function Wordmark({ className }: { className?: string }) {
  return (
    <span
      className={cn(
        'font-extrabold text-[15px] italic tracking-[-0.01em]',
        INK.ink,
        className,
      )}
    >
      ultrafast
    </span>
  )
}

/** The total as one solid figure, its unit set smaller: "254k". */
export function BigNumber({
  tokens,
  className,
}: {
  tokens: number
  className?: string
}) {
  const { figure, unit } = meterParts(tokens)
  return (
    <span
      className={cn(
        'font-bold leading-[0.9] tracking-[-0.055em] [font-variant-numeric:lining-nums_proportional-nums]',
        INK.ink,
        className,
      )}
    >
      {figure}
      {unit ? (
        <span
          className={cn(
            'ml-[0.04em] font-semibold text-[0.52em] tracking-[-0.02em]',
            INK.ink2,
          )}
        >
          {unit}
        </span>
      ) : null}
    </span>
  )
}
