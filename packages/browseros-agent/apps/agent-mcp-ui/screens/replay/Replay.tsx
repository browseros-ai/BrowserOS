import { ArrowLeft, History } from 'lucide-react'
import { StatusBadge } from '@/components/cockpit/StatusBadge'
import { Spinner } from '@/components/ui/spinner'
import { EventTimeline } from './EventTimeline'
import { PlaybackTransport } from './PlaybackTransport'
import { ReplayViewport } from './ReplayViewport'
import { useReplayData } from './replay.data'
import { frameIndexAt } from './replay.helpers'
import { usePlayback } from './use-playback'

export function Replay() {
  const { replay, isLoading, navigate } = useReplayData()
  const playback = usePlayback(replay?.totalSeconds ?? 0)

  if (isLoading || !replay) {
    return (
      <div className="flex h-full flex-1 items-center justify-center bg-bg-canvas text-ink-3">
        <Spinner />
      </div>
    )
  }

  const back = () => navigate('/governance/audit')
  const currentFrameIndex = frameIndexAt(replay.frames, playback.time)
  const currentFrame = replay.frames[currentFrameIndex]

  const stats: { label: string; value: string }[] = [
    { label: 'Duration', value: replay.duration },
    { label: 'Tokens', value: replay.tokens },
    { label: 'Steps', value: replay.steps },
    { label: 'Approvals', value: replay.approvals },
  ]

  return (
    <div className="flex h-screen min-h-0 flex-col bg-bg-canvas">
      <header className="flex shrink-0 items-center gap-4 border-border border-b bg-card px-5 py-3">
        <button
          type="button"
          onClick={back}
          className="flex items-center gap-1.5 font-semibold text-ink-2 text-sm hover:text-ink"
        >
          <ArrowLeft className="size-4" />
          Audit trail
        </button>
        <span className="h-5 w-px bg-border-2" />
        <span className="inline-flex items-center gap-1.5 rounded-full bg-accent-tint px-2.5 py-0.5 font-bold text-[10.5px] text-accent-ink uppercase tracking-wider">
          <History className="size-3" />
          Replay
        </span>
        <div className="min-w-0 flex-1">
          <div className="truncate font-bold text-ink text-sm">
            {replay.taskTitle}
          </div>
          <div className="text-ink-3 text-xs">
            {replay.agentLabel} · {replay.harness}
            {replay.startedAt ? ` · ${replay.startedAt}` : ''}
          </div>
        </div>
        <StatusBadge status={replay.status} />
        <div className="ml-2 flex gap-5">
          {stats.map((stat) => (
            <div key={stat.label}>
              <div className="font-bold text-[10px] text-ink-4 uppercase tracking-wider">
                {stat.label}
              </div>
              <div className="font-bold font-mono text-ink text-sm">
                {stat.value}
              </div>
            </div>
          ))}
        </div>
      </header>

      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col gap-3 p-4">
          <ReplayViewport site={replay.site} frame={currentFrame} />
          <PlaybackTransport
            playback={playback}
            totalSeconds={replay.totalSeconds}
            frames={replay.frames}
          />
        </div>
        <EventTimeline
          frames={replay.frames}
          currentFrameIndex={currentFrameIndex}
          currentTime={playback.time}
          onSeek={playback.seek}
        />
      </div>
    </div>
  )
}
