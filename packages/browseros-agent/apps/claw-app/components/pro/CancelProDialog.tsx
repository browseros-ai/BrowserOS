/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Plan } from '@browseros/claw-api'
import { useQueryClient } from '@tanstack/react-query'
import { MessageCircleHeart } from 'lucide-react'
import { useRef } from 'react'
import { Button } from '@/components/motion/button/base'
import { StatefulButton } from '@/components/motion/button/stateful'
import {
  CenterMorphModal,
  CenterMorphModalClose,
  CenterMorphModalContent,
} from '@/components/motion/center-morph-modal'
import { Textarea } from '@/components/ui/textarea'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { useCancelPlan, usePlan } from '@/modules/api/plan.hooks'
import { isCardTrial } from '@/modules/plan/trial.helpers'
import { toastApiError, toastSuccess } from '@/modules/toast/toast'
import { BACKING_THANKS, formatDate } from '@/screens/pro/pro.helpers'

const FEEDBACK_ID = 'pro-cancel-feedback'
const FEEDBACK_MAX = 1000

interface CancelProDialogProps {
  plan: Plan
  open: boolean
  onOpenChange: (open: boolean) => void
}

/** Cancel is the quiet choice here; Keep Pro is the filled one. */
export function CancelProDialog({
  plan,
  open,
  onOpenChange,
}: CancelProDialogProps) {
  // Uncontrolled: the text is read once on confirm and kept if that fails.
  const feedback = useRef<HTMLTextAreaElement>(null)
  const queryClient = useQueryClient()
  // A free trial link reads as a paid plan, so only a card trial gets the trial words.
  const trial = isCardTrial(plan)
  const keepsUntil = trial
    ? (plan.trialEndsAt ?? plan.periodEnd)
    : plan.periodEnd
  const title = trial ? 'Cancel your free trial?' : 'Cancel ultrafast?'
  const confirm = trial ? 'Cancel trial' : 'Cancel ultrafast'
  const cancel = useCancelPlan({
    onSuccess: (next, { feedback: said }) => {
      queryClient.setQueryData(usePlan.getKey(), next)
      const until = next.endsAt ?? next.periodEnd
      const date = until === null ? '' : formatDate(until)
      toastSuccess(
        trial
          ? said
            ? `Thanks for telling us. Your free trial ends on ${date}`
            : `Your free trial ends on ${date}. You won’t be charged.`
          : said
            ? `Thanks for telling us. ultrafast ends on ${date}`
            : `ultrafast will end on ${date}`,
      )
      track(AnalyticsEvent.ProCancelConfirmed, { gave_feedback: Boolean(said) })
      if (trial) {
        track(AnalyticsEvent.TrialCancelConfirmed, {
          gave_feedback: Boolean(said),
        })
      }
      onOpenChange(false)
    },
    onError: (error) =>
      toastApiError('Couldn’t cancel ultrafast. Nothing changed.', error),
  })

  return (
    <CenterMorphModal open={open} onOpenChange={onOpenChange}>
      <CenterMorphModalContent
        ariaLabel={title}
        showCloseButton={false}
        className="max-w-[480px] border-border-2 bg-card"
      >
        <div className="flex flex-col gap-5 p-6">
          <div className="flex flex-col gap-4">
            <h2 className="font-bold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
              {title}
            </h2>
            <p className="text-[14.5px] text-ink-2">
              You keep ultrafast until{' '}
              <strong className="text-cyanotype-ink">
                {keepsUntil === null ? '' : formatDate(keepsUntil)}
              </strong>
              {trial
                ? ', and you won’t be charged. After that, neo goes back to Free.'
                : '. After that, neo goes back to Free and you aren’t charged again.'}
            </p>
            <p className="text-[14px] text-ink-2">{BACKING_THANKS}</p>
            <div className="flex flex-col gap-3 rounded-xl border border-border-2 bg-bg-sunken p-4">
              <p className="flex gap-2.5 text-[14px] text-ink-2">
                <MessageCircleHeart
                  aria-hidden
                  className="mt-0.5 size-4 shrink-0 text-primary"
                />
                <span>
                  We’re sorry to see you go. If you have a moment, tell us why.
                  It helps us make neo better.
                </span>
              </p>
              <div className="flex flex-col gap-1.5">
                <label
                  htmlFor={FEEDBACK_ID}
                  className="font-semibold text-[14px] text-cyanotype-ink"
                >
                  What could we do better?{' '}
                  <span className="font-normal text-ink-3">(optional)</span>
                </label>
                <Textarea
                  ref={feedback}
                  id={FEEDBACK_ID}
                  rows={3}
                  maxLength={FEEDBACK_MAX}
                  placeholder="A sentence or two is plenty"
                  disabled={cancel.isPending}
                  className="bg-card"
                />
                <span className="text-[13px] text-ink-3">
                  Only the BrowserOS team reads this.
                </span>
              </div>
            </div>
            <p className="text-[13.5px] text-ink-3">
              Changed your mind before then? Choose Keep ultrafast on this page.
            </p>
          </div>
          <div className="flex justify-end gap-2">
            <StatefulButton
              variant="ghost"
              state={cancel.isPending ? 'loading' : 'idle'}
              loadingText={confirm}
              disabled={cancel.isPending}
              onClick={() => {
                const said = feedback.current?.value.trim()
                cancel.mutate(said ? { feedback: said } : {})
              }}
            >
              {confirm}
            </StatefulButton>
            <CenterMorphModalClose>
              <Button
                disabled={cancel.isPending}
                onClick={() => {
                  if (trial) {
                    track(AnalyticsEvent.TrialKept, {
                      source: 'cancel_dialog',
                    })
                  }
                }}
                className="font-semibold"
              >
                Keep ultrafast
              </Button>
            </CenterMorphModalClose>
          </div>
        </div>
      </CenterMorphModalContent>
    </CenterMorphModal>
  )
}
