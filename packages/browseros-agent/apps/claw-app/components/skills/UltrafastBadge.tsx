/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { SkillUltrafast } from '@browseros/claw-api'
import { Zap } from 'lucide-react'
import {
  AnimatedBadge,
  type AnimatedBadgeSize,
} from '@/components/motion/animated-badge'
import { ultrafastBadge } from '@/screens/skills/skills.helpers'

/** Whether a skill runs ultrafast, needs a repair, or runs the normal way. */
export function UltrafastBadge({
  ultrafast,
  size = 'sm',
  className,
}: {
  ultrafast: SkillUltrafast | undefined
  size?: AnimatedBadgeSize
  className?: string
}) {
  const meta = ultrafastBadge(ultrafast)
  if (!meta) return null
  return (
    <AnimatedBadge
      status={meta.tone}
      size={size}
      icon={<Zap aria-hidden className="size-3" />}
      className={className}
    >
      {meta.label}
    </AnimatedBadge>
  )
}
