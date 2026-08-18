import { ArrowUpRight } from 'lucide-react'
import { NavLink, useLocation } from 'react-router'
import { cn } from '@/lib/utils'
import {
  type TaskSummary,
  taskScreenshotUrl,
  useTaskScreenshotBaseUrl,
} from '@/modules/api/audit.hooks'
import { formatDuration, formatRelative } from '@/screens/audit/audit.helpers'
import {
  type ActivityCardCaptionTone,
  activityCardCaptionTones,
} from './activityCardTone'

interface SupportingTileProps {
  task: TaskSummary
  now: number
  className?: string
  captionTone?: ActivityCardCaptionTone
}

/**
 * Cyanotype supporting tile. Mirrors the lead's captured-media well
 * and tone-switchable caption at a compact scale.
 */
export function SupportingTile({
  task,
  now,
  className,
  captionTone = 'light',
}: SupportingTileProps) {
  const isLive = task.status === 'live'
  const isStopped = task.status === 'cancelled'
  const screenshotId = task.latestScreenshotId ?? null
  const screenshotBaseUrl = useTaskScreenshotBaseUrl()
  const location = useLocation()
  return (
    <NavLink
      to={`/audit/${encodeURIComponent(task.sessionId)}`}
      state={{ from: location.pathname }}
      data-testid={`support-tile-${task.sessionId}`}
      className={cn(
        'group relative flex flex-col overflow-hidden rounded-[9px] border border-cyanotype-border bg-card transition-[border-color,box-shadow] duration-150 hover:border-cyanotype-blue hover:shadow-sm',
        className,
      )}
    >
      <div className="relative aspect-[16/10] overflow-hidden border-cyanotype-border border-b bg-cyanotype-well">
        {screenshotId !== null && screenshotBaseUrl !== null ? (
          <img
            src={taskScreenshotUrl(
              task.sessionId,
              screenshotId,
              screenshotBaseUrl,
            )}
            alt={`Session preview from ${task.label}`}
            loading="lazy"
            decoding="async"
            className="absolute inset-0 h-full w-full object-cover object-top"
          />
        ) : screenshotId !== null ? (
          <div className="absolute inset-0 animate-pulse bg-cyanotype-hover" />
        ) : (
          <NoShotComposition task={task} />
        )}
        <span className="pointer-events-none absolute top-2.5 right-2.5 flex size-6 items-center justify-center rounded-full bg-white/85 text-cyanotype-ink opacity-0 shadow-sm backdrop-blur-md transition-[opacity,transform] duration-200 group-hover:-translate-y-0.5 group-hover:opacity-100">
          <ArrowUpRight className="size-3.5" />
        </span>
      </div>
      <Caption
        task={task}
        now={now}
        isLive={isLive}
        isStopped={isStopped}
        tone={captionTone}
      />
    </NavLink>
  )
}

function Caption({
  task,
  now,
  isLive,
  isStopped,
  tone,
}: {
  task: TaskSummary
  now: number
  isLive: boolean
  isStopped: boolean
  tone: ActivityCardCaptionTone
}) {
  const toneClasses = activityCardCaptionTones[tone]
  return (
    <div
      className={cn(
        'flex flex-col gap-1.5 px-4 pt-3 pb-3.5',
        toneClasses.surface,
      )}
      data-caption-tone={tone}
    >
      <div className="flex items-center gap-2 font-medium text-[11.5px] leading-[14px]">
        <span className="truncate">{task.label}</span>
        {isLive && (
          <span className="rounded-full bg-cyanotype-live px-2 py-0.5 font-semibold text-[10px] text-cyanotype-live-ink">
            LIVE
          </span>
        )}
        {isStopped && <span className={toneClasses.subdued}>STOPPED</span>}
      </div>
      <h3 className="truncate font-bold text-[14px] leading-5 tracking-[-0.02em]">
        {task.name}
      </h3>
      <p className="text-[11.5px] tabular-nums leading-[14px]">
        {formatDuration(task.durationMs)} · {task.dispatchCount}t ·{' '}
        {formatRelative(task.startedAt, now)}
      </p>
    </div>
  )
}

function NoShotComposition({ task }: { task: TaskSummary }) {
  const verbs = task.toolSequence.slice(0, 4)
  return (
    <div className="absolute inset-0 bg-cyanotype-well">
      <div className="pointer-events-none absolute inset-0 flex flex-col justify-center gap-0.5 pl-4 text-[14px] text-cyanotype-ink/18 leading-tight tracking-tight">
        {verbs.map((verb, idx) => (
          <span
            // biome-ignore lint/suspicious/noArrayIndexKey: tool sequence is stable-ordered per session, not a reorderable list
            key={`${verb}-${idx}`}
            style={{ marginLeft: `${idx * 6}px` }}
            className="truncate"
          >
            {verb}
          </span>
        ))}
      </div>
    </div>
  )
}
