/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { useState } from 'react'
import { useUltrafastJoined } from '@/screens/ultrafast/ultrafast-price'
import { FeedbackInviteCard, useFeedbackOffer } from './FeedbackInviteCard'
import {
  isProductHuntPromoAvailable,
  ProductHuntBanner,
} from './ProductHuntBanner'
import {
  readUltrafastBannerDismissed,
  UltrafastBanner,
} from './UltrafastBanner'

export type PromoKind = 'feedback' | 'product-hunt' | 'ultrafast'

const PRIORITY: readonly PromoKind[] = ['feedback', 'product-hunt', 'ultrafast']
// Pause the waitlist promotion until re-enabled.
const SHOW_ULTRAFAST_BANNER = false

export interface PromoSlotState {
  /** The one promotion Home shows, or null when none is due. */
  kind: PromoKind | null
  close: (kind: PromoKind) => void
}

/** Picks at most one promotion, so Home never stacks banners. */
export function usePromoSlot(): PromoSlotState {
  const feedback = useFeedbackOffer()
  const ultrafastJoined = useUltrafastJoined()
  const [closed, setClosed] = useState<readonly PromoKind[]>([])
  const available: Record<PromoKind, boolean> = {
    feedback: feedback.bookUrl !== undefined,
    'product-hunt': isProductHuntPromoAvailable(),
    ultrafast:
      SHOW_ULTRAFAST_BANNER &&
      !ultrafastJoined &&
      !readUltrafastBannerDismissed(),
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
  if (kind === 'ultrafast') {
    return <UltrafastBanner onClosed={() => close('ultrafast')} />
  }
  return null
}
