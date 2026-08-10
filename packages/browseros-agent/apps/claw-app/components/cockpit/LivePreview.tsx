import { Globe } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { Replayer } from 'rrweb'
import { cn } from '@/lib/utils'
import { useApiBaseUrl } from '@/modules/api/audit.hooks'

// rrweb-player's CSS positions the `.replayer-wrapper` we scale below.
import 'rrweb-player/dist/style.css'

const FULL_SNAPSHOT = 2
const META = 4
const DEFAULT_SIZE = { width: 1280, height: 720 }

interface LivePreviewProps {
  site: string
  sessionId: string
  /** Preview a specific owned tab; omitted follows the session's live target. */
  browserTabId?: number
  live?: boolean
  className?: string
}

interface RrwebEvent {
  type: number
  data: unknown
  timestamp: number
}

/**
 * Streams a live session's own rrweb recording into an rrweb Replayer in live
 * mode, so the running card shows a real, moving preview without the browser
 * ever capturing a screenshot. Remounts on tab change so one tab's stream is
 * never briefly shown as another.
 */
export function LivePreview({
  sessionId,
  browserTabId,
  ...props
}: LivePreviewProps) {
  return (
    <SessionLivePreview
      key={`${sessionId}:${browserTabId ?? ''}`}
      sessionId={sessionId}
      browserTabId={browserTabId}
      {...props}
    />
  )
}

function SessionLivePreview({
  site,
  sessionId,
  browserTabId,
  live,
  className,
}: LivePreviewProps) {
  const baseUrl = useApiBaseUrl()
  const mountRef = useRef<HTMLDivElement>(null)
  const [rendering, setRendering] = useState(false)

  useEffect(() => {
    const mount = mountRef.current
    if (!mount || baseUrl === null) return

    const query =
      browserTabId === undefined ? '' : `?browserTabId=${browserTabId}`
    const url = `${baseUrl}/api/v1/sessions/${sessionId}/recording/live${query}`
    const source = new EventSource(url)

    let replayer: Replayer | null = null
    let observer: ResizeObserver | null = null
    // Events accumulated before rrweb can start: it needs a full snapshot to
    // build the DOM, so append batches buffer here until one arrives.
    let pending: RrwebEvent[] = []

    const teardown = (): void => {
      observer?.disconnect()
      observer = null
      if (replayer) {
        try {
          replayer.destroy()
        } catch {
          // already gone; we are resetting anyway
        }
        replayer = null
      }
      pending = []
      mount.replaceChildren()
      setRendering(false)
    }

    const toRrweb = (raw: string): RrwebEvent[] => {
      const parsed = JSON.parse(raw) as {
        ts: number
        type: number
        data: unknown
      }[]
      return parsed.map((event) => ({
        type: event.type,
        data: event.data,
        timestamp: event.ts,
      }))
    }

    const buildIfReady = (): void => {
      const lastSnapshot = pending.findLastIndex(
        (event) => event.type === FULL_SNAPSHOT,
      )
      if (lastSnapshot === -1) return
      const metaBefore = pending
        .slice(0, lastSnapshot + 1)
        .findLastIndex((event) => event.type === META)
      const start = metaBefore === -1 ? lastSnapshot : metaBefore
      const initial = pending.slice(start)
      pending = []
      mount.replaceChildren()
      try {
        replayer = new Replayer(initial as never, {
          root: mount,
          liveMode: true,
          skipInactive: false,
          showWarning: false,
        })
        replayer.startLive(initial[0]?.timestamp)
      } catch {
        replayer = null
        return
      }
      scaleToFit(mount, recordedSize(initial), (next) => {
        observer = next
      })
      setRendering(true)
    }

    const ingest = (events: RrwebEvent[]): void => {
      if (replayer) {
        for (const event of events) replayer.addEvent(event as never)
        return
      }
      pending.push(...events)
      buildIfReady()
    }

    source.addEventListener('switch', () => {
      // A new document (navigation or tab switch): tear down; the bootstrap
      // that follows rebuilds the player for the new DOM.
      teardown()
    })
    source.addEventListener('bootstrap', (event) => ingest(toRrweb(event.data)))
    source.addEventListener('append', (event) => ingest(toRrweb(event.data)))
    // The server fell behind and will re-send a fresh switch + bootstrap on this
    // same connection; reset so the rebuild is clean.
    source.addEventListener('reset', () => teardown())
    // No recording yet (or a pinned tab with none): hold the placeholder.
    source.addEventListener('idle', () => teardown())

    return () => {
      source.close()
      teardown()
    }
  }, [baseUrl, sessionId, browserTabId])

  return (
    <div
      data-live-preview={sessionId}
      className={cn(
        'relative flex items-center justify-center overflow-hidden bg-bg-sunken',
        className ?? 'h-[132px] w-full',
      )}
    >
      <div ref={mountRef} className="absolute inset-0 overflow-hidden" />
      {!rendering && (
        <div className="flex flex-col items-center gap-1.5 text-ink-3">
          <Globe className="size-7" />
          <code className="font-mono text-[11px] text-ink-2">{site}</code>
        </div>
      )}
      {live && rendering && (
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

function recordedSize(events: readonly RrwebEvent[]): {
  width: number
  height: number
} {
  const meta = events.find((event) => event.type === META)
  const data = meta?.data as { width?: unknown; height?: unknown } | undefined
  const width =
    typeof data?.width === 'number' && data.width > 0
      ? data.width
      : DEFAULT_SIZE.width
  const height =
    typeof data?.height === 'number' && data.height > 0
      ? data.height
      : DEFAULT_SIZE.height
  return { width, height }
}

/** Scales rrweb's fixed-size player to cover the card, tracking resizes. */
function scaleToFit(
  mount: HTMLElement,
  size: { width: number; height: number },
  registerObserver: (observer: ResizeObserver) => void,
): void {
  const wrapper = mount.querySelector<HTMLElement>('.replayer-wrapper')
  if (!wrapper) return
  wrapper.style.position = 'absolute'
  wrapper.style.transformOrigin = 'top left'
  const apply = (): void => {
    const rect = mount.getBoundingClientRect()
    if (rect.width === 0 || rect.height === 0) return
    const scale = Math.max(rect.width / size.width, rect.height / size.height)
    wrapper.style.transform = `scale(${scale})`
    wrapper.style.left = `${(rect.width - size.width * scale) / 2}px`
    wrapper.style.top = `${(rect.height - size.height * scale) / 2}px`
  }
  apply()
  const observer = new ResizeObserver(apply)
  observer.observe(mount)
  registerObserver(observer)
}
