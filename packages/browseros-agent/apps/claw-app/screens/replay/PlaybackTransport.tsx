import { Pause, Play, RotateCcw } from 'lucide-react'
import type { ChangeEvent } from 'react'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { cn } from '@/lib/utils'
import type { ReplayFrame } from '@/modules/api/replay.hooks'
import { formatTime, KIND_STYLE, PLAYBACK_SPEEDS } from './replay.helpers'
import type { Playback } from './use-playback'

const SCRUBBER_STEP = 0.1

interface PlaybackTransportProps {
  playback: Playback
  totalSeconds: number
  frames: readonly ReplayFrame[]
}

/**
 * Play / pause + scrubber + speed picker. Non-action frames render as
 * coloured bookmarks on the scrubber so the user can jump straight to
 * an approval, block, or done moment.
 */
export function PlaybackTransport({
  playback,
  totalSeconds,
  frames,
}: PlaybackTransportProps) {
  const { time, isPlaying, speed, setSpeed, togglePlay, seek } = playback
  const finished = time >= totalSeconds
  const progress = totalSeconds === 0 ? 0 : (time / totalSeconds) * 100

  const onScrubberChange = (event: ChangeEvent<HTMLInputElement>) => {
    seek(Number(event.target.value))
  }

  return (
    <div className="rounded-2xl border border-border-2 bg-card p-3 shadow-sm">
      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={togglePlay}
          aria-label={
            finished ? 'Restart playback' : isPlaying ? 'Pause' : 'Play'
          }
          className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-accent text-white shadow"
        >
          {finished ? (
            <RotateCcw className="size-4" />
          ) : isPlaying ? (
            <Pause className="size-4" />
          ) : (
            <Play className="size-4" />
          )}
        </button>
        <span className="w-20 shrink-0 font-mono text-ink-2 text-xs">
          {formatTime(time)} / {formatTime(totalSeconds)}
        </span>
        <div className="relative flex h-7 flex-1 items-center">
          <div className="absolute right-0 left-0 h-1.5 rounded-full bg-bg-sunken" />
          <div
            className="absolute left-0 h-1.5 rounded-full bg-accent transition-[width] duration-100"
            style={{ width: `${progress}%` }}
          />
          {frames.map((frame) => {
            if (frame.kind === 'action') return null
            const pct = totalSeconds === 0 ? 0 : (frame.t / totalSeconds) * 100
            const kind = KIND_STYLE[frame.kind]
            return (
              <button
                type="button"
                key={`bookmark-${frame.kind}-${frame.t}`}
                title={frame.caption}
                aria-label={`Jump to ${frame.caption}`}
                onClick={() => seek(frame.t)}
                style={{ left: `${pct}%` }}
                className={cn(
                  'absolute z-10 size-2.5 -translate-x-1/2 rounded-full border-2 border-card shadow-sm',
                  kind.dotClass,
                )}
              />
            )
          })}
          <span
            aria-hidden
            style={{ left: `${progress}%` }}
            className="absolute z-10 size-3.5 -translate-x-1/2 rounded-full border-[2.5px] border-accent bg-card shadow"
          />
          <input
            type="range"
            aria-label="Playback position"
            aria-valuetext={`${formatTime(time)} of ${formatTime(totalSeconds)}`}
            min={0}
            max={totalSeconds}
            step={SCRUBBER_STEP}
            value={time}
            onChange={onScrubberChange}
            className="absolute inset-0 z-20 h-full w-full cursor-pointer appearance-none bg-transparent opacity-0 focus-visible:opacity-100"
          />
        </div>
        <ToggleGroup
          value={[String(speed)]}
          onValueChange={(values) => {
            const next = Number(values[0])
            if (Number.isFinite(next)) setSpeed(next)
          }}
          spacing={0}
          variant="outline"
          className="bg-bg-sunken p-0.5"
        >
          {PLAYBACK_SPEEDS.map((s) => (
            <ToggleGroupItem
              key={s}
              value={String(s)}
              className="h-7 rounded-md border-none bg-transparent px-2.5 font-bold text-ink-3 text-xs shadow-none hover:bg-transparent hover:text-ink aria-pressed:bg-card aria-pressed:text-ink aria-pressed:shadow-sm"
            >
              {s}×
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </div>
    </div>
  )
}
