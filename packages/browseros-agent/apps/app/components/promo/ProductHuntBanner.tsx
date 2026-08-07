import { ArrowRight, X } from 'lucide-react'
import { type FC, type ReactNode, useEffect, useState } from 'react'
import { useNavigate } from 'react-router'
import ProductHuntLogo from '@/assets/producthunt.svg'
import { Button } from '@/components/ui/button'
import {
  PRODUCT_HUNT_BANNER_CLICKED_EVENT,
  PRODUCT_HUNT_BANNER_DISMISSED_EVENT,
  PRODUCT_HUNT_BANNER_SHOWN_EVENT,
} from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'
import { sentry } from '@/lib/sentry/sentry'
import { productHuntBannerDismissedStorage } from './product-hunt-banner.storage'

/**
 * @public
 */
export const PRODUCT_HUNT_URL = 'https://bit.ly/browseros-ph'

// The banner is available immediately and auto-hides after the end of Aug 14
// 2026 (PDT), so it never lingers past the launch window.
const HIDE_AFTER = Date.parse('2026-08-15T07:00:00Z')

const withinLaunchWindow = (): boolean => Date.now() < HIDE_AFTER

export const ProductHuntBannerCard: FC<{
  onOpen: () => void
  onDismiss: () => void
}> = ({ onOpen, onDismiss }) => (
  <div className="flex items-center gap-4 rounded-xl border border-border bg-card p-4 shadow-sm transition-all hover:shadow-md">
    <img
      src={ProductHuntLogo}
      alt="Product Hunt"
      className="h-9 w-9 shrink-0"
    />
    <div className="min-w-0 flex-1">
      <p className="font-semibold text-sm">
        Upvote us on Product Hunt and get 100 credits 🎉
      </p>
      <p className="text-muted-foreground text-xs">
        Upvote and comment, then DM us on Discord with a screenshot.
      </p>
    </div>
    <Button
      size="sm"
      onClick={onOpen}
      aria-label="Upvote BrowserOS on Product Hunt"
      className="shrink-0 gap-1.5 bg-[#ff6154] text-white hover:bg-[#e5563f]"
    >
      Upvote us
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

export const ProductHuntBanner: FC<{ fallback?: ReactNode }> = ({
  fallback = null,
}) => {
  const [dismissed, setDismissed] = useState<boolean | null>(null)
  const navigate = useNavigate()

  useEffect(() => {
    productHuntBannerDismissedStorage
      .getValue()
      .then(setDismissed)
      .catch((error) => {
        sentry.captureException(error, {
          extra: { message: 'Failed to read Product Hunt banner dismissal' },
        })
        setDismissed(true)
      })

    const unwatch = productHuntBannerDismissedStorage.watch(setDismissed)
    return () => unwatch()
  }, [])

  const visible = dismissed === false && withinLaunchWindow()

  useEffect(() => {
    if (visible) track(PRODUCT_HUNT_BANNER_SHOWN_EVENT)
  }, [visible])

  if (dismissed === null) return null
  if (!visible) return fallback

  const handleOpen = () => {
    track(PRODUCT_HUNT_BANNER_CLICKED_EVENT)
    chrome.tabs.create({ url: PRODUCT_HUNT_URL })
    // Product Hunt takes the foreground; this tab moves to the credits page so
    // the Discord handle form is waiting when they come back.
    navigate('/settings/usage')
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
