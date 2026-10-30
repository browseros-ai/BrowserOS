/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { PlanComputer } from '@browseros/claw-api'
import { useQueryClient } from '@tanstack/react-query'
import { Button } from '@/components/motion/button/base'
import { StatefulButton } from '@/components/motion/button/stateful'
import { MorphingModal } from '@/components/motion/morphing-modal'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import {
  usePlan,
  usePlanKey,
  useRemoveComputer,
} from '@/modules/api/plan.hooks'
import { toastApiError, toastSuccess } from '@/modules/toast/toast'
import { computerName } from '@/screens/pro/pro.helpers'

interface RemoveComputerDialogProps {
  open: boolean
  /** The computer to take off the key, kept while the dialog closes. */
  computer: PlanComputer | null
  onClose: () => void
}

/** Asks once before another computer on the key goes back to Free. */
export function RemoveComputerDialog({
  open,
  computer,
  onClose,
}: RemoveComputerDialogProps) {
  const queryClient = useQueryClient()
  const remove = useRemoveComputer({
    onSuccess: (change) => {
      queryClient.setQueryData(usePlan.getKey(), change.plan)
      if (change.key) queryClient.setQueryData(usePlanKey.getKey(), change.key)
      if (computer) toastSuccess(`Removed ${computerName(computer)}`)
      track(AnalyticsEvent.ProComputerRemoved)
      onClose()
    },
    onError: (error) => toastApiError('Couldn’t remove this computer', error),
  })
  const name = computer ? computerName(computer) : ''

  return (
    <MorphingModal
      viewId={open && computer ? 'remove' : null}
      onClose={onClose}
      placement="center"
      className="max-w-[440px] border-border-2 bg-card"
    >
      <div className="flex flex-col gap-5">
        <div className="flex flex-col gap-2">
          <h2 className="font-bold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
            Remove {name}?
          </h2>
          <p className="text-[14.5px] text-ink-2">
            Pro turns off on that computer the next time it checks in, usually
            within a few hours. You can add it back any time with your key.
          </p>
        </div>
        <div className="flex justify-end gap-2">
          <Button variant="ghost" disabled={remove.isPending} onClick={onClose}>
            Cancel
          </Button>
          <StatefulButton
            state={remove.isPending ? 'loading' : 'idle'}
            loadingText="Remove"
            disabled={remove.isPending || !computer}
            onClick={() => {
              if (computer) remove.mutate({ installId: computer.installId })
            }}
            className="font-semibold"
          >
            Remove
          </StatefulButton>
        </div>
      </div>
    </MorphingModal>
  )
}
