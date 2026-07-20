import { Globe } from 'lucide-react'
import { useEffect, useState } from 'react'
import { cn } from '@/lib/utils'
import { useSessionBrowserTabPreviewUrl } from '@/modules/api/audit.hooks'

interface MiniScreencastProps {
  site: string
  sessionId: string
  browserTabId?: number
  live?: boolean
  /**
   * Timestamp of the tab's newest capture. Its only job here is to make
   * the preview URL unique per frame; undefined means the tab has never
   * been captured, which renders the globe placeholder.
   */
  previewCapturedAt?: number
  /** AgentRunningCard overrides the compact default to fill its preview zone. */
  className?: string
}

/**
 * Renders a live session tab's latest JPEG from the canonical binary route,
 * with a host placeholder when there is no captured frame.
 *
 * Every capture timestamp yields a new URL. An off-screen Image decodes that
 * frame before `displayedSrc` advances, preserving the previous pixels until
 * the replacement is ready and avoiding a flash between polling updates.
 */
export function MiniScreencast({
  site,
  sessionId,
  browserTabId,
  live,
  previewCapturedAt,
  className,
}: MiniScreencastProps) {
  const incomingSrc = useSessionBrowserTabPreviewUrl(
    sessionId,
    browserTabId,
    previewCapturedAt,
  )
  // The DOM paints displayedSrc until the replacement has decoded.
  const [displayedSrc, setDisplayedSrc] = useState<string | null>(incomingSrc)

  useEffect(() => {
    if (incomingSrc === null) {
      setDisplayedSrc(null)
      return
    }
    if (incomingSrc === displayedSrc) return
    // A failed fetch never advances displayedSrc, leaving the last good frame.
    let cancelled = false
    const image = new Image()
    image.onload = () => {
      if (!cancelled) setDisplayedSrc(incomingSrc)
    }
    image.src = incomingSrc
    return () => {
      cancelled = true
    }
  }, [incomingSrc, displayedSrc])

  return (
    <div
      className={cn(
        'relative flex items-center justify-center overflow-hidden bg-bg-sunken',
        className ?? 'h-[132px] w-full',
      )}
    >
      {displayedSrc ? (
        <img
          data-preview-url={displayedSrc}
          src={displayedSrc}
          alt={`Live view of ${site}`}
          className="h-full w-full object-cover"
          // Bad bytes on the initial frame fall back to the placeholder.
          onError={() => setDisplayedSrc(null)}
        />
      ) : (
        <div className="flex flex-col items-center gap-1.5 text-ink-3">
          <Globe className="size-7" />
          <code className="font-mono text-[11px] text-ink-2">{site}</code>
        </div>
      )}
      {live && (
        <span
          aria-hidden
          className={cn(
            'absolute top-2.5 right-2.5 size-2 animate-pulse-dot rounded-full bg-green',
            // The translucent ring keeps the dot readable over busy previews.
            'ring-2 ring-bg-canvas/70',
          )}
        />
      )}
    </div>
  )
}
