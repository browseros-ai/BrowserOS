import { ArrowUpRight } from 'lucide-react'
import { NavLink, useLocation } from 'react-router'
import { AgentDot } from '@/components/audit/AgentDot'
import { cn } from '@/lib/utils'
import {
  type TaskSummary,
  taskScreenshotUrl,
  useTaskScreenshotBaseUrl,
} from '@/modules/api/audit.hooks'
import { formatDuration, formatRelative } from '@/screens/audit/audit.helpers'

interface SupportingTileProps {
  task: TaskSummary
  now: number
  className?: string
}

/**
 * Supporting tile in the cockpit editorial bento. Mirrors the
 * lead's split-zone structure (visual on top, dark caption block
 * at the bottom) at a smaller scale so all four supporting cells
 * share the lead's visual language. Two variants driven by data:
 * with-screenshot fills the top zone with the captured image;
 * without-screenshot renders the tool sequence as a small
 * typographic composition in place of the image.
 */
export function SupportingTile({ task, now, className }: SupportingTileProps) {
  const isLive = task.status === 'live'
  const screenshotId = task.lastScreenshotDispatchId
  const screenshotBaseUrl = useTaskScreenshotBaseUrl()
  const location = useLocation()
  return (
    <NavLink
      to={`/audit/${encodeURIComponent(task.sessionId)}`}
      state={{ from: location.pathname }}
      data-testid={`support-tile-${task.sessionId}`}
      className={cn(
        'group relative flex flex-col overflow-hidden rounded-2xl border border-border-2 bg-bg-sunken transition-[border-color] duration-150 hover:border-accent/40',
        className,
      )}
    >
      <div className="relative flex-1 overflow-hidden">
        {screenshotId !== null && screenshotBaseUrl !== null ? (
          <img
            src={taskScreenshotUrl(screenshotId, screenshotBaseUrl)}
            alt={`Session preview from ${task.agentLabel}`}
            loading="lazy"
            decoding="async"
            className="absolute inset-0 h-full w-full object-cover object-top"
          />
        ) : screenshotId !== null ? (
          <div className="absolute inset-0 animate-pulse bg-card-tint" />
        ) : (
          <NoShotComposition task={task} />
        )}
        <span className="pointer-events-none absolute top-2.5 right-2.5 flex size-6 items-center justify-center rounded-full bg-white/85 text-ink opacity-0 shadow-sm backdrop-blur-md transition-[opacity,transform] duration-200 group-hover:-translate-y-0.5 group-hover:opacity-100">
          <ArrowUpRight className="size-3.5" />
        </span>
      </div>
      <Caption task={task} now={now} isLive={isLive} />
    </NavLink>
  )
}

function Caption({
  task,
  now,
  isLive,
}: {
  task: TaskSummary
  now: number
  isLive: boolean
}) {
  return (
    <div className="flex flex-col gap-0.5 bg-ink-deep px-3.5 py-2 text-white">
      <div className="flex items-center gap-2 font-mono text-[9.5px] text-white/75 uppercase tracking-[0.08em]">
        <AgentDot slug={task.slug} />
        <span className="truncate text-white/95">{task.agentLabel}</span>
        {isLive && (
          <span className="inline-flex items-center gap-1 text-[#b1dbb8]">
            <span
              aria-hidden
              className="inline-block size-1.5 animate-[pulse-dot_1.4s_ease-in-out_infinite] rounded-full bg-[#b1dbb8]"
            />
            LIVE
          </span>
        )}
      </div>
      <h3 className="truncate font-semibold text-[12.5px] text-white leading-tight">
        {task.title}
      </h3>
      <p className="font-mono text-[10.5px] text-white/65 tabular-nums">
        {formatDuration(task.durationMs)}{' '}
        <span className="text-white/35">·</span> {task.dispatchCount}t{' '}
        <span className="text-white/35">·</span>{' '}
        {formatRelative(task.startedAt, now)}
      </p>
    </div>
  )
}

function NoShotComposition({ task }: { task: TaskSummary }) {
  // Small typographic composition, still on the same dark ink
  // background so the caption block flows continuously into the
  // top zone. Keeps the tile a single dark object rather than a
  // two-color card.
  const verbs = task.toolSequence.slice(0, 4)
  return (
    <div className="absolute inset-0 bg-gradient-to-br from-ink via-ink-2 to-ink">
      <div className="pointer-events-none absolute inset-0 flex flex-col justify-center gap-0.5 pl-4 font-mono text-[14px] text-white/18 leading-tight tracking-tight">
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
