/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Zap } from 'lucide-react'
import type { ReactNode } from 'react'
import {
  AnimatedBadge,
  type AnimatedBadgeSize,
} from '@/components/motion/animated-badge'

/** A chip saying what ultrafast did in a task, in the words the caller gives it. */
export function UltrafastRunBadge({
  children,
  size = 'sm',
  className,
}: {
  children: ReactNode
  size?: AnimatedBadgeSize
  className?: string
}) {
  return (
    <AnimatedBadge
      status="info"
      size={size}
      icon={
        <Zap aria-hidden className={size === 'md' ? 'size-3.5' : 'size-3'} />
      }
      className={className}
    >
      {children}
    </AnimatedBadge>
  )
}
