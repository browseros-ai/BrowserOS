import { Layers } from 'lucide-react'
import { cn } from '@/lib/utils'
import type { ReplayFrame } from '@/modules/api/replay.hooks'
import { formatTime, KIND_STYLE, VERB_META } from './replay.helpers'

interface EventTimelineProps {
  frames: readonly ReplayFrame[]
  currentFrameIndex: number
  /** Seconds elapsed, used to dim frames that have not played yet. */
  currentTime: number
  onSeek: (seconds: number) => void
}

/**
 * Right-rail vertical event list. Each row corresponds to one frame
 * in the recording. Past frames are full-opacity, future frames dim,
 * and the row matching the playhead gets an accent-tint background.
 * Clicking a row seeks the player to that frame.
 */
export function EventTimeline({
  frames,
  currentFrameIndex,
  currentTime,
  onSeek,
}: EventTimelineProps) {
  return (
    <aside className="flex w-[320px] shrink-0 flex-col border-border border-l bg-card">
      <header className="flex items-center gap-1.5 border-border border-b px-4 py-3 font-bold text-ink text-sm">
        <Layers className="size-3.5 text-ink-3" />
        Action timeline
      </header>
      <div className="flex flex-1 flex-col overflow-y-auto px-3 py-2">
        {frames.map((frame, i) => {
          const seen = frame.t <= currentTime + 0.001
          const isCurrent = i === currentFrameIndex
          const verb = VERB_META[frame.verb]
          const kind = KIND_STYLE[frame.kind]
          const showConnector = i < frames.length - 1
          return (
            <button
              type="button"
              key={`frame-${frame.kind}-${frame.verb}-${frame.t}`}
              onClick={() => onSeek(frame.t)}
              className={cn(
                'flex gap-3 rounded-lg p-2.5 text-left transition-opacity',
                isCurrent && 'bg-accent-tint',
                seen ? 'opacity-100' : 'opacity-50',
              )}
            >
              <div className="flex shrink-0 flex-col items-center">
                <span
                  className={cn(
                    'flex size-6 items-center justify-center rounded-md',
                    kind.tileBgClass,
                    kind.tileFgClass,
                  )}
                >
                  <verb.Icon className="size-3" />
                </span>
                {showConnector && (
                  <span aria-hidden className="mt-1 w-px flex-1 bg-border-2" />
                )}
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="font-semibold text-ink text-xs">
                    {verb.label}
                  </span>
                  <span className="ml-auto font-mono text-[10.5px] text-ink-3">
                    {formatTime(frame.t)}
                  </span>
                </div>
                <p className="mt-0.5 text-ink-2 text-xs leading-snug">
                  {frame.caption}
                </p>
                {frame.note && (
                  <span
                    className={cn(
                      'mt-1 inline-block rounded-full px-2 py-0.5 font-bold text-[10px]',
                      kind.noteClass,
                    )}
                  >
                    {frame.note}
                  </span>
                )}
              </div>
            </button>
          )
        })}
      </div>
    </aside>
  )
}
