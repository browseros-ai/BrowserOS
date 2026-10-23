/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { cn } from '@/lib/utils'
import { HarnessIcon } from './HarnessIcon'
import { asHarness } from './harness.types'

interface AgentMarkProps {
  harness?: string
  /** Shown when the harness has no mark of its own. */
  color?: string
  className?: string
}

/** A connected agent's logo in a small round badge, or its colour dot. */
export function AgentMark({ harness, color, className }: AgentMarkProps) {
  const known = asHarness(harness)
  return (
    <span
      aria-hidden
      className={cn(
        'inline-grid size-[22px] shrink-0 place-items-center rounded-full border border-border-2 bg-card',
        className,
      )}
    >
      {known ? (
        <HarnessIcon harness={known} className="size-[13px]" />
      ) : (
        <span
          className="size-2 rounded-full"
          style={{ background: color ?? 'currentColor' }}
        />
      )}
    </span>
  )
}
