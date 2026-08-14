/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Copy } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { skillCommand } from '@/screens/skills/skills.helpers'

interface RunSkillButtonProps {
  name: string
  size?: 'sm' | 'default'
}

/** Copies the `/name` command a user pastes into a coding agent to run a skill. */
export function RunSkillButton({ name, size = 'sm' }: RunSkillButtonProps) {
  const onRun = () => {
    if (typeof window === 'undefined' || !navigator?.clipboard?.writeText) {
      return
    }
    void navigator.clipboard.writeText(skillCommand(name)).then(() => {
      toast.success(`Copied ${skillCommand(name)}`, {
        description: 'Paste it into your coding agent to run this task.',
      })
    })
  }
  return (
    <Button size={size} variant="outline" className="rounded-9" onClick={onRun}>
      <Copy />
      Run
    </Button>
  )
}
