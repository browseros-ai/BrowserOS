/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The flat ink surface the ultrafast card and its Details share, and the pieces set on it.
 */

import { cn } from '@/lib/utils'
import { meterParts } from './energy-meter.helpers'

/** Cyanotype ink with off-white type; Signal Blue is the one accent on it. */
export const INK = {
  panel: 'bg-[#0c2742] text-[#eef3ff] dark:bg-[#101d33]',
  ink: 'text-[#eef3ff]',
  ink2: 'text-[rgb(238_243_255/0.74)]',
  ink3: 'text-[rgb(238_243_255/0.6)]',
  label: 'text-[rgb(238_243_255/0.66)]',
  line: 'border-[rgb(238_243_255/0.14)]',
  accent: 'text-[#9cc0ff]',
} as const

export const PANEL_SHADOW =
  'shadow-[0_1px_2px_rgb(1_18_63/0.12),0_12px_28px_-18px_rgb(1_18_63/0.45)] dark:shadow-[inset_0_0_0_1px_rgb(238_243_255/0.08)]'

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
        'whitespace-nowrap font-bold leading-[0.95] tracking-[-0.055em] [font-variant-numeric:lining-nums_proportional-nums]',
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

/** "36 minutes", "49%" or "3 sec" as a figure and a smaller unit; anything else, such as "None yet", as words. */
function factParts(value: string): { figure: string; unit: string } | null {
  const match = /^([\d.,]+)(%| .+)?$/.exec(value)
  return match ? { figure: match[1] ?? value, unit: match[2] ?? '' } : null
}

/** One figure on the ink with its label under it, for a `dl`. */
export function InkFact({ label, value }: { label: string; value: string }) {
  const parts = factParts(value)
  return (
    <div className="flex min-w-0 flex-col-reverse gap-0.5">
      <dt className={cn('text-[13px]', INK.label)}>{label}</dt>
      <dd
        className={cn(
          'm-0 whitespace-nowrap leading-[1.15] [font-variant-numeric:lining-nums_proportional-nums]',
          INK.ink,
          parts
            ? 'font-bold text-[24px] tracking-[-0.02em]'
            : 'font-semibold text-[17px] tracking-[-0.01em]',
        )}
      >
        {parts ? (
          <>
            {parts.figure}
            {parts.unit ? (
              <small
                className={cn(
                  'font-semibold text-[13.5px] tracking-normal',
                  INK.ink2,
                )}
              >
                {parts.unit}
              </small>
            ) : null}
          </>
        ) : (
          value
        )}
      </dd>
    </div>
  )
}
