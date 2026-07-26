import { ArrowRight, X } from 'lucide-react'
import { type FC, useEffect, useState } from 'react'
import BrowserClawLogo from '@/assets/browserclaw_logo.png'
import { Button } from '@/components/ui/button'
import {
  BROWSERCLAW_PROMO_BANNER_CLICKED_EVENT,
  BROWSERCLAW_PROMO_BANNER_DISMISSED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'
import { sentry } from '@/lib/sentry/sentry'
import { browserClawPromoDismissedStorage } from './browserclaw-promo.storage'

const BROWSERCLAW_PROMO_URL = 'https://browseros.com/agents/'

export const BrowserClawPromoBannerCard: FC<{
  onOpen: () => void
  onDismiss: () => void
}> = ({ onOpen, onDismiss }) => (
  <div className="flex items-center gap-4 rounded-xl border border-border bg-card p-4 shadow-sm transition-all hover:shadow-md">
    <img
      src={BrowserClawLogo}
      alt="BrowserClaw"
      className="h-10 w-10 shrink-0 rounded-lg"
    />
    <div className="min-w-0 flex-1">
      <p className="flex items-center gap-2 font-semibold text-sm">
        Meet BrowserClaw — the browser for AI agents
      </p>
      <p className="text-muted-foreground text-xs">
        A new product from the BrowserOS team — free, open source, and ready to
        download
      </p>
    </div>
    <Button
      variant="outline"
      size="sm"
      onClick={onOpen}
      className="shrink-0 border-[var(--accent-orange)] bg-[var(--accent-orange)]/10 text-[var(--accent-orange)] hover:bg-[var(--accent-orange)]/20 hover:text-[var(--accent-orange)]"
    >
      Check it out
      <ArrowRight className="ml-1 h-3 w-3" />
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

export const BrowserClawPromoBanner: FC = () => {
  const [dismissed, setDismissed] = useState<boolean | null>(null)

  useEffect(() => {
    browserClawPromoDismissedStorage
      .getValue()
      .then(setDismissed)
      .catch((error) => {
        sentry.captureException(error, {
          extra: { message: 'Failed to read BrowserClaw promo dismissal' },
        })
      })

    const unwatch = browserClawPromoDismissedStorage.watch((newDismissed) => {
      setDismissed(newDismissed)
    })

    return () => unwatch()
  }, [])

  if (dismissed !== false) return null

  const handleOpen = () => {
    track(BROWSERCLAW_PROMO_BANNER_CLICKED_EVENT)
    chrome.tabs.create({ url: BROWSERCLAW_PROMO_URL })
  }

  const handleDismiss = async () => {
    track(BROWSERCLAW_PROMO_BANNER_DISMISSED_EVENT)
    setDismissed(true)
    try {
      await browserClawPromoDismissedStorage.setValue(true)
    } catch (error) {
      sentry.captureException(error, {
        extra: { message: 'Failed to persist BrowserClaw promo dismissal' },
      })
    }
  }

  return (
    <BrowserClawPromoBannerCard onOpen={handleOpen} onDismiss={handleDismiss} />
  )
}
