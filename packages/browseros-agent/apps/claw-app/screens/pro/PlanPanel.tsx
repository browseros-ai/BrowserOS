/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Plan } from '@browseros/claw-api'
import { useQueryClient } from '@tanstack/react-query'
import { CalendarX, CreditCard, ExternalLink } from 'lucide-react'
import type { ReactNode } from 'react'
import { Button } from '@/components/motion/button/base'
import { StatefulButton } from '@/components/motion/button/stateful'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { usePlan, useResumePlan } from '@/modules/api/plan.hooks'
import { toastApiError, toastSuccess } from '@/modules/toast/toast'
import { formatDate, PLAN_UNCHECKED } from './pro.helpers'
import type { ProPortal } from './pro.hooks'

interface PlanPanelProps {
  plan: Plan
  portal: ProPortal
  onCancel: () => void
}

/** The plan as one line, with Billing and Cancel, for each state it can be in. */
export function PlanPanel({ plan, portal, onCancel }: PlanPanelProps) {
  const queryClient = useQueryClient()
  const resume = useResumePlan({
    onSuccess: (next) => {
      queryClient.setQueryData(usePlan.getKey(), next)
      toastSuccess(
        next.periodEnd === null
          ? 'ultrafast stays on.'
          : `ultrafast stays on. It renews on ${formatDate(next.periodEnd)}`,
      )
      track(AnalyticsEvent.ProResumed)
    },
    onError: (error) => toastApiError('Couldn’t keep ultrafast', error),
  })

  const billing = (
    <Button
      variant="secondary"
      size="sm"
      disabled={portal.isOpening}
      onClick={() => portal.open('billing')}
      className="font-semibold"
    >
      <ExternalLink aria-hidden className="size-4" />
      Billing
    </Button>
  )
  const updateCard = (
    <Button
      size="sm"
      disabled={portal.isOpening}
      onClick={() => portal.open('updateCard')}
      className="font-semibold"
    >
      Update card
    </Button>
  )
  const cancel = (
    <Button variant="ghost" size="sm" onClick={onCancel} className="text-ink-2">
      Cancel ultrafast
    </Button>
  )

  if (plan.status === 'ending') {
    return (
      <PlanRow
        stale={plan.stale}
        icon={<CalendarX />}
        lead={`Ends ${plan.endsAt === null ? 'soon' : formatDate(plan.endsAt)}.`}
        rest="No more charges."
      >
        <StatefulButton
          size="sm"
          state={resume.isPending ? 'loading' : 'idle'}
          loadingText="Keep ultrafast"
          disabled={resume.isPending}
          onClick={() => resume.mutate()}
          className="font-semibold"
        >
          Keep ultrafast
        </StatefulButton>
        {billing}
      </PlanRow>
    )
  }

  if (plan.status === 'unpaid') {
    return (
      <PlanRow
        stale={plan.stale}
        icon={<CreditCard />}
        lead="ultrafast is paused."
        rest="Update your card to turn it back on."
      >
        {updateCard}
        {billing}
        {cancel}
      </PlanRow>
    )
  }

  if (plan.status === 'paymentFailed') {
    return (
      <PlanRow
        stale={plan.stale}
        icon={<CreditCard />}
        lead="Your last payment didn’t go through."
        rest={
          plan.graceUntil === null
            ? undefined
            : `ultrafast stays on until ${formatDate(plan.graceUntil)}.`
        }
      >
        {updateCard}
        {cancel}
      </PlanRow>
    )
  }

  return (
    <PlanRow
      stale={plan.stale}
      lead="You’re on ultrafast"
      rest={
        plan.periodEnd === null
          ? undefined
          : `Renews ${formatDate(plan.periodEnd)}`
      }
    >
      {billing}
      {cancel}
    </PlanRow>
  )
}

// ---------------------------------------------------------------------------

function PlanRow({
  icon,
  lead,
  rest,
  stale,
  children,
}: {
  /** Set for a state that needs a look; shown in amber. */
  icon?: ReactNode
  lead: string
  rest?: string
  stale: boolean
  children: ReactNode
}) {
  return (
    <section
      aria-label="Your plan"
      aria-live="polite"
      className="flex flex-col gap-2"
    >
      <div className="flex flex-wrap items-center justify-between gap-x-5 gap-y-3">
        <p className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[15px] text-ink-2">
          {icon ? (
            <span aria-hidden className="text-amber-ink [&>svg]:size-4">
              {icon}
            </span>
          ) : null}
          <strong className="font-bold text-[17px] text-cyanotype-ink">
            {lead}
          </strong>
          {rest ? <span>{rest}</span> : null}
        </p>
        <span className="flex flex-wrap items-center gap-1.5 max-sm:w-full">
          {children}
        </span>
      </div>
      {stale ? (
        <p role="status" className="text-[13.5px] text-ink-3">
          {PLAN_UNCHECKED}
        </p>
      ) : null}
    </section>
  )
}
