/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { SessionStatus } from '@browseros/claw-api'
import { Check, CircleAlert, Hand } from 'lucide-react'
import {
  AnimatedBadge,
  type AnimatedBadgeSize,
} from '@/components/motion/animated-badge'
import { SESSION_STATUS_BADGE } from '@/lib/status'

const ICON_SIZE: Record<AnimatedBadgeSize, string> = {
  sm: 'size-3',
  md: 'size-3.5',
}

/** A task's status in plain words, with an icon that says the same thing. */
export function SessionStatusBadge({
  status,
  size = 'sm',
  className,
}: {
  status: SessionStatus
  size?: AnimatedBadgeSize
  className?: string
}) {
  const meta = SESSION_STATUS_BADGE[status]
  const iconClass = ICON_SIZE[size]
  // Working keeps beUI's own loading spinner.
  const icon =
    status === 'done' ? (
      <Check aria-hidden className={iconClass} />
    ) : status === 'cancelled' ? (
      <Hand aria-hidden className={iconClass} />
    ) : status === 'failed' ? (
      <CircleAlert aria-hidden className={iconClass} />
    ) : undefined
  return (
    <AnimatedBadge
      status={meta.badge}
      size={size}
      icon={icon}
      className={className}
    >
      {meta.label}
    </AnimatedBadge>
  )
}
