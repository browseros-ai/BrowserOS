/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { useQueryClient } from '@tanstack/react-query'
import type { ReactElement } from 'react'
import { toast } from 'sonner'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { useDeleteSkill, useSkills } from '@/modules/api/skills.hooks'

interface DeleteSkillDialogProps {
  name: string
  onDeleted: () => void
  trigger?: ReactElement
}

export function DeleteSkillDialog({
  name,
  onDeleted,
  trigger,
}: DeleteSkillDialogProps) {
  const queryClient = useQueryClient()
  const remove = useDeleteSkill()

  const onConfirm = () => {
    void toast.promise(
      remove.mutateAsync({ name }).then(() => {
        void queryClient.invalidateQueries({ queryKey: useSkills.getKey() })
        onDeleted()
      }),
      {
        loading: `Deleting ${name}…`,
        success: `Deleted ${name}`,
        error: 'Could not delete the task',
      },
    )
  }

  return (
    <AlertDialog>
      <AlertDialogTrigger
        render={
          trigger ?? (
            <Button variant="outline" size="sm" className="rounded-9 text-red">
              Delete
            </Button>
          )
        }
      />
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete this task?</AlertDialogTitle>
          <AlertDialogDescription>
            This removes {name} and unlinks its skill from your coding agents.
            Its run history is discarded.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction onClick={onConfirm}>Delete task</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
