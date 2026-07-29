import { ArrowRight } from 'lucide-react'
import { NavLink } from 'react-router'
import { Skeleton } from '@/components/ui/skeleton'
import type { TaskSummary } from '@/modules/api/audit.hooks'
import { formatRelative } from '@/screens/cockpit/cockpit.helpers'

interface RecentPeekProps {
  tasks: TaskSummary[]
  isPending?: boolean
}

const RECENT_PEEK_LIMIT = 2

/** Up to two quiet hairline rows plus one link into the full activity view. */
export function RecentPeek({ tasks, isPending }: RecentPeekProps) {
  const now = Date.now()
  const rows = tasks.slice(0, RECENT_PEEK_LIMIT)

  return (
    <div className="mt-2 flex flex-col gap-1">
      {isPending ? (
        <>
          <Skeleton className="h-4 w-64" />
          <Skeleton className="h-4 w-48" />
        </>
      ) : rows.length === 0 ? (
        <p className="text-ink-3 text-sm">No activity yet</p>
      ) : (
        rows.map((task) => (
          <div
            className="flex items-baseline gap-2 text-sm"
            key={task.sessionId}
          >
            <span className="shrink-0 font-medium text-ink-2">
              {task.label}
            </span>
            <span className="truncate text-ink-3">{task.site}</span>
            <span className="ml-auto shrink-0 font-mono text-ink-3 text-xs tabular-nums">
              {formatRelative(task.startedAt, now)}
            </span>
          </div>
        ))
      )}
      <div className="pt-0.5">
        <NavLink
          className="group inline-flex items-center gap-1 text-ink-3 text-xs transition-colors hover:text-ink"
          to="/audit"
        >
          all activity
          <ArrowRight className="size-3 transition-transform group-hover:translate-x-0.5" />
        </NavLink>
      </div>
    </div>
  )
}
