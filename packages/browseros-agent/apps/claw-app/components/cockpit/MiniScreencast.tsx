import { Globe } from 'lucide-react'
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
}

/**
 * Card-top tile on the Running-now homepage cards. Renders the live
 * screencast JPEG from the background poller when available; falls
 * back to a tinted block with the site host and a small globe when
 * the cache is cold or the page is in failure backoff.
 *
 * The `live` flag adds a pulsing dot top-right matching the design's
 * running indicator. The dot gets a translucent ring so it reads
 * against busy thumbnails.
 */
export function MiniScreencast({
  site,
  live,
  screencast,
}: MiniScreencastProps) {
  const hasFrame =
    screencast !== null &&
    screencast !== undefined &&
    screencast.jpegBase64.length > 0
  return (
    <div className="relative flex h-[132px] items-center justify-center overflow-hidden bg-bg-sunken">
      {hasFrame ? (
        <img
          src={`data:image/jpeg;base64,${screencast.jpegBase64}`}
          alt={`Live view of ${site}`}
          className="h-full w-full object-cover"
          // Decode off the main thread so a slow decode does not
          // stall the homepage's 1.5s poll cycle.
          decoding="async"
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
