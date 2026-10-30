/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { useQueryClient } from '@tanstack/react-query'
import { KeyRound } from 'lucide-react'
import { Button } from '@/components/motion/button/base'
import { StatefulButton } from '@/components/motion/button/stateful'
import {
  CenterMorphModal,
  CenterMorphModalClose,
  CenterMorphModalContent,
} from '@/components/motion/center-morph-modal'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { usePlan, usePlanKey, useResetKey } from '@/modules/api/plan.hooks'
import { toastApiError } from '@/modules/toast/toast'

interface ResetKeyDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

/** Replaces the key; the new one only ever arrives by email. Mount it afresh for each opening. */
export function ResetKeyDialog({ open, onOpenChange }: ResetKeyDialogProps) {
  const queryClient = useQueryClient()
  const reset = useResetKey({
    onSuccess: (change) => {
      queryClient.setQueryData(usePlan.getKey(), change.plan)
      if (change.key) queryClient.setQueryData(usePlanKey.getKey(), change.key)
      track(AnalyticsEvent.ProKeyReset)
    },
    onError: (error) => toastApiError('Couldn’t reset your key', error),
  })

  return (
    <CenterMorphModal open={open} onOpenChange={onOpenChange}>
      <CenterMorphModalContent
        ariaLabel={
          reset.isSuccess
            ? 'We’ve emailed your new key'
            : 'Reset your licence key?'
        }
        showCloseButton={false}
        className="max-w-[440px] border-border-2 bg-card"
      >
        {reset.isSuccess ? (
          <div className="flex flex-col items-center gap-5 p-6 text-center">
            <span
              aria-hidden
              className="grid size-14 place-items-center rounded-full bg-green-tint text-green-ink"
            >
              <KeyRound className="size-6" />
            </span>
            <div className="flex flex-col gap-2">
              <h2 className="font-bold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
                We’ve emailed your new key
              </h2>
              <p className="text-[14.5px] text-ink-2">
                Your old key no longer works. Paste the new one on your other
                computers to turn Pro back on there.
              </p>
            </div>
            <CenterMorphModalClose>
              <Button className="font-semibold">Done</Button>
            </CenterMorphModalClose>
          </div>
        ) : (
          <div className="flex flex-col gap-5 p-6">
            <div className="flex flex-col gap-2">
              <h2 className="font-bold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
                Reset your licence key?
              </h2>
              <p className="text-[14.5px] text-ink-2">
                You get a new key, and the old one stops working. Pro stays on
                here. Every other computer goes back to Free until you paste the
                new key on it. We’ll email you the new key.
              </p>
              <p className="text-[13.5px] text-ink-3">
                Do this if your key was shared or a computer was lost. Your
                subscription and payments don’t change.
              </p>
            </div>
            <div className="flex justify-end gap-2">
              <CenterMorphModalClose>
                <Button variant="ghost" disabled={reset.isPending}>
                  Cancel
                </Button>
              </CenterMorphModalClose>
              <StatefulButton
                state={reset.isPending ? 'loading' : 'idle'}
                loadingText="Reset key"
                disabled={reset.isPending}
                onClick={() => reset.mutate()}
                className="bg-red font-semibold text-white hover:bg-red/90"
              >
                Reset key
              </StatefulButton>
            </div>
          </div>
        )}
      </CenterMorphModalContent>
    </CenterMorphModal>
  )
}
