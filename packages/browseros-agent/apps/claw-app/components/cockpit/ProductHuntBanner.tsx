import { ArrowRight } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Button } from '@/components/motion/button/base'
import { ProductHuntIcon } from '@/components/ui/svgs/productHuntIcon'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { PromoCard } from './PromoCard'

const PRODUCT_HUNT_URL = 'https://bit.ly/browseros-ext'

// The banner is available immediately and auto-hides after the end of Aug 14
// 2026 (PDT), so it never lingers past the launch window.
const HIDE_AFTER = Date.parse('2026-08-15T07:00:00Z')
const DISMISS_KEY = 'productHuntBannerDismissed'

/** True while the launch window is open and the reader has not dismissed it. */
export function isProductHuntPromoAvailable(): boolean {
  return !readDismissed() && withinLaunchWindow()
}

function withinLaunchWindow(): boolean {
  return Date.now() < HIDE_AFTER
}

function readDismissed(): boolean {
  try {
    return localStorage.getItem(DISMISS_KEY) === 'true'
  } catch {
    return false
  }
}

function persistDismissed(): void {
  try {
    localStorage.setItem(DISMISS_KEY, 'true')
  } catch {
    // A new tab without storage access simply forgets the dismissal.
  }
}

export function ProductHuntBanner({ onClosed }: { onClosed?: () => void }) {
  const [dismissed, setDismissed] = useState(readDismissed)
  const visible = !dismissed && withinLaunchWindow()

  useEffect(() => {
    if (visible) track(AnalyticsEvent.ProductHuntBannerShown)
  }, [visible])

  if (!visible) return null

  const handleOpen = () => {
    track(AnalyticsEvent.ProductHuntBannerClicked)
    chrome.tabs.create({ url: PRODUCT_HUNT_URL })
  }

  const handleDismiss = () => {
    track(AnalyticsEvent.ProductHuntBannerDismissed)
    setDismissed(true)
    persistDismissed()
    onClosed?.()
  }

  return (
    <PromoCard
      aria-label="Product Hunt launch"
      icon={<ProductHuntIcon />}
      title="We’re live on Product Hunt"
      description="A vote or comment helps keep BrowserOS free."
      onDismiss={handleDismiss}
      action={
        <Button
          size="sm"
          variant="ghost"
          onClick={handleOpen}
          aria-label="Support BrowserOS on Product Hunt"
          className="h-8 px-2.5 font-semibold text-[13.5px] text-accent-ink hover:bg-card/70 hover:text-accent-ink"
        >
          Support us
          <ArrowRight aria-hidden className="size-[15px]" />
        </Button>
      }
    />
  )
}
