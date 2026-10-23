/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Cockpit entry to the Ultrafast waitlist. Impressions are not tracked: the
 * cockpit is the new tab page, and the waitlist's own view event is the
 * funnel's denominator.
 */

import { Zap } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useHref } from 'react-router'
import { ButtonLink } from '@/components/motion/button/base'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { useUltrafastJoined } from '@/screens/ultrafast/ultrafast-price'
import { PromoCard } from './PromoCard'

const DISMISS_KEY = 'ultrafastBannerDismissed:v1'

export function readUltrafastBannerDismissed(): boolean {
  try {
    return localStorage.getItem(DISMISS_KEY) === 'true'
  } catch {
    return false
  }
}

export function UltrafastBanner({ onClosed }: { onClosed?: () => void }) {
  const joined = useUltrafastJoined()
  const [dismissed, setDismissed] = useState(readUltrafastBannerDismissed)
  const waitlistHref = useHref('/ultrafast')

  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key === DISMISS_KEY && readUltrafastBannerDismissed()) {
        setDismissed(true)
        onClosed?.()
      }
    }
    window.addEventListener('storage', onStorage)
    return () => window.removeEventListener('storage', onStorage)
  }, [onClosed])

  const dismiss = () => {
    setDismissed(true)
    try {
      localStorage.setItem(DISMISS_KEY, 'true')
    } catch {
      // Still hide this tab's banner when browser storage is unavailable.
    }
    onClosed?.()
  }

  if (joined || dismissed) return null
  return (
    <PromoCard
      aria-label="Ultrafast mode"
      icon={<Zap />}
      title="Ultrafast mode is coming"
      description="Faster browser tasks for your AI agents."
      dismissLabel="Dismiss this banner"
      onDismiss={dismiss}
      action={
        <ButtonLink
          href={waitlistHref}
          size="sm"
          onClick={() => track(AnalyticsEvent.UltrafastBannerClicked)}
          className="h-[34px] px-3.5 font-semibold text-[13.5px]"
        >
          Join the waitlist
        </ButtonLink>
      }
    />
  )
}
