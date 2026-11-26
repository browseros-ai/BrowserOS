/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Plan, SessionSummary } from '@browseros/claw-api'
import { ArrowRight, CirclePause, Lock } from 'lucide-react'
import { NavLink } from 'react-router'
import {
  Alert,
  AlertAction,
  AlertContent,
  AlertDescription,
} from '@/components/motion/alert'
import { usePlan } from '@/modules/api/plan.hooks'

const LINK_CLASS =
  'inline-flex h-[34px] items-center gap-1.5 rounded-md px-3.5 font-semibold text-[13.5px] outline-none focus-visible:ring-2 focus-visible:ring-ring'

/**
 * What the task page says when choosing a browser profile, which comes with ultrafast, shaped
 * the task: it stopped because ultrafast is off, or an agent on Free asked for a profile and the
 * task stayed in Default. Tasks that never touched a profile show nothing.
 */
export function ProfileNote({ session }: { session: SessionSummary }) {
  if (session.stopReason === 'profileNeedsUltrafast') {
    return <StoppedNote profile={session.browserProfile?.name} />
  }
  if (session.profileLocked) return <LockedNote />
  return null
}

function LockedNote() {
  return (
    <Alert
      variant="info"
      className="rounded-2xl px-5 py-4"
      contentClassName="flex-wrap items-center gap-3.5 md:flex-nowrap"
    >
      <Lock aria-hidden className="size-5 shrink-0" />
      <AlertContent className="min-w-0 space-y-0">
        <AlertDescription className="text-[15px] text-ink">
          Stayed in your Default profile. Choosing a profile comes with
          ultrafast.
        </AlertDescription>
      </AlertContent>
      <AlertAction className="shrink-0 pt-0">
        <NavLink
          to="/pro"
          className={`${LINK_CLASS} border border-border-2 bg-card text-cyanotype-ink hover:bg-muted`}
        >
          See ultrafast
          <ArrowRight aria-hidden className="size-4" />
        </NavLink>
      </AlertAction>
    </Alert>
  )
}

/** A paused plan needs a working card; any other plan without ultrafast needs ultrafast. */
function isPaused(plan: Plan | undefined): boolean {
  return plan?.status === 'unpaid' || plan?.status === 'paymentFailed'
}

function StoppedNote({ profile }: { profile: string | undefined }) {
  const plan = usePlan()
  const paused = isPaused(plan.data)
  const where = profile ? `your ${profile} profile` : 'a browser profile'
  return (
    <div className="flex flex-col gap-2">
      <Alert
        variant="warning"
        className="rounded-2xl px-5 py-4"
        contentClassName="flex-wrap items-center gap-3.5 md:flex-nowrap"
      >
        <CirclePause aria-hidden className="size-5 shrink-0" />
        <AlertContent className="min-w-0 space-y-0">
          <AlertDescription className="text-[15px] text-ink">
            <strong>This task stopped.</strong> It was set to work in {where},
            and choosing a profile comes with ultrafast.{' '}
            {paused ? 'ultrafast is paused' : 'ultrafast is off'}, so neo
            stopped instead of switching to another account.
          </AlertDescription>
        </AlertContent>
        {plan.isPending ? null : (
          <AlertAction className="shrink-0 pt-0">
            <NavLink
              to="/pro"
              className={`${LINK_CLASS} bg-primary text-primary-foreground hover:bg-primary/90`}
            >
              {paused ? 'Update card' : 'Get ultrafast'}
            </NavLink>
          </AlertAction>
        )}
      </Alert>
      <p className="px-1 text-[14px] text-ink-3">
        Or ask your agent to run it again in your Default profile.
      </p>
    </div>
  )
}
