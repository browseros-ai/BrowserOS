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

interface DecodedPreviewFrame {
  sessionId: string
  browserTabId: number
  previewCapturedAt: number
  src: string
}

/**
 * Renders a live session tab's latest JPEG from the canonical binary route,
 * with a host placeholder when there is no captured frame.
 *
 * Every capture timestamp yields a new URL. An off-screen Image decodes that
 * frame before the visible frame advances. Previous pixels remain only for a
 * newer capture of the same session tab; identity changes render the
 * placeholder immediately so one tab can never be shown as another.
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
  const [decodedFrame, setDecodedFrame] = useState<DecodedPreviewFrame | null>(
    () =>
      incomingSrc !== null &&
      browserTabId !== undefined &&
      previewCapturedAt !== undefined
        ? { sessionId, browserTabId, previewCapturedAt, src: incomingSrc }
        : null,
  )
  const [failedSrc, setFailedSrc] = useState<string | null>(null)
  const displayedSrc =
    decodedFrame !== null &&
    decodedFrame.sessionId === sessionId &&
    decodedFrame.browserTabId === browserTabId &&
    previewCapturedAt !== undefined &&
    decodedFrame.previewCapturedAt <= previewCapturedAt
      ? decodedFrame.src
      : null

  useEffect(() => {
    if (
      incomingSrc === null ||
      browserTabId === undefined ||
      previewCapturedAt === undefined
    ) {
      setDecodedFrame(null)
      setFailedSrc(null)
      return
    }
    if (failedSrc === incomingSrc) return
    if (
      decodedFrame?.sessionId === sessionId &&
      decodedFrame.browserTabId === browserTabId &&
      decodedFrame.previewCapturedAt === previewCapturedAt &&
      decodedFrame.src === incomingSrc
    ) {
      return
    }
    let cancelled = false
    const image = new Image()
    image.onload = () => {
      if (cancelled) return
      setDecodedFrame({
        sessionId,
        browserTabId,
        previewCapturedAt,
        src: incomingSrc,
      })
      setFailedSrc(null)
    }
    image.onerror = () => {
      if (cancelled) return
      setDecodedFrame(null)
      setFailedSrc(incomingSrc)
    }
    image.src = incomingSrc
    return () => {
      cancelled = true
    }
  }, [
    browserTabId,
    decodedFrame,
    failedSrc,
    incomingSrc,
    previewCapturedAt,
    sessionId,
  ])

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
          // Bad visible bytes fall back to the placeholder without retrying the same URL.
          onError={() => {
            setDecodedFrame(null)
            setFailedSrc(displayedSrc)
          }}
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
