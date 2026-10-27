import { VideoOff } from 'lucide-react'
import type { Ref } from 'react'
import { Card } from '@/components/motion/card'
import type { DescribedStep } from '@/lib/step-sentences'
import {
  taskScreenshotUrl,
  useTaskScreenshotBaseUrl,
} from '@/modules/api/audit.hooks'
import { ReplayPanel } from '@/screens/replay/ReplayPanel'
import type { ReplaySession } from '@/screens/replay/use-replay-session'

interface TaskPlayerProps {
  ref?: Ref<HTMLDivElement>
  sessionId: string
  replay: ReplaySession
  /** The step being shown when there is no recording to play. */
  step?: DescribedStep
  /** Its picture, when there is no recording to play here. */
  snapshotId: number | null
  /** Opens the full-screen player. */
  onFullScreen: () => void
  /** The full-screen player is open; this one holds the step's picture. */
  fullScreen: boolean
}

/**
 * The recording when there is one, else the picture for the chosen step.
 * While a recording downloads or plays full screen, its picture sits inside
 * the player's frame with the controls showing, so nothing moves around it.
 */
export function TaskPlayer({
  ref,
  sessionId,
  replay,
  step,
  snapshotId,
  onFullScreen,
  fullScreen,
}: TaskPlayerProps) {
  const baseUrl = useTaskScreenshotBaseUrl()
  const hasRecording = replay.plan.durationSeconds > 0
  const pictureSrc =
    snapshotId !== null && baseUrl !== null
      ? taskScreenshotUrl(sessionId, snapshotId, baseUrl)
      : undefined

  return (
    <Card
      ref={ref}
      id="player"
      aria-label="Playback"
      size="compact"
      className="h-[min(64vh,540px)] min-h-[340px] scroll-mt-6 gap-0 overflow-hidden border-border-2 bg-card p-3 shadow-card"
    >
      {hasRecording ? (
        <ReplayPanel
          session={replay}
          parked={fullScreen}
          posterSrc={pictureSrc}
          onToggleFullScreen={onFullScreen}
        />
      ) : replay.recordingState === 'loading' ? (
        <ReplayPanel
          session={replay}
          loading
          posterSrc={pictureSrc}
          onToggleFullScreen={onFullScreen}
        />
      ) : pictureSrc ? (
        <div className="relative min-h-0 flex-1 overflow-hidden rounded-[10px] bg-bg-sunken shadow-shot">
          <img
            src={pictureSrc}
            alt={step ? `What the page looked like: ${step.sentence}` : ''}
            className="h-full w-full object-cover object-top-left"
          />
          {step ? (
            <span className="absolute bottom-3.5 left-1/2 max-w-[calc(100%-32px)] -translate-x-1/2 truncate rounded-full bg-ink-deep/90 px-3.5 py-2 font-semibold text-[13.5px] text-white">
              {step.sentence}
            </span>
          ) : null}
        </div>
      ) : (
        <div className="grid min-h-0 flex-1 place-items-center rounded-[10px] bg-bg-sunken p-6 text-center">
          <div className="flex max-w-[40ch] flex-col items-center gap-2.5">
            <span className="grid size-11 place-items-center rounded-full bg-accent-tint text-primary">
              <VideoOff aria-hidden className="size-[18px]" />
            </span>
            <p className="font-bold text-cyanotype-ink">
              No recording for this task
            </p>
            <p className="text-[14px] text-ink-2">
              The steps on the right show what happened.
            </p>
          </div>
        </div>
      )}
    </Card>
  )
}
