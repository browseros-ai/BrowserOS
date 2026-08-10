import { ArrowUpRight } from 'lucide-react'
import { NavLink, useLocation } from 'react-router'
import { cn } from '@/lib/utils'
import {
  type TaskSummary,
  taskScreenshotUrl,
  useTaskScreenshotBaseUrl,
} from '@/modules/api/audit.hooks'
import {
  abbreviateSequence,
  formatDuration,
  formatRelative,
} from '@/screens/audit/audit.helpers'
import {
  type ActivityCardCaptionTone,
  activityCardCaptionTones,
} from './activityCardTone'

interface LeadRunTileProps {
  task: TaskSummary
  now: number
  className?: string
  captionTone?: ActivityCardCaptionTone
}

/**
 * Lead-story tile for the cockpit editorial layout.
 *
 * Cyanotype lead tile: captured session media in a quiet well over
 * a tone-switchable caption. The new-tab default is the light tone;
 * hover previews can retain the saturated blue treatment.
 */
export function LeadRunTile({
  task,
  now,
  className,
  captionTone = 'light',
}: LeadRunTileProps) {
  const isLive = task.status === 'live'
  const isFailed = task.status === 'failed'
  const isStopped = task.status === 'cancelled'
  const screenshotId = task.latestScreenshotId ?? null
  const screenshotBaseUrl = useTaskScreenshotBaseUrl()
  const location = useLocation()
  return (
    <NavLink
      to={`/audit/${encodeURIComponent(task.sessionId)}`}
      state={{ from: location.pathname }}
      data-testid={`lead-tile-${task.sessionId}`}
      className={cn(
        'group relative flex flex-col overflow-hidden rounded-[9px] border border-cyanotype-border bg-card transition-[border-color,box-shadow] duration-150 hover:border-cyanotype-blue hover:shadow-sm',
        className,
      )}
    >
      <div className="relative flex-1 overflow-hidden border-cyanotype-border border-b bg-cyanotype-well">
        {screenshotId !== null && screenshotBaseUrl !== null ? (
          <img
            src={taskScreenshotUrl(
              task.sessionId,
              screenshotId,
              screenshotBaseUrl,
            )}
            alt={`Session hero from ${task.label}`}
            loading="lazy"
            decoding="async"
            className="absolute inset-0 h-full w-full object-cover object-top"
          />
        ) : screenshotId !== null ? (
          <div className="absolute inset-0 animate-pulse bg-cyanotype-hover" />
        ) : (
          <LeadNoShotComposition task={task} />
        )}
        <span className="pointer-events-none absolute top-3 right-3 flex size-8 items-center justify-center rounded-full bg-white/85 text-cyanotype-ink opacity-0 shadow-sm backdrop-blur-md transition-[opacity,transform] duration-200 group-hover:-translate-y-0.5 group-hover:opacity-100">
          <ArrowUpRight className="size-4" />
        </span>
      </div>
      <Caption
        task={task}
        now={now}
        isLive={isLive}
        isFailed={isFailed}
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
  isFailed,
  isStopped,
  tone,
}: {
  task: TaskSummary
  now: number
  isLive: boolean
  isFailed: boolean
  isStopped: boolean
  tone: ActivityCardCaptionTone
}) {
  const toneClasses = activityCardCaptionTones[tone]
  return (
    <div
      className={cn(
        'relative flex flex-col gap-[7px] px-5 pt-4 pb-[18px]',
        toneClasses.surface,
      )}
      data-caption-tone={tone}
    >
      <div className="flex items-center gap-2 font-medium text-[11.5px] leading-[14px]">
        <span className="truncate">{task.label}</span>
        {isFailed && (
          <span className="rounded-full bg-red-tint px-2 py-0.5 font-bold text-[10px] text-cyanotype-error">
            FAILED
          </span>
        )}
        {isStopped && <span className={toneClasses.subdued}>STOPPED</span>}
      </div>
      <h2 className="truncate font-bold text-[22px] leading-[1.2] tracking-[-0.02em]">
        {task.name}
      </h2>
      <p
        className={cn(
          'text-[11.5px] tabular-nums leading-[1.5]',
          isLive && 'pr-16',
        )}
      >
        {formatDuration(task.durationMs)} · {task.dispatchCount} tool
        {task.dispatchCount === 1 ? '' : 's'} ·{' '}
        {isLive
          ? 'running now'
          : `started ${formatRelative(task.startedAt, now)}`}
      </p>
      <p
        className={cn('truncate text-[11px] leading-[1.5]', isLive && 'pr-16')}
      >
        {abbreviateSequence(task.toolSequence)}
      </p>
      {isLive && (
        <span className="absolute right-3 bottom-4 rounded-full bg-cyanotype-live px-2.5 py-[3px] font-semibold text-[11px] text-cyanotype-live-ink leading-[14px]">
          Live
        </span>
      )}
    </div>
  )
}

function LeadNoShotComposition({ task }: { task: TaskSummary }) {
  const verbs = task.toolSequence.slice(0, 5)
  return (
    <div className="absolute inset-0 bg-cyanotype-well">
      <div className="pointer-events-none absolute inset-0 flex flex-col justify-center gap-1 p-8 text-[30px] text-cyanotype-ink/12 leading-[1.05] tracking-tight md:text-[38px]">
        {verbs.map((verb, idx) => (
          <span
            // biome-ignore lint/suspicious/noArrayIndexKey: tool sequence is stable-ordered per session, not a reorderable list
            key={`${verb}-${idx}`}
            style={{ marginLeft: `${(idx % 3) * 20}px` }}
          >
            {verb}
          </span>
        ))}
      </div>
    </div>
  )
}
