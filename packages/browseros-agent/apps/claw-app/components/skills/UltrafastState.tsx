/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { SkillUltrafast } from '@browseros/claw-api'
import type { ReactNode } from 'react'
import { UltrafastWordmark } from '@/components/audit/UltrafastMarks'
import { cn } from '@/lib/utils'
import {
  type UltrafastMark,
  ultrafastMark,
} from '@/screens/skills/skills.helpers'

const WORD_TONE: Record<UltrafastMark['tone'], string> = {
  amber: 'text-amber-ink',
  red: 'text-red-ink',
  muted: 'text-ink-3',
}

/** The wordmark for an ultrafast skill, then its state in words when it isn't verified. */
export function UltrafastState({
  ultrafast,
  wordmarkClassName,
  children,
  className,
}: {
  ultrafast: SkillUltrafast | undefined
  wordmarkClassName?: string
  /** Shown after the wordmark while the skill is verified, such as its speed. */
  children?: ReactNode
  className?: string
}) {
  const mark = ultrafastMark(ultrafast)
  if (!mark) return null
  return (
    <span
      className={cn(
        'inline-flex shrink-0 items-baseline gap-1.5 whitespace-nowrap',
        className,
      )}
    >
      <UltrafastWordmark
        className={cn(wordmarkClassName, mark.dim && 'text-ink-3')}
      />{' '}
      {mark.word ? (
        <span className={cn('font-semibold', WORD_TONE[mark.tone])}>
          {mark.word}
        </span>
      ) : (
        children
      )}
    </span>
  )
}
