/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Plan, PlanKey } from '@browseros/claw-api'
import { useQueryClient } from '@tanstack/react-query'
import { BadgeCheck, CalendarX, CreditCard, ExternalLink } from 'lucide-react'
import type { ReactNode } from 'react'
import { Button, type ButtonVariant } from '@/components/motion/button/base'
import { StatefulButton } from '@/components/motion/button/stateful'
import { cn } from '@/lib/utils'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { usePlan, useResumePlan } from '@/modules/api/plan.hooks'
import { toastApiError, toastSuccess } from '@/modules/toast/toast'
import { formatDate, PLAN_UNCHECKED } from './pro.helpers'
import type { ProPortal } from './pro.hooks'

interface PlanPanelProps {
  plan: Plan
  planKey?: PlanKey
  portal: ProPortal
  onCancel: () => void
}

/** What a computer with Pro sees, for each state the plan can be in. */
export function PlanPanel({ plan, planKey, portal, onCancel }: PlanPanelProps) {
  const queryClient = useQueryClient()
  const resume = useResumePlan({
    onSuccess: (next) => {
      queryClient.setQueryData(usePlan.getKey(), next)
      toastSuccess(
        next.periodEnd === null
          ? 'Pro stays on.'
          : `Pro stays on. It renews on ${formatDate(next.periodEnd)}`,
      )
      track(AnalyticsEvent.ProResumed)
    },
    onError: (error) => toastApiError('Couldn’t keep Pro', error),
  })

  const billing = (variant: ButtonVariant) => (
    <Button
      variant={variant}
      disabled={portal.isOpening}
      onClick={() => portal.open('billing')}
      className="w-full font-semibold"
    >
      <ExternalLink aria-hidden className="size-4" />
      Billing and receipts
    </Button>
  )
  const updateCard = (
    <Button
      disabled={portal.isOpening}
      onClick={() => portal.open('updateCard')}
      className="w-full font-semibold"
    >
      Update card
    </Button>
  )
  const quietCancel = (
    <Button
      variant="ghost"
      size="sm"
      onClick={onCancel}
      className="self-center text-ink-3"
    >
      Cancel Pro
    </Button>
  )
  const stale = plan.stale ? (
    <p role="status" className="text-[13.5px] text-ink-3">
      {PLAN_UNCHECKED}
    </p>
  ) : null

  if (plan.status === 'ending') {
    const ends = plan.endsAt === null ? '' : formatDate(plan.endsAt)
    return (
      <>
        <StatusHead
          icon={<CalendarX />}
          warn
          title={`Pro ends on ${ends}`}
          sub="You won’t be charged again."
        />
        <p className="text-[14.5px] text-ink-2">
          Changed your mind? Keep Pro and it renews on {ends} as usual.
        </p>
        <StatefulButton
          state={resume.isPending ? 'loading' : 'idle'}
          loadingText="Keep Pro"
          disabled={resume.isPending}
          onClick={() => resume.mutate()}
          className="w-full font-semibold"
        >
          Keep Pro
        </StatefulButton>
        {billing('ghost')}
        {stale}
      </>
    )
  }

  if (plan.status === 'unpaid') {
    return (
      <>
        <StatusHead
          icon={<CreditCard />}
          warn
          title="Pro is paused"
          sub="Your last few payments didn’t go through."
        />
        <p className="text-[14.5px] text-ink-2">
          Pro features are off for now. Update your card and Pro turns back on
          once the payment goes through. Nothing you’ve saved is lost.
        </p>
        {updateCard}
        {billing('ghost')}
        {quietCancel}
        {stale}
      </>
    )
  }

  if (plan.status === 'paymentFailed') {
    return (
      <>
        <StatusHead
          icon={<CreditCard />}
          warn
          title="Your card needs a look"
          sub="Last month’s payment didn’t go through."
        />
        <p className="text-[14.5px] text-ink-2">
          Stripe will try again over the next few days. Pro stays on until{' '}
          {plan.graceUntil === null ? '' : formatDate(plan.graceUntil)}.
        </p>
        {updateCard}
        {quietCancel}
        {stale}
      </>
    )
  }

  return (
    <>
      <StatusHead icon={<BadgeCheck />} title="You’re on Pro" />
      <dl className="flex flex-col gap-2.5 rounded-xl border border-border-2 bg-card px-4 py-3.5 text-[14.5px]">
        {plan.periodEnd !== null ? (
          <div className="flex justify-between gap-3">
            <dt className="text-ink-2">Next payment</dt>
            <dd className="font-semibold text-cyanotype-ink">
              {formatDate(plan.periodEnd)}
            </dd>
          </div>
        ) : null}
        {planKey ? (
          <div className="flex justify-between gap-3">
            <dt className="text-ink-2">Computers</dt>
            <dd className="font-semibold text-cyanotype-ink tabular-nums">
              {planKey.computers.length} of {planKey.cap}
            </dd>
          </div>
        ) : null}
      </dl>
      {billing('secondary')}
      {quietCancel}
      {stale}
    </>
  )
}

// ---------------------------------------------------------------------------

function StatusHead({
  icon,
  title,
  sub,
  warn = false,
}: {
  icon: ReactNode
  title: string
  sub?: string
  warn?: boolean
}) {
  return (
    <div className="flex items-center gap-3">
      <span
        aria-hidden
        className={cn(
          'grid size-11 shrink-0 place-items-center rounded-full [&>svg]:size-5',
          warn
            ? 'bg-amber-tint text-amber-ink'
            : 'bg-green-tint text-green-ink',
        )}
      >
        {icon}
      </span>
      <div>
        <div className="font-bold text-[19px] text-cyanotype-ink tracking-[-0.015em]">
          {title}
        </div>
        {sub ? <p className="mt-1 text-[14.5px] text-ink-2">{sub}</p> : null}
      </div>
    </div>
  )
}
