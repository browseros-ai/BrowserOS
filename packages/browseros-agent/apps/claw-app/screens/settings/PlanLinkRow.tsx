/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Plan } from '@browseros/claw-api'
import { ArrowRight } from 'lucide-react'
import { useHref } from 'react-router'
import {
  AnimatedBadge,
  type AnimatedBadgeStatus,
} from '@/components/motion/animated-badge'
import { ButtonLink } from '@/components/motion/button/base'
import { usePlan } from '@/modules/api/plan.hooks'
import { formatDate, keyHasEnded } from '@/screens/pro/pro.helpers'
import { SettingRow, SettingsSection } from './settings.components'

const PLAN_ROW_ID = 'your-plan'

/** Settings' one row about Pro; everything else about it is on the ultrafast page. */
export function PlanLinkRow() {
  const plan = usePlan()
  const proHref = useHref('/pro')
  // Loading or unreachable reads as no billing, so the row stays away.
  if (plan.isPending || plan.isError || !plan.data.available) return null
  const current = plan.data
  const pill = planPill(current)

  return (
    <SettingsSection title="Your plan">
      <SettingRow
        id={PLAN_ROW_ID}
        title={
          <span className="flex flex-wrap items-center gap-2">
            {current.status === 'none' ? 'Free' : 'ultrafast'}
            {pill ? (
              <AnimatedBadge status={pill.status} size="sm" showIcon={false}>
                {pill.label}
              </AnimatedBadge>
            ) : null}
          </span>
        }
        description={planLine(current)}
        control={
          <ButtonLink
            href={proHref}
            variant="secondary"
            className="font-semibold"
          >
            Open ultrafast
            <ArrowRight aria-hidden className="size-4" />
          </ButtonLink>
        }
      />
    </SettingsSection>
  )
}

function dateOr(epochMs: number | null): string {
  return epochMs === null ? '' : formatDate(epochMs)
}

function planPill(
  plan: Plan,
): { label: string; status: AnimatedBadgeStatus } | null {
  switch (plan.status) {
    case 'active':
      return { label: 'Active', status: 'success' }
    case 'ending':
      return { label: `Ends ${dateOr(plan.endsAt)}`, status: 'warning' }
    case 'paymentFailed':
      return { label: 'Payment failed', status: 'warning' }
    case 'unpaid':
      return { label: 'Paused', status: 'danger' }
    default:
      return null
  }
}

function planLine(plan: Plan): string {
  switch (plan.status) {
    case 'active':
      return `Next payment on ${dateOr(plan.periodEnd)}. Billing, receipts and cancelling are on the ultrafast page.`
    case 'ending':
      return `ultrafast stays on until ${dateOr(plan.endsAt)}. You can keep it from the ultrafast page.`
    case 'paymentFailed':
      return 'Your last payment didn’t go through. Update your card on the ultrafast page.'
    case 'unpaid':
      return 'ultrafast is paused because your payments didn’t go through. Update your card on the ultrafast page.'
    default:
      return keyHasEnded(plan)
        ? 'Your licence key’s ultrafast plan has ended. Get it again on the ultrafast page.'
        : 'See what ultrafast adds, or use your licence key if you already paid.'
  }
}
