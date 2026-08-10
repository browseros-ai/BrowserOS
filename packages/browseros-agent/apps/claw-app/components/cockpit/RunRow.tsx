import { NavLink, useLocation } from 'react-router'
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
 * One cyanotype activity-table row. The five columns intentionally
 * share the header's fixed grid so scan lines stay perfectly aligned.
 */
export function RunRow({ task, now }: RunRowProps) {
  const isLive = task.status === 'live'
  const isStopped = task.status === 'cancelled'
  const location = useLocation()
  return (
    <NavLink
      to={`/audit/${encodeURIComponent(task.sessionId)}`}
      state={{ from: location.pathname }}
      data-testid={`run-row-${task.sessionId}`}
      className="group grid grid-cols-[minmax(0,0.9fr)_minmax(0,1.4fr)_64px_64px] items-center gap-3 border-cyanotype-rule border-t px-4 py-2.5 transition-colors duration-150 first:border-t-0 hover:bg-cyanotype-hover md:grid-cols-[236px_minmax(0,1fr)_240px_72px_64px] md:gap-4"
    >
      <span className="inline-flex min-w-0 items-center gap-2 text-[11px] text-cyanotype-blue leading-[14px]">
        <span className="truncate">{task.label}</span>
        {isLive && (
          <span className="font-semibold text-[10px] text-cyanotype-live-text">
            LIVE
          </span>
        )}
        {isStopped && (
          <span className="text-[10px] text-cyanotype-soft">STOPPED</span>
        )}
      </span>
      <span className="min-w-0 truncate text-[13px] text-cyanotype-ink leading-4">
        {task.name}
      </span>
      <span className="hidden truncate text-[11px] text-cyanotype-soft leading-[14px] md:block">
        {abbreviateSequence(task.toolSequence)}
      </span>
      <span className="text-right text-[11px] text-cyanotype-muted tabular-nums leading-[14px]">
        {formatDuration(task.durationMs)}
      </span>
      <span className="text-right text-[11px] text-cyanotype-soft tabular-nums leading-[14px]">
        {formatRelative(task.startedAt, now)}
      </span>
    </NavLink>
  )
}
