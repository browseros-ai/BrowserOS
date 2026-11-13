/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { useState } from 'react'
import { usePlan } from '@/modules/api/plan.hooks'
import { FeedbackInviteCard, useFeedbackOffer } from './FeedbackInviteCard'
import { ProBanner, readProBannerDismissed } from './ProBanner'
import {
  isProductHuntPromoAvailable,
  ProductHuntBanner,
} from './ProductHuntBanner'

export type PromoKind = 'feedback' | 'product-hunt' | 'pro'

const PRIORITY: readonly PromoKind[] = ['feedback', 'product-hunt', 'pro']

export interface PromoSlotState {
  /** The one promotion Home shows, or null when none is due. */
  kind: PromoKind | null
  close: (kind: PromoKind) => void
}

/** Picks at most one promotion, so Home never stacks banners. */
export function usePromoSlot(): PromoSlotState {
  const feedback = useFeedbackOffer()
  // Loading or unreachable reads as no billing, so the Pro bar waits.
  const plan = usePlan()
  const [closed, setClosed] = useState<readonly PromoKind[]>([])
  const available: Record<PromoKind, boolean> = {
    feedback: feedback.bookUrl !== undefined,
    'product-hunt': isProductHuntPromoAvailable(),
    pro:
      plan.data?.available === true &&
      plan.data.status === 'none' &&
      !readProBannerDismissed(),
  }
  const kind =
    PRIORITY.find(
      (candidate) => available[candidate] && !closed.includes(candidate),
    ) ?? null
  return {
    kind,
    close: (done) => setClosed((previous) => [...previous, done]),
  }
}

export function PromoSlot({ kind, close }: PromoSlotState) {
  if (kind === 'feedback') {
    return <FeedbackInviteCard onClosed={() => close('feedback')} />
  }
  if (kind === 'product-hunt') {
    return <ProductHuntBanner onClosed={() => close('product-hunt')} />
  }
  if (kind === 'pro') {
    return <ProBanner onClosed={() => close('pro')} />
  }
  return null
}
