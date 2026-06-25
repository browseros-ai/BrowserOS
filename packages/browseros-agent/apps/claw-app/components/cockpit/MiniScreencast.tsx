import { Globe } from 'lucide-react'
import { cn } from '@/lib/utils'

interface MiniScreencastProps {
  site: string
  live?: boolean
}

/**
 * Placeholder card-top "screencast" tile. The vision plan calls for a
 * live thumbnail of the actual page the agent is on; until the
 * recording pipeline lands, we show a tinted block with the site
 * host and a small globe so the card has weight and the site
 * identity reads at a glance. The `live` flag adds a pulsing dot
 * top-right matching the design's running indicator.
 */
export function MiniScreencast({ site, live }: MiniScreencastProps) {
  return (
    <div className="relative flex h-[132px] items-center justify-center bg-bg-sunken">
      <div className="flex flex-col items-center gap-1.5 text-ink-3">
        <Globe className="size-7" />
        <code className="font-mono text-[11px] text-ink-2">{site}</code>
      </div>
      {live && (
        <span
          aria-hidden
          className={cn(
            'absolute top-2.5 right-2.5 size-2 animate-pulse-dot rounded-full bg-green',
          )}
        />
      )}
    </div>
  )
}
