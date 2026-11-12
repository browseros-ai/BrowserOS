import { Globe } from 'lucide-react'
import { cn } from '@/lib/utils'
import { useLiveFrame } from '@/modules/api/live-frames'

interface MiniScreencastProps {
  site: string
  sessionId: string
  /** Omitted follows the session; a supplied tab must still belong to it. */
  browserTabId?: number
  live?: boolean
  className?: string
}

/**
 * A card's live view of the tab its session is working in, streamed by the
 * server's screencast. It holds only the newest frame: rrweb live players that
 * kept every event reproduced renderer OOM crashes, and a card needs current
 * pixels, not history.
 */
export function MiniScreencast({
  site,
  sessionId,
  browserTabId,
  live,
  className,
}: MiniScreencastProps) {
  const src = useLiveFrame(sessionId, browserTabId)

  return (
    <div
      data-live-session={sessionId}
      className={cn(
        'relative flex items-center justify-center overflow-hidden bg-bg-sunken',
        className ?? 'h-[132px] w-full',
      )}
    >
      {src ? (
        <img
          src={src}
          alt={`Live view of ${site}`}
          className="h-full w-full object-cover"
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
