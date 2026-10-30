/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { skipToken } from '@tanstack/react-query'
import { usePlan, usePlanKey, usePlanPricing } from '@/modules/api/plan.hooks'
import { isPro, proPrices } from './pro.helpers'
import { useProCheckout, useProPortal } from './pro.hooks'

/** Everything the neo Pro page reads: the plan, prices, key, checkout and portal. */
export function useProData() {
  const plan = usePlan()
  const available = plan.data?.available === true
  const pricing = usePlanPricing({
    variables: available ? undefined : skipToken,
  })
  const onKey = plan.data?.onKey === true && isPro(plan.data)
  const planKey = usePlanKey({ variables: onKey ? undefined : skipToken })
  const checkout = useProCheckout()
  const portal = useProPortal()

  return {
    plan,
    pricing,
    prices: pricing.data ? proPrices(pricing.data) : null,
    planKey,
    checkout,
    portal,
  }
}
