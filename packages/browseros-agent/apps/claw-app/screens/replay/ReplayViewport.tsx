import { Globe, Lock } from 'lucide-react'
import { cn } from '@/lib/utils'
import type { ReplayFrame } from '@/modules/api/replay.hooks'
import { KIND_STYLE, VERB_META } from './replay.helpers'

interface ReplayViewportProps {
  site: string
  /** The frame whose caption is currently displayed. */
  frame: ReplayFrame | undefined
}

/**
 * Reconstructed browser viewport for the replay player. Fake site
 * chrome at the top, tinted page region in the middle, and a caption
 * overlay pill near the bottom that mirrors the prototype's
 * RR-Web-style "what is happening right now" hint.
 */
export function ReplayViewport({ site, frame }: ReplayViewportProps) {
  return (
    <div className="relative flex flex-1 flex-col overflow-hidden rounded-2xl border border-border-2 bg-card shadow-sm">
      <div className="flex h-9 shrink-0 items-center gap-2 border-border border-b bg-bg-sunken px-3">
        <span className="flex gap-1.5">
          <span className="size-2.5 rounded-full bg-[#FF5F57]" />
          <span className="size-2.5 rounded-full bg-[#FEBC2E]" />
          <span className="size-2.5 rounded-full bg-[#28C840]" />
        </span>
        <div className="ml-3 flex h-6 flex-1 items-center gap-2 rounded-md border border-border-2 bg-card px-3 font-mono text-ink-2 text-xs">
          <Lock className="size-3 text-ink-3" />
          <span className="truncate">{site}</span>
        </div>
        <span className="rounded-full bg-bg-sunken px-2 py-0.5 font-bold text-[10px] text-ink-3 uppercase tracking-wide">
          recorded
        </span>
      </div>
      <div className="relative flex flex-1 items-center justify-center overflow-hidden bg-[radial-gradient(circle_at_top,#fff,var(--color-bg-sunken))]">
        <div className="flex flex-col items-center gap-3 text-ink-3">
          <Globe className="size-12" />
          <code className="font-mono text-sm">{site}</code>
          <p className="max-w-xs text-center text-ink-4 text-xs">
            Site reconstruction lands once the RR-Web recording pipeline is
            wired.
          </p>
        </div>
        {frame && <Caption frame={frame} />}
      </div>
    </div>
  )
}

function Caption({ frame }: { frame: ReplayFrame }) {
  const verb = VERB_META[frame.verb]
  const kind = KIND_STYLE[frame.kind]
  return (
    <div className="absolute bottom-5 left-1/2 z-10 flex max-w-[82%] -translate-x-1/2 items-center gap-2.5 rounded-full bg-[#1B1A17]/90 px-4 py-2 shadow-xl backdrop-blur">
      <span
        className={cn(
          'flex size-5 items-center justify-center rounded-md text-white',
          kind.dotClass,
        )}
      >
        <verb.Icon className="size-3" />
      </span>
      <span className="truncate font-semibold text-[#EDEAE2] text-xs">
        {frame.caption}
      </span>
    </div>
  )
}
