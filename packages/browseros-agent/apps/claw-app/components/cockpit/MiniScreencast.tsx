import { Globe } from 'lucide-react'
import { useEffect, useState } from 'react'
import { cn } from '@/lib/utils'
import { useSessionPreviewUrl } from '@/modules/api/audit.hooks'

const PREVIEW_REFRESH_MS = 1500

interface MiniScreencastProps {
  site: string
  sessionId: string
  live?: boolean
  /** AgentRunningCard overrides the compact default to fill its preview zone. */
  className?: string
}

interface DecodedPreviewFrame {
  sessionId: string
  src: string
}

/**
 * Renders a live session's latest JPEG from the canonical binary route,
 * with a host placeholder when there is no captured frame.
 *
 * An off-screen Image decodes each refreshed response before the visible frame
 * advances. Previous pixels remain while a newer frame for the same session
 * loads; identity changes render the placeholder immediately so one session
 * can never be shown as another.
 */
export function MiniScreencast({ sessionId, ...props }: MiniScreencastProps) {
  return (
    <SessionMiniScreencast key={sessionId} sessionId={sessionId} {...props} />
  )
}

function SessionMiniScreencast({
  site,
  sessionId,
  live,
  className,
}: MiniScreencastProps) {
  const [refresh, setRefresh] = useState(Date.now)
  const incomingSrc = useSessionPreviewUrl(sessionId, refresh)
  const [decodedFrame, setDecodedFrame] = useState<DecodedPreviewFrame | null>(
    null,
  )
  const [failedSrc, setFailedSrc] = useState<string | null>(null)
  const displayedSrc =
    decodedFrame !== null && decodedFrame.sessionId === sessionId
      ? decodedFrame.src
      : null

  useEffect(() => {
    const timer = window.setInterval(
      () => setRefresh(Date.now()),
      PREVIEW_REFRESH_MS,
    )
    return () => window.clearInterval(timer)
  }, [])

  useEffect(() => {
    if (incomingSrc === null) return
    if (failedSrc === incomingSrc) return
    if (decodedFrame?.src === incomingSrc) return
    let cancelled = false
    const image = new Image()
    image.onload = () => {
      if (cancelled) return
      setDecodedFrame({ sessionId, src: incomingSrc })
      setFailedSrc(null)
    }
    image.onerror = () => {
      if (cancelled) return
      setFailedSrc(incomingSrc)
    }
    image.src = incomingSrc
    return () => {
      cancelled = true
    }
  }, [decodedFrame?.src, failedSrc, incomingSrc, sessionId])

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
