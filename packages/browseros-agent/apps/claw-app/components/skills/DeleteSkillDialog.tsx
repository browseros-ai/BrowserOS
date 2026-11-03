/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { useQueryClient } from '@tanstack/react-query'
import { Button } from '@/components/motion/button/base'
import { StatefulButton } from '@/components/motion/button/stateful'
import {
  CenterMorphModal,
  CenterMorphModalClose,
  CenterMorphModalContent,
} from '@/components/motion/center-morph-modal'
import { useDeleteSkill, useSkills } from '@/modules/api/skills.hooks'
import { toastApiError, toastSuccess } from '@/modules/toast/toast'
import { skillCommand } from '@/screens/skills/skills.helpers'

interface DeleteSkillDialogProps {
  name: string
  description: string
  open: boolean
  onOpenChange: (open: boolean) => void
  onDeleted?: () => void
}

/** Asks once before a skill, its phrase and its runs go away. */
export function DeleteSkillDialog({
  name,
  description,
  open,
  onOpenChange,
  onDeleted,
}: DeleteSkillDialogProps) {
  const queryClient = useQueryClient()
  const remove = useDeleteSkill({
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: useSkills.getKey() })
      toastSuccess('Deleted the skill')
      onOpenChange(false)
      onDeleted?.()
    },
    onError: (error) => toastApiError('Couldn’t delete the skill', error),
  })

  return (
    <CenterMorphModal open={open} onOpenChange={onOpenChange}>
      <CenterMorphModalContent
        ariaLabel="Delete this skill?"
        showCloseButton={false}
        className="max-w-[440px] border-border-2 bg-card"
      >
        <div className="flex flex-col gap-5 p-6">
          <div className="flex flex-col gap-2">
            <h2 className="font-bold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
              Delete this skill?
            </h2>
            <p className="text-[14.5px] text-ink-2">
              “{description}” and its phrase{' '}
              <span className="font-mono text-[13.5px] text-accent-ink">
                {skillCommand(name)}
              </span>{' '}
              go away from neo and from your AI agents, along with its past
              runs.
            </p>
          </div>
          <div className="flex justify-end gap-2">
            <CenterMorphModalClose>
              <Button variant="ghost" disabled={remove.isPending}>
                Cancel
              </Button>
            </CenterMorphModalClose>
            <StatefulButton
              state={remove.isPending ? 'loading' : 'idle'}
              loadingText="Deleting"
              disabled={remove.isPending}
              onClick={() => remove.mutate({ name })}
              className="bg-red font-semibold text-white hover:bg-red/90"
            >
              Delete
            </StatefulButton>
          </div>
        </div>
      </CenterMorphModalContent>
    </CenterMorphModal>
  )
}
