/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { ArrowRight } from 'lucide-react'
import { HarnessIcon } from '@/components/harness/HarnessIcon'
import type { Harness } from '@/components/harness/harness.types'
import { cn } from '@/lib/utils'

/** Each agent mark after the first pops in this much later, so they arrive one by one. */
const FACE_STAGGER_MS = 60

/**
 * The connected agents as small overlapping marks, at most `limit` of them. With `pop`, they
 * spring in one after another when they mount.
 */
export function AgentFaces({
  agents,
  limit = 4,
  pop = false,
  className,
}: {
  agents: readonly Harness[]
  limit?: number
  pop?: boolean
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
          style={
            pop ? { animationDelay: `${index * FACE_STAGGER_MS}ms` } : undefined
          }
          className={cn(
            'grid size-6 place-items-center rounded-full bg-card shadow-[0_0_0_2px_var(--color-card),0_0_0_3px_var(--color-border-2)]',
            index > 0 && '-ml-1.5',
            pop && 'animate-copy-face',
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
        <ArrowRight
          className="size-[13px] animate-copy-nudge"
          strokeWidth={2.2}
        />
      </span>
      <span className="truncate">Paste it into your agent</span>
      <AgentFaces agents={agents} pop className="ml-auto" />
    </span>
  )
}
