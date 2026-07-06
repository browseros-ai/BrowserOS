import { Globe } from 'lucide-react'
import { useEffect, useState } from 'react'
import { cn } from '@/lib/utils'
import type { ScreencastFrame } from '@/modules/api/tabs.hooks'

interface MiniScreencastProps {
  site: string
  live?: boolean
  /**
   * Latest poller frame for this page. When present the component
   * renders the JPEG as the card top; when null/undefined the
   * placeholder globe + host tile is shown. The container has a
   * fixed height either way so the card never shifts as frames
   * appear or disappear.
   */
  screencast?: ScreencastFrame | null
  /**
   * Overrides the container sizing. Defaults to `h-[132px] w-full`
   * for compact cards. AgentRunningCard passes
   * `h-full w-full` so the frame fills its `flex-1` zone instead of
   * clamping at 132px, which used to leave a Sky Tint strip below
   * the image inside the 300px running-now tile.
   */
  className?: string
}

/**
 * Card-top tile on the Running-now homepage cards. Renders the live
 * screencast JPEG from the background poller when available; falls
 * back to a tinted block with the site host and a small globe when
 * the cache is cold or the page is in failure backoff.
 *
 * Flicker-free frame swap: every time `screencast.capturedAt` ticks
 * we kick off an off-screen `new Image()` to pre-decode the next
 * frame, and only swap the visible `<img src>` once the decode has
 * completed. Without this the browser unloads the old pixels the
 * moment the src attribute changes, briefly exposing the container
 * backdrop between paints; the operator sees that as a flicker
 * every 1.5s. The pre-decode trades one extra render per frame for
 * a perfectly stable visible image.
 *
 * The `live` flag adds a pulsing dot top-right matching the design's
 * running indicator. The dot gets a translucent ring so it reads
 * against busy thumbnails.
 */
export function MiniScreencast({
  site,
  live,
  screencast,
  className,
}: MiniScreencastProps) {
  const incomingSrc =
    screencast && screencast.jpegBase64.length > 0
      ? `data:image/jpeg;base64,${screencast.jpegBase64}`
      : null

  // `displayedSrc` is the src actually painted in the DOM. It only
  // moves forward once the new bytes have decoded successfully.
  const [displayedSrc, setDisplayedSrc] = useState<string | null>(incomingSrc)

  useEffect(() => {
    if (incomingSrc === null) {
      setDisplayedSrc(null)
      return
    }
    if (incomingSrc === displayedSrc) return
    // Pre-decode in an off-screen Image. The browser caches the
    // decoded pixels keyed by the data URL, so when we then set
    // them on the visible <img> the swap is instant (no blank gap).
    let cancelled = false
    const img = new Image()
    img.onload = () => {
      if (!cancelled) setDisplayedSrc(incomingSrc)
    }
    img.onerror = () => {
      // Decode failed (truncated bytes, unexpected encoding). Skip
      // this frame; the next poll will retry with fresh bytes.
    }
    img.src = incomingSrc
    return () => {
      cancelled = true
    }
  }, [incomingSrc, displayedSrc])

  const showImage = displayedSrc !== null

  return (
    <div
      className={cn(
        'relative flex items-center justify-center overflow-hidden bg-bg-sunken',
        className ?? 'h-[132px] w-full',
      )}
    >
      {showImage ? (
        <img
          src={displayedSrc}
          alt={`Live view of ${site}`}
          className="h-full w-full object-cover"
          // Catches corruption that slipped past the off-screen
          // pre-decode (most relevant on initial mount, where
          // displayedSrc is seeded directly from incomingSrc
          // without going through the Image() decode gate).
          // Falling back to null here re-renders into the globe
          // placeholder so the operator never sees a browser
          // broken-image icon.
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
            // Translucent ring so the dot stays readable against busy
            // live thumbnails.
            'ring-2 ring-bg-canvas/70',
          )}
        />
      )}
    </div>
  )
}
