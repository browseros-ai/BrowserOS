/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Plan } from '@browseros/claw-api'
import { useQueryClient } from '@tanstack/react-query'
import { CalendarX, CreditCard, ExternalLink, Gift } from 'lucide-react'
import { type ReactNode, useEffect } from 'react'
import { Button } from '@/components/motion/button/base'
import { StatefulButton } from '@/components/motion/button/stateful'
import { cn } from '@/lib/utils'
import {
  AnalyticsEvent,
  track,
  trackOncePerSession,
} from '@/modules/analytics/events'
import { usePlan, useResumePlan } from '@/modules/api/plan.hooks'
import {
  daysLabel,
  isCardTrial,
  trialDaysLeft,
} from '@/modules/plan/trial.helpers'
import { toastApiError, toastSuccess } from '@/modules/toast/toast'
import { formatDate, PLAN_UNCHECKED } from './pro.helpers'
import type { ProPortal } from './pro.hooks'

interface PlanPanelProps {
  plan: Plan
  portal: ProPortal
  onCancel: () => void
  /** "$20", what a month costs once a free trial ends, when the prices are known. */
  monthlyPrice?: string
  /** How many days a free trial lasts, for the days bar. */
  trialLength: number
}

/** The plan as one line, with Billing and Cancel, for each state it can be in. */
export function PlanPanel({
  plan,
  portal,
  onCancel,
  monthlyPrice,
  trialLength,
}: PlanPanelProps) {
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
      if (isCardTrial(next)) {
        track(AnalyticsEvent.TrialKept, { source: 'plan_row' })
      }
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

  // A trial from a free trial link reads as a paid plan, so only a card trial gets these rows.
  const trialEnd = isCardTrial(plan) ? plan.trialEndsAt : undefined
  const daysLeft = trialDaysLeft(plan, Date.now())
  const daysLeftShown =
    trialEnd !== undefined && daysLeft !== null && plan.status === 'active'

  // Reports what was on screen, which only the rendered row knows.
  useEffect(() => {
    if (daysLeftShown && daysLeft !== null) {
      trackOncePerSession(AnalyticsEvent.TrialDaysLeftViewed, {
        days_left: daysLeft,
      })
    }
  }, [daysLeftShown, daysLeft])

  if (trialEnd !== undefined && plan.status === 'ending') {
    return (
      <PlanRow
        stale={plan.stale}
        icon={<CalendarX />}
        lead={`Trial ends ${formatDate(trialEnd)}.`}
        rest="You won’t be charged."
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

  if (trialEnd !== undefined && daysLeft !== null && daysLeftShown) {
    return (
      <PlanRow
        stale={plan.stale}
        icon={<Gift />}
        calm
        lead="Free trial"
        rest={`${daysLabel(daysLeft)} left. ${
          monthlyPrice
            ? `Then ${monthlyPrice} a month from ${formatDate(trialEnd)}.`
            : `Then it renews on ${formatDate(trialEnd)}.`
        }`}
        below={
          <DaysBar left={daysLeft} total={Math.max(trialLength, daysLeft)} />
        }
      >
        {billing}
        <Button
          variant="ghost"
          size="sm"
          onClick={onCancel}
          className="text-ink-2"
        >
          Cancel trial
        </Button>
      </PlanRow>
    )
  }

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
  calm = false,
  lead,
  rest,
  below,
  stale,
  children,
}: {
  /** Set for a state that needs a look; shown in amber unless `calm`. */
  icon?: ReactNode
  calm?: boolean
  lead: string
  rest?: string
  /** Shown under the line, such as the free trial's days. */
  below?: ReactNode
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
            <span
              aria-hidden
              className={cn(
                '[&>svg]:size-4',
                calm ? 'text-primary' : 'text-amber-ink',
              )}
            >
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
      {below}
      {stale ? (
        <p role="status" className="text-[13.5px] text-ink-3">
          {PLAN_UNCHECKED}
        </p>
      ) : null}
    </section>
  )
}

/** One mark per trial day, the ones used filled in. */
function DaysBar({ left, total }: { left: number; total: number }) {
  const used = total - left
  const days = Array.from({ length: total }, (_, index) => index + 1)
  return (
    <div
      role="img"
      aria-label={`${left} of ${total} days left`}
      className="flex w-full max-w-[280px] gap-1"
    >
      {days.map((day) => (
        <span
          key={day}
          className={cn(
            'h-1.5 flex-1 rounded-full',
            day <= used ? 'bg-primary' : 'bg-border-2',
          )}
        />
      ))}
    </div>
  )
}
