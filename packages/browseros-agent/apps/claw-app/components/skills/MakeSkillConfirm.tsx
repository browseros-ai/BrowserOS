/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { SessionSummary } from '@browseros/claw-api'
import { BookmarkPlus, Globe } from 'lucide-react'
import { Favicon } from '@/components/favicon/Favicon'
import { agentIdentity } from '@/components/harness/harness.types'
import { Button } from '@/components/motion/button/base'
import {
  CenterMorphModal,
  CenterMorphModalClose,
  CenterMorphModalContent,
} from '@/components/motion/center-morph-modal'
import { taskDayAndTime } from '@/screens/audit/audit.helpers'

interface MakeSkillConfirmProps {
  task: SessionSummary
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Starts the draft; the caller closes the dialog. */
  onConfirm: () => void
}

/** Asks once before neo writes a skill from a task, since that starts a paid draft. */
export function MakeSkillConfirm({
  task,
  open,
  onOpenChange,
  onConfirm,
}: MakeSkillConfirmProps) {
  return (
    <CenterMorphModal open={open} onOpenChange={onOpenChange}>
      <CenterMorphModalContent
        ariaLabel="Make this task a skill?"
        showCloseButton={false}
        className="max-w-[460px] border-border-2 bg-card"
      >
        <div className="flex flex-col gap-4 p-6">
          <div className="flex flex-col gap-2">
            <h2 className="font-extrabold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
              Make this task a skill?
            </h2>
            <p className="text-[15px] text-ink-2">
              neo reads what your agent did here and writes a skill from it. You
              check it before you save. It takes about 10 seconds.
            </p>
          </div>
          <TaskCard task={task} />
          <div className="mt-2 flex flex-wrap justify-end gap-2">
            <CenterMorphModalClose>
              <Button variant="secondary" className="h-[38px] font-semibold">
                Not now
              </Button>
            </CenterMorphModalClose>
            <Button onClick={onConfirm} className="h-[38px] px-4 font-semibold">
              <BookmarkPlus aria-hidden className="size-4" />
              Make the skill
            </Button>
          </div>
        </div>
      </CenterMorphModalContent>
    </CenterMorphModal>
  )
}

/** The task the skill comes from: its site, name, when and which agent. */
function TaskCard({ task }: { task: SessionSummary }) {
  const agent = agentIdentity(task.harness, task.label || task.slug)
  return (
    <div className="flex items-center gap-2.5 rounded-xl border border-border-2 bg-card-tint px-3 py-2.5">
      <span
        aria-hidden
        className="grid size-7 shrink-0 place-items-center rounded-md bg-accent-tint text-ink-3"
      >
        {task.site ? (
          <Favicon site={task.site} className="size-4 rounded-[3px]" />
        ) : (
          <Globe className="size-4" />
        )}
      </span>
      <span className="flex min-w-0 flex-col">
        <span className="truncate font-semibold text-[14px] text-cyanotype-ink">
          {task.name}
        </span>
        <span className="truncate text-[13px] text-ink-3">
          {taskDayAndTime(task.startedAt, Date.now())} · {agent.name}
        </span>
      </span>
    </div>
  )
}
