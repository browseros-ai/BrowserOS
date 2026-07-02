import { NavLink, useLocation } from 'react-router'
import { AgentDot } from '@/components/audit/AgentDot'
import type { TaskSummary } from '@/modules/api/audit.hooks'
import {
  abbreviateSequence,
  formatDuration,
  formatRelative,
} from '@/screens/audit/audit.helpers'

interface RunRowProps {
  task: TaskSummary
  now: number
}

/**
 * Typographic-tail row for older sessions. Hairline-separated,
 * no card frame, no screenshot. Reads like the "next up" strip on
 * a magazine cover. Row layout mirrors the audit list to give the
 * operator a familiar scan pattern.
 */
export function RunRow({ task, now }: RunRowProps) {
  const isLive = task.status === 'live'
  const location = useLocation()
  return (
    <NavLink
      to={`/audit/${encodeURIComponent(task.sessionId)}`}
      state={{ from: location.pathname }}
      data-testid={`run-row-${task.sessionId}`}
      className="group grid grid-cols-[max-content_1fr_max-content_max-content_max-content] items-center gap-4 border-border-2 border-t px-2 py-3 transition-colors duration-150 hover:bg-card-tint"
    >
      <span className="inline-flex items-center gap-2 font-mono text-[11.5px] text-ink-3 uppercase tracking-[0.06em]">
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
      </span>
      <span className="min-w-0 truncate text-[13px] text-ink-1">
        {task.title}
      </span>
      <span className="hidden truncate font-mono text-[11.5px] text-ink-3 md:inline-block md:max-w-[240px]">
        {abbreviateSequence(task.toolSequence)}
      </span>
      <span className="text-right font-mono text-[11.5px] text-ink-2 tabular-nums">
        {formatDuration(task.durationMs)}
      </span>
      <span className="text-right font-mono text-[11.5px] text-ink-3 tabular-nums">
        {formatRelative(task.startedAt, now)}
      </span>
    </NavLink>
  )
}
