import { Globe } from 'lucide-react'
import { useEffect, useState, useSyncExternalStore } from 'react'
import { cn } from '@/lib/utils'
import { sessionPreviewUrl, useApiBaseUrl } from '@/modules/api/audit.hooks'

const PREVIEW_REFRESH_MS = 3000
const PREVIEW_TIMEOUT_MS = 10_000

function subscribeToVisibility(onChange: () => void): () => void {
  document.addEventListener('visibilitychange', onChange)
  return () => document.removeEventListener('visibilitychange', onChange)
}

const isDocumentVisible = () => document.visibilityState === 'visible'

interface MiniScreencastProps {
  site: string
  sessionId: string
  /** Omitted follows the session; a supplied tab must still belong to it. */
  browserTabId?: number
  live?: boolean
  className?: string
}

/**
 * A card owns only its displayed JPEG and one pending replacement. rrweb live
 * players retained every played event and reproduced renderer OOM crashes;
 * these small previews need current pixels, not a growing replay history.
 */
export function MiniScreencast({
  sessionId,
  browserTabId,
  ...props
}: MiniScreencastProps) {
  return (
    <SessionMiniScreencast
      key={`${sessionId}:${browserTabId ?? 'follow'}`}
      sessionId={sessionId}
      browserTabId={browserTabId}
      {...props}
    />
  )
}

function SessionMiniScreencast({
  site,
  sessionId,
  browserTabId,
  live,
  className,
}: MiniScreencastProps) {
  const baseUrl = useApiBaseUrl()
  const [src, setSrc] = useState<string | null>(null)
  const visible = useSyncExternalStore(
    subscribeToVisibility,
    isDocumentVisible,
    () => false,
  )

  useEffect(() => {
    if (baseUrl === null || !visible) return
    let disposed = false
    let timer: number | undefined
    let deadline: number | undefined
    let request: AbortController | null = null
    let currentUrl: string | null = null
    let pendingUrl: string | null = null
    let image: HTMLImageElement | null = null
    let cancelDecode: (() => void) | null = null

    const clearPendingImage = (): void => {
      if (image) {
        image.onload = null
        image.onerror = null
        image.src = ''
        image = null
      }
      cancelDecode = null
      if (pendingUrl) URL.revokeObjectURL(pendingUrl)
      pendingUrl = null
    }

    const refresh = async (): Promise<void> => {
      request = new AbortController()
      const { signal } = request
      // Bound both network and decoding time. Cleanup also settles the decode
      // promise so an unmounted card cannot remain reachable through its await.
      deadline = window.setTimeout(() => {
        request?.abort()
        cancelDecode?.()
      }, PREVIEW_TIMEOUT_MS)
      try {
        const response = await fetch(
          sessionPreviewUrl(sessionId, Date.now(), baseUrl, browserTabId),
          { signal, cache: 'no-store' },
        )
        if (!response.ok) throw new Error('Preview unavailable')
        const blob = await response.blob()
        if (disposed || signal.aborted) return
        pendingUrl = URL.createObjectURL(blob)
        // Fetch once: decoding and displaying the blob URL avoids two captures
        // of the no-store JPEG endpoint for the same refresh.
        await new Promise<void>((resolve, reject) => {
          cancelDecode = () => reject(new Error('Preview decode cancelled'))
          image = new Image()
          image.onload = () => resolve()
          image.onerror = () => reject(new Error('Preview decode failed'))
          image.src = pendingUrl ?? ''
        })
        if (disposed || signal.aborted) return
        const previous = currentUrl
        currentUrl = pendingUrl
        pendingUrl = null
        setSrc(currentUrl)
        if (previous) URL.revokeObjectURL(previous)
      } catch {
        // A busy capture slot or transient failure keeps the last good frame.
      } finally {
        window.clearTimeout(deadline)
        clearPendingImage()
        // Schedule only after completion; a slow request must never create a
        // queue of screenshots, decoded images or outstanding HTTP requests.
        if (!disposed) timer = window.setTimeout(refresh, PREVIEW_REFRESH_MS)
      }
    }

    void refresh()
    return () => {
      disposed = true
      window.clearTimeout(timer)
      window.clearTimeout(deadline)
      request?.abort()
      cancelDecode?.()
      clearPendingImage()
      if (currentUrl) URL.revokeObjectURL(currentUrl)
      currentUrl = null
      setSrc(null)
    }
  }, [baseUrl, sessionId, browserTabId, visible])

  return (
    <div
      className={cn(
        'relative flex items-center justify-center overflow-hidden bg-bg-sunken',
        className ?? 'h-[132px] w-full',
      )}
    >
      {src ? (
        <img
          data-preview-url={src}
          src={src}
          alt={`Live view of ${site}`}
          className="h-full w-full object-cover"
          onError={() => setSrc(null)}
        />
      ) : (
        <div className="flex flex-col items-center gap-1.5 text-ink-3">
          <Globe className="size-7" />
          <code className="font-mono text-[11px] text-ink-2">{site}</code>
        </div>
      )}
      {live && src && (
        <span
          aria-hidden
          className={cn(
            'absolute top-2.5 right-2.5 size-2 animate-pulse-dot rounded-full bg-green',
            'ring-2 ring-bg-canvas/70',
          )}
        />
      )}
    </div>
  )
}
