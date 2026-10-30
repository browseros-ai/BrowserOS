/**
 * @license
 * Copyright 2026 BrowserOS
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
import {
  useApproveSkillWrite,
  useSkill,
  useSkills,
} from '@/modules/api/skills.hooks'
import { toastApiError, toastSuccess } from '@/modules/toast/toast'

interface AllowWritesDialogProps {
  name: string
  description: string
  open: boolean
  onOpenChange: (open: boolean) => void
}

/** Asks once before a skill's ultrafast flow may add, submit or send things on its own. */
export function AllowWritesDialog({
  name,
  description,
  open,
  onOpenChange,
}: AllowWritesDialogProps) {
  const queryClient = useQueryClient()
  const approve = useApproveSkillWrite({
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: useSkill.getKey({ name }),
      })
      void queryClient.invalidateQueries({ queryKey: useSkills.getKey() })
      toastSuccess('This skill can now run ultrafast')
      onOpenChange(false)
    },
    onError: (error) => toastApiError('Couldn’t allow writes', error),
  })

  return (
    <CenterMorphModal open={open} onOpenChange={onOpenChange}>
      <CenterMorphModalContent
        ariaLabel="Let this skill run ultrafast?"
        showCloseButton={false}
        className="max-w-[460px] border-border-2 bg-card"
      >
        <div className="flex flex-col gap-5 p-6">
          <div className="flex flex-col gap-2">
            <h2 className="font-bold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
              Let this skill run ultrafast?
            </h2>
            <p className="text-[14.5px] text-ink-2">
              “{description}” changes something on the site, like adding to a
              cart or sending a form. Its fast version was checked without doing
              that. Once you allow it, your agent runs it in about a second
              without asking again.
            </p>
            <p className="text-[14px] text-ink-3">
              If the site changes, the fast version stops before repeating
              anything and your agent finishes the task the normal way.
            </p>
          </div>
          <div className="flex justify-end gap-2">
            <CenterMorphModalClose>
              <Button variant="ghost" disabled={approve.isPending}>
                Not now
              </Button>
            </CenterMorphModalClose>
            <StatefulButton
              state={approve.isPending ? 'loading' : 'idle'}
              loadingText="Allowing"
              disabled={approve.isPending}
              onClick={() => approve.mutate({ name })}
              className="font-semibold"
            >
              Allow writes
            </StatefulButton>
          </div>
        </div>
      </CenterMorphModalContent>
    </CenterMorphModal>
  )
}
