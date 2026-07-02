import { ArrowUpRight } from 'lucide-react'
import { NavLink, useLocation } from 'react-router'
import { AgentDot } from '@/components/audit/AgentDot'
import { cn } from '@/lib/utils'
import { type TaskSummary, taskScreenshotUrl } from '@/modules/api/audit.hooks'
import {
  abbreviateSequence,
  formatDuration,
  formatRelative,
} from '@/screens/audit/audit.helpers'

interface LeadRunTileProps {
  task: TaskSummary
  now: number
  className?: string
}

/**
 * Lead-story tile for the cockpit editorial layout.
 *
 * Split into two zones so the caption never has to fight the
 * screenshot for legibility: the top ~62% is the raw screenshot,
 * the bottom ~38% is a solid dark caption block carrying the
 * session meta in white. The transition between the two is a
 * short gradient fade so the seam does not read as a hard line.
 *
 * Hover raises a tiny arrow chip in the top-right corner as the
 * only affordance. Whole tile is a link.
 */
export function LeadRunTile({ task, now, className }: LeadRunTileProps) {
  const isLive = task.status === 'live'
  const isFailed = task.status === 'failed'
  const screenshotId = task.lastScreenshotDispatchId
  const location = useLocation()
  return (
    <NavLink
      to={`/audit/${encodeURIComponent(task.sessionId)}`}
      state={{ from: location.pathname }}
      data-testid={`lead-tile-${task.sessionId}`}
      className={cn(
        'group relative flex flex-col overflow-hidden rounded-[18px] border border-border-2 bg-bg-sunken transition-[border-color] duration-150 hover:border-accent/40',
        className,
      )}
    >
      <div className="relative flex-1 overflow-hidden">
        {screenshotId !== null ? (
          <img
            src={taskScreenshotUrl(screenshotId)}
            alt={`Session hero from ${task.agentLabel}`}
            loading="lazy"
            decoding="async"
            className="absolute inset-0 h-full w-full object-cover object-top"
          />
        ) : (
          <LeadNoShotComposition task={task} />
        )}
        <span className="pointer-events-none absolute top-3 right-3 flex size-8 items-center justify-center rounded-full bg-white/85 text-ink opacity-0 shadow-sm backdrop-blur-md transition-[opacity,transform] duration-200 group-hover:-translate-y-0.5 group-hover:opacity-100">
          <ArrowUpRight className="size-4" />
        </span>
      </div>
      <Caption task={task} now={now} isLive={isLive} isFailed={isFailed} />
    </NavLink>
  )
}

function Caption({
  task,
  now,
  isLive,
  isFailed,
}: {
  task: TaskSummary
  now: number
  isLive: boolean
  isFailed: boolean
}) {
  return (
    <div className="flex flex-col gap-1 bg-ink px-5 py-3 text-white">
      <div className="flex items-center gap-3 font-mono text-[10.5px] text-white/80 uppercase tracking-[0.08em]">
        <span className="inline-flex items-center gap-1.5">
          <AgentDot slug={task.slug} />
          <span className="text-white">{task.agentLabel}</span>
        </span>
        {isLive && (
          <span className="inline-flex items-center gap-1.5 text-accent">
            <span
              aria-hidden
              className="inline-block size-1.5 animate-[pulse-dot_1.4s_ease-in-out_infinite] rounded-full bg-accent shadow-[0_0_8px_hsl(19_89%_56%/0.7)]"
            />
            LIVE
          </span>
        )}
        {isFailed && (
          <span className="inline-flex items-center gap-1.5 text-red-400">
            <span
              aria-hidden
              className="inline-block size-1.5 rounded-full bg-red-400"
            />
            FAILED
          </span>
        )}
      </div>
      <h2 className="truncate font-semibold text-[17px] text-white leading-tight tracking-tight md:text-[19px]">
        {task.title}
      </h2>
      <p className="font-mono text-[11.5px] text-white/70 tabular-nums">
        {formatDuration(task.durationMs)}{' '}
        <span className="text-white/40">·</span> {task.dispatchCount} tool
        {task.dispatchCount === 1 ? '' : 's'}{' '}
        <span className="text-white/40">·</span>{' '}
        {isLive
          ? 'running now'
          : `started ${formatRelative(task.startedAt, now)}`}
      </p>
      <p className="truncate font-mono text-[11px] text-white/55">
        {abbreviateSequence(task.toolSequence)}
      </p>
    </div>
  )
}

/**
 * When the lead session has no screenshot yet the top zone becomes
 * a dark composition of the tool sequence rendered as large mono
 * type. The absence of an image becomes a design opportunity.
 */
function LeadNoShotComposition({ task }: { task: TaskSummary }) {
  const verbs = task.toolSequence.slice(0, 5)
  return (
    <div className="absolute inset-0 bg-gradient-to-br from-ink via-ink-2 to-ink">
      <div className="pointer-events-none absolute inset-0 flex flex-col justify-center gap-1 p-8 font-mono text-[30px] text-white/12 leading-[1.05] tracking-tight md:text-[38px]">
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
