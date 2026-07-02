import { ArrowUpRight } from 'lucide-react'
import { NavLink } from 'react-router'
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
 * The lead-story tile for the cockpit editorial layout. Screenshot
 * dominates the composition; a soft ink scrim over the bottom half
 * makes the caption legible. No card body under the image, no glass
 * chip overlay. Whole tile is a link. Hover raises a tiny arrow in
 * the top-right corner as the only affordance.
 */
export function LeadRunTile({ task, now, className }: LeadRunTileProps) {
  const isLive = task.status === 'live'
  const isFailed = task.status === 'failed'
  const screenshotId = task.lastScreenshotDispatchId
  return (
    <NavLink
      to={`/audit/${encodeURIComponent(task.sessionId)}`}
      data-testid={`lead-tile-${task.sessionId}`}
      className={cn(
        'group relative flex flex-col overflow-hidden rounded-[20px] border border-border-2 transition-[border-color] duration-150 hover:border-accent/40',
        // No fill, no shadow: this is a shell, not a card. The
        // screenshot below carries all the visual weight.
        'bg-bg-sunken',
        className,
      )}
    >
      {screenshotId !== null ? (
        <img
          src={taskScreenshotUrl(screenshotId)}
          alt={`Session hero from ${task.agentLabel}`}
          loading="lazy"
          decoding="async"
          className="absolute inset-0 h-full w-full object-cover"
        />
      ) : (
        <LeadNoShotComposition task={task} />
      )}

      {/* Soft ink scrim over the bottom half so the caption reads
          against the recorded page below. Kept CSS-only, no
          backdrop-blur, no glass. */}
      <div className="pointer-events-none absolute inset-x-0 bottom-0 h-[62%] bg-gradient-to-t from-ink-1/85 via-ink-1/50 to-transparent" />

      {/* Hover cue: a small arrow slides into the top-right corner.
          Uses translate + opacity, both on the composited layer, so
          the underlying image is not re-rasterised. */}
      <span className="pointer-events-none absolute top-4 right-4 flex size-8 items-center justify-center rounded-full bg-white/85 text-ink-1 opacity-0 shadow-sm backdrop-blur-md transition-[opacity,transform] duration-200 group-hover:-translate-y-0.5 group-hover:opacity-100">
        <ArrowUpRight className="size-4" />
      </span>

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
    <div className="relative mt-auto flex flex-col gap-1.5 p-6 pt-16 text-white">
      <div className="flex items-center gap-3 font-mono text-[11.5px] text-white/85 uppercase tracking-[0.08em]">
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
      <h2 className="text-balance font-semibold text-2xl leading-tight tracking-tight md:text-[26px]">
        {task.title}
      </h2>
      <p className="font-mono text-[12.5px] text-white/75 tabular-nums">
        {formatDuration(task.durationMs)}{' '}
        <span className="text-white/50">·</span> {task.dispatchCount} tool
        {task.dispatchCount === 1 ? '' : 's'}{' '}
        <span className="text-white/50">·</span>{' '}
        {isLive
          ? 'running now'
          : `started ${formatRelative(task.startedAt, now)}`}
      </p>
      <p className="truncate font-mono text-[12px] text-white/60">
        {abbreviateSequence(task.toolSequence)}
      </p>
    </div>
  )
}

/**
 * When the lead session has no screenshot yet we make the ink
 * gradient the whole background and let the tool sequence become
 * the composition. This is intentional: the screen still has weight
 * and drama, it just uses type as the material.
 */
function LeadNoShotComposition({ task }: { task: TaskSummary }) {
  const verbs = task.toolSequence.slice(0, 6)
  return (
    <div className="absolute inset-0 bg-gradient-to-br from-ink-1 via-ink-2 to-ink-1">
      <div className="pointer-events-none absolute inset-0 flex flex-col justify-center gap-1 p-10 font-mono text-[38px] text-white/10 leading-[1.1] tracking-tight md:text-[52px]">
        {verbs.map((verb, idx) => (
          <span
            // biome-ignore lint/suspicious/noArrayIndexKey: tool sequence is stable-ordered per session, not a reorderable list
            key={`${verb}-${idx}`}
            style={{ marginLeft: `${(idx % 3) * 24}px` }}
          >
            {verb}
          </span>
        ))}
      </div>
    </div>
  )
}
