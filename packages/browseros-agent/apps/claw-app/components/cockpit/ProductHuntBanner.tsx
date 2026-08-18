import { X } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { ProductHuntIcon } from '@/components/ui/svgs/productHuntIcon'
import { AnalyticsEvent, track } from '@/modules/analytics/events'

const PRODUCT_HUNT_URL = 'https://bit.ly/browseros-ext'

// The banner is available immediately and auto-hides after the end of Aug 14
// 2026 (PDT), so it never lingers past the launch window.
const HIDE_AFTER = Date.parse('2026-08-15T07:00:00Z')
const DISMISS_KEY = 'productHuntBannerDismissed'

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

export function ProductHuntBannerCard({
  onOpen,
  onDismiss,
}: {
  onOpen: () => void
  onDismiss: () => void
}) {
  return (
    <div className="flex items-center gap-3 rounded-[9px] border border-[#ff6154]/25 bg-[#ff6154]/[0.06] px-4 py-2.5">
      <ProductHuntIcon className="size-7 shrink-0" />
      <p className="min-w-0 flex-1 truncate text-[13px] text-cyanotype-ink leading-5">
        <span className="font-semibold">Live on Product Hunt.</span>{' '}
        <span className="text-cyanotype-soft">
          A vote or comment helps keep BrowserOS free.
        </span>
      </p>
      <Button
        size="sm"
        onClick={onOpen}
        aria-label="Support BrowserOS on Product Hunt"
        className="h-7 shrink-0 bg-[#ff6154] px-3 text-[#18181b] text-[13px] hover:bg-[#e5563f]"
      >
        Support →
      </Button>
      <button
        type="button"
        onClick={onDismiss}
        aria-label="Dismiss"
        className="shrink-0 rounded-sm p-1 text-cyanotype-soft opacity-60 transition-opacity hover:opacity-100"
      >
        <X className="size-3.5" />
      </button>
    </div>
  )
}

export function ProductHuntBanner() {
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
  }

  return <ProductHuntBannerCard onOpen={handleOpen} onDismiss={handleDismiss} />
}
