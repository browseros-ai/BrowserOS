import { X } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { ProductHuntIcon } from '@/components/ui/svgs/productHuntIcon'
import { AnalyticsEvent, track } from '@/modules/analytics/events'

const PRODUCT_HUNT_URL =
  'https://www.producthunt.com/products/browseros_ai?launch=browseros-neo&utm_source=browseros-neo-newtab&utm_medium=extension&utm_campaign=ph-launch'

// Launch moment: Aug 7 2026 12:01 AM PDT. The banner is hidden until then and
// auto-hides seven days later, so it only shows during the launch window.
const LAUNCH_AT = Date.parse('2026-08-07T07:01:00Z')
const HIDE_AFTER = Date.parse('2026-08-14T07:01:00Z')
const DISMISS_KEY = 'productHuntBannerDismissed'

function withinLaunchWindow(): boolean {
  const now = Date.now()
  return now >= LAUNCH_AT && now < HIDE_AFTER
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

export function ProductHuntBannerCard({
  onOpen,
  onDismiss,
}: {
  onOpen: () => void
  onDismiss: () => void
}) {
  return (
    <div className="flex items-center gap-4 rounded-xl border border-border bg-card px-4 py-3 shadow-sm">
      <span className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-[#da552f]/10">
        <ProductHuntIcon className="size-6 text-[#da552f]" />
      </span>
      <div className="min-w-0 flex-1">
        <p className="font-semibold text-foreground text-sm">
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
        <ProductHuntIcon className="size-3.5" />
        Check out our launch
      </Button>
      <button
        type="button"
        onClick={onDismiss}
        aria-label="Dismiss"
        className="shrink-0 rounded-sm p-1 text-muted-foreground opacity-50 transition-opacity hover:opacity-100"
      >
        <X className="size-3.5" />
      </button>
    </div>
  )
}

export function ProductHuntBanner() {
  const [dismissed, setDismissed] = useState(readDismissed)
  const visible = !dismissed && withinLaunchWindow()

  // Record the impression once the banner is actually shown.
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
  }

  return (
    <div className="mx-auto max-w-7xl px-8 pt-6">
      <ProductHuntBannerCard onOpen={handleOpen} onDismiss={handleDismiss} />
    </div>
  )
}
