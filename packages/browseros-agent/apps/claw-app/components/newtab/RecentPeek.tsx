import { ArrowRight } from 'lucide-react'
import { NavLink, useLocation } from 'react-router'
import { AgentDot } from '@/components/audit/AgentDot'
import type { TaskSummary } from '@/modules/api/audit.hooks'
import { formatRelative } from '@/screens/audit/audit.helpers'

interface RecentPeekProps {
  tasks: TaskSummary[]
  isPending: boolean
}

/** Two to three light rows of recent runs, linking to the full audit list. */
export function RecentPeek({ tasks, isPending }: RecentPeekProps) {
  const now = Date.now()
  const location = useLocation()

  return (
    <div>
      {isPending ? (
        <div className="space-y-1">
          {['r1', 'r2', 'r3'].map((id) => (
            <div
              key={id}
              className="h-8 animate-pulse rounded-lg bg-card-tint"
            />
          ))}
        </div>
      ) : tasks.length === 0 ? (
        <p className="px-2 py-3 text-ink-3 text-sm">No activity yet</p>
      ) : (
        <ul className="space-y-0.5">
          {tasks.map((task) => (
            <li key={task.sessionId}>
              <NavLink
                to={`/audit/${encodeURIComponent(task.sessionId)}`}
                state={{ from: location.pathname }}
                data-testid={`recent-peek-${task.sessionId}`}
                className="grid grid-cols-[max-content_1fr_max-content] items-center gap-3 rounded-lg px-2 py-2 transition-colors hover:bg-card-tint"
              >
                <span className="inline-flex items-center gap-2 text-[13px] text-ink-2">
                  <AgentDot slug={task.slug} />
                  {task.label}
                </span>
                <span className="min-w-0 truncate text-[13px] text-ink-3">
                  {task.site || task.name}
                </span>
                <span className="text-right font-mono text-[11.5px] text-ink-3 tabular-nums">
                  {formatRelative(task.startedAt, now)}
                </span>
              </NavLink>
            </li>
          ))}
        </ul>
      )}
      <div className="pt-2">
        <NavLink
          to="/audit"
          className="group inline-flex items-center gap-1 font-mono text-[11px] text-ink-3 uppercase tracking-[0.08em] transition-colors hover:text-ink"
        >
          all activity
          <ArrowRight className="size-3 transition-transform group-hover:translate-x-0.5" />
        </NavLink>
      </div>
    </div>
  )
}
