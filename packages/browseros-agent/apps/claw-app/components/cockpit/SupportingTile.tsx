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

interface SupportingTileProps {
  task: TaskSummary
  now: number
  className?: string
}

/**
 * Supporting tile in the cockpit editorial bento. Two variants
 * driven by data: with-screenshot (top half is the image, bottom
 * half is a hairline-separated meta block) and without-screenshot
 * (the tool sequence is rendered as a vertical typographic list,
 * turning the absence of an image into a composition opportunity).
 * No filled card body, no shadows, hairline border only. Whole
 * tile is a link.
 */
export function SupportingTile({ task, now, className }: SupportingTileProps) {
  const isLive = task.status === 'live'
  const hasShot = task.lastScreenshotDispatchId !== null
  return (
    <NavLink
      to={`/audit/${encodeURIComponent(task.sessionId)}`}
      data-testid={`support-tile-${task.sessionId}`}
      className={cn(
        'group relative flex flex-col overflow-hidden rounded-2xl border border-border-2 bg-card transition-[border-color] duration-150 hover:border-accent/40',
        className,
      )}
    >
      {hasShot ? (
        <WithScreenshot task={task} />
      ) : (
        <WithoutScreenshot task={task} />
      )}
      <MetaBlock task={task} now={now} isLive={isLive} />
      <span className="pointer-events-none absolute top-3 right-3 flex size-6 items-center justify-center rounded-full bg-white/85 text-ink-1 opacity-0 shadow-sm backdrop-blur-md transition-[opacity,transform] duration-200 group-hover:-translate-y-0.5 group-hover:opacity-100">
        <ArrowUpRight className="size-3.5" />
      </span>
    </NavLink>
  )
}

function WithScreenshot({ task }: { task: TaskSummary }) {
  const screenshotId = task.lastScreenshotDispatchId
  if (screenshotId === null) return null
  return (
    <div className="relative aspect-[16/10] w-full overflow-hidden bg-bg-sunken">
      <img
        src={taskScreenshotUrl(screenshotId)}
        alt={`Session preview from ${task.agentLabel}`}
        loading="lazy"
        decoding="async"
        className="absolute inset-0 h-full w-full object-cover"
      />
    </div>
  )
}

function WithoutScreenshot({ task }: { task: TaskSummary }) {
  // Tool sequence as a vertical typographic list. Sits on a warm
  // tinted surface so it visually differs from the screenshot-
  // bearing tiles without being a placeholder.
  const verbs = task.toolSequence.slice(0, 5)
  return (
    <div className="relative aspect-[16/10] w-full overflow-hidden bg-bg-sunken">
      <div className="pointer-events-none absolute inset-0 flex flex-col justify-center gap-0.5 pl-5 font-mono text-[18px] text-ink-3/80 leading-tight tracking-tight">
        {verbs.map((verb, idx) => (
          <span
            // biome-ignore lint/suspicious/noArrayIndexKey: tool sequence is stable-ordered per session, not a reorderable list
            key={`${verb}-${idx}`}
            style={{ marginLeft: `${idx * 8}px` }}
            className="truncate"
          >
            {verb}
          </span>
        ))}
      </div>
    </div>
  )
}

function MetaBlock({
  task,
  now,
  isLive,
}: {
  task: TaskSummary
  now: number
  isLive: boolean
}) {
  return (
    <div className="flex flex-col gap-1 border-border-2 border-t px-4 py-3">
      <div className="flex items-center gap-2 font-mono text-[10.5px] text-ink-3 uppercase tracking-[0.08em]">
        <AgentDot slug={task.slug} />
        <span className="text-ink-2">{task.agentLabel}</span>
        {isLive && (
          <span className="inline-flex items-center gap-1 text-accent">
            <span
              aria-hidden
              className="inline-block size-1.5 animate-[pulse-dot_1.4s_ease-in-out_infinite] rounded-full bg-accent"
            />
            LIVE
          </span>
        )}
      </div>
      <h3 className="truncate font-semibold text-[13.5px] text-ink-1">
        {task.title}
      </h3>
      <p className="font-mono text-[11px] text-ink-3 tabular-nums">
        {formatDuration(task.durationMs)} <span className="text-ink-4">·</span>{' '}
        {task.dispatchCount}t <span className="text-ink-4">·</span>{' '}
        {formatRelative(task.startedAt, now)}
      </p>
      <p className="truncate font-mono text-[10.5px] text-ink-3">
        {abbreviateSequence(task.toolSequence)}
      </p>
    </div>
  )
}
