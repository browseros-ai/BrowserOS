import { Pause, Play, RotateCcw } from 'lucide-react'
import { ActionSwapRollIcon } from '@/components/motion/action-swap-roll'
import { Button } from '@/components/motion/button/base'
import { RangeSlider } from '@/components/motion/range-slider'
import { Tabs, TabsList, TabsTrigger } from '@/components/motion/tabs'
import { cn } from '@/lib/utils'
import { formatTime, KIND_STYLE, PLAYBACK_SPEEDS } from './replay.helpers'
import type { ReplayAction } from './session-replay'
import type { Playback } from './use-playback'

const SCRUBBER_STEP = 0.1

interface PlaybackTransportProps {
  playback: Playback
  /** Replayable activity for the whole session, not one tab's recording. */
  totalSeconds: number
  actions: readonly ReplayAction[]
  onSeek: (seconds: number) => void
  /** Shown but inert, while the recording is still loading. */
  disabled?: boolean
}

/**
 * Play / pause + scrubber + speed picker for global activity time.
 * Non-action tools render as coloured bookmarks on the scrubber so the user
 * can jump straight to a block or done moment.
 */
export function PlaybackTransport({
  playback,
  totalSeconds,
  actions,
  onSeek,
  disabled = false,
}: PlaybackTransportProps) {
  const { time, isPlaying, speed, setSpeed, togglePlay } = playback
  const finished = time >= totalSeconds

  const mode = finished ? 'restart' : isPlaying ? 'pause' : 'play'

  return (
    <div className="flex items-center gap-3 px-1 pt-1">
      <Button
        size="icon"
        onClick={togglePlay}
        disabled={disabled}
        aria-label={
          finished ? 'Restart playback' : isPlaying ? 'Pause' : 'Play'
        }
        className="size-10 shrink-0 rounded-full"
      >
        <ActionSwapRollIcon value={mode}>
          {mode === 'restart' ? (
            <RotateCcw className="size-4" />
          ) : mode === 'pause' ? (
            <Pause className="size-4" />
          ) : (
            <Play className="size-4" />
          )}
        </ActionSwapRollIcon>
      </Button>
      <div className="relative min-w-0 flex-1">
        <RangeSlider
          aria-label="Playback position"
          formatValueText={(value) =>
            `${formatTime(value)} of ${formatTime(totalSeconds)}`
          }
          min={0}
          max={Math.max(totalSeconds, SCRUBBER_STEP)}
          step={SCRUBBER_STEP}
          value={time}
          onValueChange={onSeek}
          disabled={disabled}
          showTicks={false}
          className="h-9 bg-accent-tint [&>div:first-child>div]:bg-primary/25"
        />
        {/* Same 10px inset as the slider's handle, so marks line up with time. */}
        <div className="pointer-events-none absolute inset-x-[10px] inset-y-0">
          {actions.map((action) => {
            const { frame } = action
            if (frame.kind === 'action') return null
            const pct =
              totalSeconds === 0 ? 0 : (action.startAt / totalSeconds) * 100
            return (
              <button
                type="button"
                key={frame.dispatchId ?? `bookmark-${action.sourceIndex}`}
                title={frame.caption}
                aria-label={`Jump to ${frame.caption}`}
                onClick={() => onSeek(action.startAt)}
                style={{ left: `${pct}%` }}
                className={cn(
                  'pointer-events-auto absolute top-1/2 z-10 size-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-card shadow-sm',
                  KIND_STYLE[frame.kind].dotClass,
                )}
              />
            )
          })}
        </div>
      </div>
      <span className="shrink-0 text-[13.5px] text-ink-3 tabular-nums">
        {formatTime(time)} / {formatTime(totalSeconds)}
      </span>
      <Tabs
        variant="segment"
        value={String(speed)}
        onValueChange={(value) => {
          const next = Number(value)
          if (Number.isFinite(next)) setSpeed(next)
        }}
      >
        <TabsList
          aria-label="Playback speed"
          className="border border-border-2"
        >
          {PLAYBACK_SPEEDS.map((s) => (
            <TabsTrigger
              key={s}
              value={String(s)}
              disabled={disabled}
              className="px-2.5 text-[13px]"
            >
              {s}×
            </TabsTrigger>
          ))}
        </TabsList>
      </Tabs>
    </div>
  )
}
