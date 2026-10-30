/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type {
  PlanCheckoutStatus,
  PlanPortalFlow,
  PriceInterval,
} from '@browseros/claw-api'
import { skipToken, useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef, useState } from 'react'
import type { ProWelcome } from '@/components/pro/WelcomeProDialog'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import {
  useOpenPortal,
  usePlan,
  usePlanCheckout,
  usePlanPricing,
  useRefreshPlan,
  useStartCheckout,
} from '@/modules/api/plan.hooks'
import { toast, toastApiError } from '@/modules/toast/toast'
import { apiErrorCode, type CheckoutPhase, checkoutPhase } from './pro.helpers'

const CHECKOUT_POLL_MS = 2000
const PORTAL_REFRESH_MS = 30_000
const REFUSALS = new Set(['already_subscribed', 'unknown_plan'])

function openInNewTab(url: string): void {
  void chrome.tabs.create({ url })
}

export interface StartProCheckout {
  plan: string
  interval: PriceInterval
  /** This computer is on a key whose Pro has ended, so paying keeps that key. */
  sameKey: boolean
}

export interface ProCheckout {
  phase: Exclude<CheckoutPhase, 'complete'>
  start: (checkout: StartProCheckout) => void
  isStarting: boolean
  /** Opens the payment page again, for a tab that was closed. */
  reopen: () => void
  /** The person says they won't pay this time. */
  cancel: () => void
  welcome: ProWelcome | null
  closeWelcome: () => void
}

/**
 * Buying Pro: open Stripe's page in a new tab, then poll the checkout every
 * 2 s until it completes or expires. After 30 minutes neo stops waiting.
 */
export function useProCheckout(): ProCheckout {
  const queryClient = useQueryClient()
  const [cancelled, setCancelled] = useState(false)
  const [sameKey, setSameKey] = useState(false)
  const [welcomeClosed, setWelcomeClosed] = useState(false)
  const settled = useRef<string | null>(null)

  const start = useStartCheckout({
    onSuccess: (checkout, { interval }) => {
      openInNewTab(checkout.url)
      track(AnalyticsEvent.ProCheckoutStarted, { interval })
    },
    onError: (error) => {
      const code = apiErrorCode(error)
      if (code && REFUSALS.has(code)) {
        track(AnalyticsEvent.ProCheckoutRefused, { reason: code })
      }
      if (code === 'unknown_plan') {
        void queryClient.invalidateQueries({
          queryKey: usePlanPricing.getKey(),
        })
        toast({
          title: 'That price changed. Check the new one and try again.',
          status: 'error',
        })
        return
      }
      toastApiError('Couldn’t open the payment page', error)
    },
  })

  const checkoutId = start.data?.checkoutId
  const progressOf = (
    lastCheckedAt: number,
    status: PlanCheckoutStatus | undefined,
  ) =>
    checkoutPhase({
      started: checkoutId !== undefined,
      cancelled,
      status,
      startedAt: start.submittedAt,
      lastCheckedAt,
    })
  const poll = usePlanCheckout({
    variables: checkoutId && !cancelled ? { checkoutId } : skipToken,
    refetchInterval: (query) => {
      const phase = progressOf(
        Math.max(query.state.dataUpdatedAt, query.state.errorUpdatedAt),
        query.state.data,
      )
      return phase === 'waiting' || phase === 'confirming'
        ? CHECKOUT_POLL_MS
        : false
    },
  })
  const phase = progressOf(
    Math.max(poll.dataUpdatedAt, poll.errorUpdatedAt),
    poll.data,
  )
  const paidPlan = phase === 'complete' ? poll.data?.plan : null

  // A query has no success callback, so the poll's outcome is acted on here,
  // once per checkout: the paid plan goes into the cache for every screen.
  useEffect(() => {
    if (!checkoutId || settled.current === checkoutId) return
    if (paidPlan) {
      settled.current = checkoutId
      queryClient.setQueryData(usePlan.getKey(), paidPlan)
      track(AnalyticsEvent.ProCheckoutCompleted)
    } else if (phase === 'abandoned' && !cancelled) {
      settled.current = checkoutId
      track(AnalyticsEvent.ProCheckoutAbandoned)
    }
  }, [checkoutId, paidPlan, phase, cancelled, queryClient])

  return {
    phase: phase === 'complete' ? 'idle' : phase,
    start: (checkout) => {
      setCancelled(false)
      setWelcomeClosed(false)
      setSameKey(checkout.sameKey)
      start.mutate({ plan: checkout.plan, interval: checkout.interval })
    },
    isStarting: start.isPending,
    reopen: () => {
      if (start.data) openInNewTab(start.data.url)
    },
    cancel: () => {
      setCancelled(true)
      track(AnalyticsEvent.ProCheckoutAbandoned)
    },
    welcome: paidPlan && !welcomeClosed ? { plan: paidPlan, sameKey } : null,
    closeWelcome: () => setWelcomeClosed(true),
  }
}

export interface ProPortal {
  open: (flow: PlanPortalFlow) => void
  isOpening: boolean
}

/**
 * Billing and receipts, and Update card, live on Stripe's portal in another
 * tab. Coming back to neo, and 30 s later, the plan is fetched fresh.
 */
export function useProPortal(): ProPortal {
  const queryClient = useQueryClient()
  const refresh = useRefreshPlan({
    onSuccess: (plan) => queryClient.setQueryData(usePlan.getKey(), plan),
  })
  const portal = useOpenPortal({
    onSuccess: ({ url }) => {
      openInNewTab(url)
      track(AnalyticsEvent.ProBillingOpened)
      window.addEventListener('focus', () => refresh.mutate(), { once: true })
      setTimeout(() => refresh.mutate(), PORTAL_REFRESH_MS)
    },
    onError: (error) => toastApiError('Couldn’t open the billing page', error),
  })
  return {
    open: (flow) => portal.mutate({ flow }),
    isOpening: portal.isPending,
  }
}
