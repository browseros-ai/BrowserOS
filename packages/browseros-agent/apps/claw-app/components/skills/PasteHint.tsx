/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { ArrowRight } from 'lucide-react'
import { HarnessIcon } from '@/components/harness/HarnessIcon'
import type { Harness } from '@/components/harness/harness.types'
import { cn } from '@/lib/utils'

/** The connected agents as small overlapping marks, at most `limit` of them. */
export function AgentFaces({
  agents,
  limit = 4,
  className,
}: {
  agents: readonly Harness[]
  limit?: number
  className?: string
}) {
  if (!agents.length) return null
  const shown = agents.slice(0, limit)
  return (
    <span
      role="img"
      aria-label={shown.join(', ')}
      className={cn('flex shrink-0', className)}
    >
      {shown.map((harness, index) => (
        <span
          key={harness}
          className={cn(
            'grid size-6 place-items-center rounded-full bg-card shadow-[0_0_0_2px_var(--color-card),0_0_0_3px_var(--color-border-2)]',
            index > 0 && '-ml-1.5',
          )}
        >
          <HarnessIcon harness={harness} className="size-[13px]" />
        </span>
      ))}
    </span>
  )
}

/** What to do after a copy: paste it into an agent, with the agents it can go to. */
export function PasteHint({
  agents,
  className,
}: {
  agents: readonly Harness[]
  className?: string
}) {
  return (
    <span
      className={cn(
        'flex min-w-0 items-center gap-2 font-semibold text-[13.5px] text-cyanotype-ink',
        className,
      )}
    >
      <span
        aria-hidden
        className="grid size-[22px] shrink-0 place-items-center rounded-full bg-accent-tint text-primary"
      >
        <ArrowRight className="size-[13px]" strokeWidth={2.2} />
      </span>
      <span className="truncate">Paste it into your agent</span>
      <AgentFaces agents={agents} className="ml-auto" />
    </span>
  )
}
