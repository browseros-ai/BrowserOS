/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { HarnessIcon } from '@/components/harness/HarnessIcon'
import type { Harness } from '@/components/harness/harness.types'
import {
  MorphPopover,
  MorphPopoverContent,
  MorphPopoverTrigger,
} from '@/components/motion/popover-morph'
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
 * The connected AI agents as overlapping marks, so seven read as calmly as
 * two, then "Connected". The marks open a beUI morph popover listing every
 * agent, with the restart hint as its footer.
 */
export function ConnectedApps({ harnesses }: ConnectedAppsProps) {
  const shown = harnesses.slice(0, VISIBLE_MARKS)
  const more = harnesses.length - shown.length

  return (
    <div className="flex min-w-0 items-center gap-2.5">
      <MorphPopover>
        <MorphPopoverTrigger>
          <button
            type="button"
            aria-label={`${FIRST_RUN_COPY.connectedApps}: ${harnesses.join(', ')}`}
            className="flex shrink-0 cursor-pointer items-center rounded-2xl outline-none transition-shadow focus-visible:ring-4 focus-visible:ring-accent-tint-2"
          >
            {/* The leftmost mark sits on top. */}
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
        </MorphPopoverTrigger>
        <MorphPopoverContent
          side="bottom"
          align="start"
          sideOffset={12}
          className="w-[252px] border border-border-2 bg-card p-2"
        >
          <div className="flex items-baseline justify-between px-2.5 pt-1.5 pb-2 font-semibold text-[13px] text-cyanotype-ink leading-[18px]">
            {FIRST_RUN_COPY.connectedApps}
            <span className="font-medium text-ink-3">{harnesses.length}</span>
          </div>
          <ul aria-label={FIRST_RUN_COPY.connectedApps} className="m-0 p-0">
            {harnesses.map((harness) => (
              <li
                key={harness}
                className="flex h-9 items-center gap-2.5 rounded-md px-2.5 font-medium text-[14px] text-cyanotype-ink leading-5"
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
        </MorphPopoverContent>
      </MorphPopover>
      <span className="font-semibold text-[14px] text-cyanotype-ink leading-[18px]">
        {FIRST_RUN_COPY.connected}
      </span>
    </div>
  )
}
