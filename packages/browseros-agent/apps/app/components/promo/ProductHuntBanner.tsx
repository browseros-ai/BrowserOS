import { ArrowRight, X } from 'lucide-react'
import { type FC, useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  PRODUCT_HUNT_BANNER_CLICKED_EVENT,
  PRODUCT_HUNT_BANNER_DISMISSED_EVENT,
  PRODUCT_HUNT_BANNER_SHOWN_EVENT,
} from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'
import { sentry } from '@/lib/sentry/sentry'
import { productHuntBannerDismissedStorage } from './product-hunt-banner.storage'

const PRODUCT_HUNT_URL =
  'https://www.producthunt.com/products/browseros_ai?launch=browseros-neo&utm_source=browseros-newtab&utm_medium=extension&utm_campaign=ph-launch'

// Launch moment: Aug 7 2026 12:01 AM PDT. The banner is hidden until then and
// auto-hides seven days later, so it only shows during the launch window.
const LAUNCH_AT = Date.parse('2026-08-07T07:01:00Z')
const HIDE_AFTER = Date.parse('2026-08-14T07:01:00Z')

const withinLaunchWindow = (): boolean => {
  const now = Date.now()
  return now >= LAUNCH_AT && now < HIDE_AFTER
}

/** The official Product Hunt mark, unaltered; `currentColor` lets it sit in the
 *  brand orange on the card and white on the CTA button. */
const ProductHuntMark: FC<{ className?: string }> = ({ className }) => (
  <svg
    viewBox="0 0 24 24"
    fill="currentColor"
    aria-hidden="true"
    className={className}
  >
    <title>Product Hunt</title>
    <path d="M13.604 8.4h-3.405V12h3.405c.995 0 1.801-.806 1.801-1.801 0-.993-.805-1.799-1.801-1.799zM12 0C5.372 0 0 5.372 0 12s5.372 12 12 12 12-5.372 12-12S18.628 0 12 0zm1.604 14.4h-3.405V18H7.801V6h5.804c2.319 0 4.2 1.88 4.2 4.199 0 2.321-1.881 4.201-4.201 4.201z" />
  </svg>
)

export const ProductHuntBannerCard: FC<{
  onOpen: () => void
  onDismiss: () => void
}> = ({ onOpen, onDismiss }) => (
  <div className="flex items-center gap-4 rounded-xl border border-border bg-card p-4 shadow-sm transition-all hover:shadow-md">
    <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-[#da552f]/10">
      <ProductHuntMark className="h-6 w-6 text-[#da552f]" />
    </span>
    <div className="min-w-0 flex-1">
      <p className="font-semibold text-sm">
        We&apos;re live on Product Hunt 🎉
      </p>
      <p className="text-muted-foreground text-xs">
        BrowserOS neo just launched. Take a look and share your feedback.
      </p>
    </div>
    <Button
      size="sm"
      onClick={onOpen}
      aria-label="Check out our Product Hunt launch"
      className="shrink-0 gap-1.5 bg-[#da552f] text-white hover:bg-[#c74a28]"
    >
      <ProductHuntMark className="h-3.5 w-3.5" />
      Check out our launch
      <ArrowRight className="h-3 w-3" />
    </Button>
    <button
      type="button"
      onClick={onDismiss}
      className="shrink-0 rounded-sm p-1 text-muted-foreground opacity-50 transition-opacity hover:opacity-100"
      aria-label="Dismiss"
    >
      <X className="h-3.5 w-3.5" />
    </button>
  </div>
)

export const ProductHuntBanner: FC = () => {
  const [dismissed, setDismissed] = useState<boolean | null>(null)

  useEffect(() => {
    productHuntBannerDismissedStorage
      .getValue()
      .then(setDismissed)
      .catch((error) => {
        sentry.captureException(error, {
          extra: { message: 'Failed to read Product Hunt banner dismissal' },
        })
      })

    const unwatch = productHuntBannerDismissedStorage.watch(setDismissed)
    return () => unwatch()
  }, [])

  const visible = dismissed === false && withinLaunchWindow()

  // Record the impression once the banner is actually shown.
  useEffect(() => {
    if (visible) track(PRODUCT_HUNT_BANNER_SHOWN_EVENT)
  }, [visible])

  if (!visible) return null

  const handleOpen = () => {
    track(PRODUCT_HUNT_BANNER_CLICKED_EVENT)
    chrome.tabs.create({ url: PRODUCT_HUNT_URL })
  }

  const handleDismiss = async () => {
    track(PRODUCT_HUNT_BANNER_DISMISSED_EVENT)
    setDismissed(true)
    try {
      await productHuntBannerDismissedStorage.setValue(true)
    } catch (error) {
      sentry.captureException(error, {
        extra: { message: 'Failed to persist Product Hunt banner dismissal' },
      })
    }
  }

  return <ProductHuntBannerCard onOpen={handleOpen} onDismiss={handleDismiss} />
}
