/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { CopyButton } from '@/components/copy/CopyButton'
import { cn } from '@/lib/utils'
import { toastSuccess } from '@/modules/toast/toast'
import { skillCommand } from '@/screens/skills/skills.helpers'

interface RunSkillButtonProps {
  name: string
  /** Outlined on a skill's page; ghost in the Skills list rows. */
  variant?: 'secondary' | 'ghost'
  className?: string
}

/** Copies the phrase a person pastes into one of their AI agents to run a skill. */
export function RunSkillButton({
  name,
  variant = 'secondary',
  className,
}: RunSkillButtonProps) {
  const phrase = skillCommand(name)
  return (
    <CopyButton
      text={phrase}
      what={phrase}
      variant={variant}
      size="sm"
      onCopied={() =>
        toastSuccess(
          `Copied ${phrase}`,
          'Paste it into one of your AI agents to run it.',
        )
      }
      className={cn('h-[34px] font-semibold text-[13.5px]', className)}
    />
  )
}
