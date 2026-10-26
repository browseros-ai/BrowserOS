/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { cn } from '@/lib/utils'
import { HarnessIcon } from './HarnessIcon'
import { agentIdentity } from './harness.types'

interface AgentMarkProps {
  /** The session's harness name, when the server knows it. */
  harness?: string
  /** The session's agent label or slug, such as "claude". */
  label?: string
  /** Shown as a dot when the agent has no mark of its own. */
  color?: string
  className?: string
}

/** A connected agent's mark and name, drawn the way Connected agents draws it. */
export function AgentMark({
  harness,
  label,
  color,
  className,
}: AgentMarkProps) {
  const agent = agentIdentity(harness, label)
  return (
    <span className={cn('inline-flex min-w-0 items-center gap-1.5', className)}>
      {agent.harness ? (
        <HarnessIcon harness={agent.harness} className="size-4 shrink-0" />
      ) : (
        <span
          aria-hidden
          className="size-2.5 shrink-0 rounded-full"
          style={{ background: color ?? 'currentColor' }}
        />
      )}
      <span className="truncate">{agent.name}</span>
    </span>
  )
}
