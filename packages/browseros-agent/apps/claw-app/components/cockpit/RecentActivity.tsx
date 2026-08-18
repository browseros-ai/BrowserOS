import { History } from 'lucide-react'
import { NavLink, useLocation } from 'react-router'
import { Skeleton } from '@/components/ui/skeleton'
import { type TaskSummary, useSessions } from '@/modules/api/audit.hooks'
import { formatDuration, formatRelative } from '@/screens/audit/audit.helpers'
import { EmptyState } from './EmptyState'
import { SupportingTile } from './SupportingTile'

// A compact card grid for the freshest runs, then a minimal text list for the
// rest, then the audit link. LIVE runs float to the top of the grid.
const HOME_GRID_COUNT = 6
const HOME_TASK_LIMIT = 12

export function RecentActivity() {
  const query = useSessions({
    variables: { limit: HOME_TASK_LIMIT },
    // Homepage feed: poll so new sessions surface without a manual refresh.
    refetchInterval: 3000,
  })
  const tasks = (query.data?.pages ?? [])
    .flatMap((p) => p.items)
    .slice(0, HOME_TASK_LIMIT)
  const now = Date.now()
  const ordered = orderByLiveThenRecency(tasks)
  const gridTasks = ordered.slice(0, HOME_GRID_COUNT)
  const listTasks = ordered.slice(HOME_GRID_COUNT)

  return (
    <section className="ph-no-capture space-y-5">
      <SectionHeader sessionCount={ordered.length} />
      {query.isPending ? (
        <ActivityGridSkeleton />
      ) : ordered.length === 0 ? (
        <EmptyState
          title="No recent activity"
          hint="Tool calls from connected agents will appear here."
          icon={<History className="size-5" />}
        />
      ) : (
        <div className="space-y-4">
          <div className="grid grid-cols-1 gap-5 sm:grid-cols-2 lg:grid-cols-3">
            {gridTasks.map((task) => (
              <SupportingTile key={task.sessionId} task={task} now={now} />
            ))}
          </div>
          {listTasks.length > 0 && <ActivityList tasks={listTasks} now={now} />}
        </div>
      )}
      <div className="pt-0.5">
        <NavLink
          to="/audit"
          className="group inline-flex items-center gap-2.5 font-medium text-[12px] text-cyanotype-blue leading-4 transition-colors hover:text-cyanotype-blue-hover"
        >
          <span>View all activity</span>
          <span
            aria-hidden
            className="h-px w-[22px] bg-current transition-[width] group-hover:w-8"
          />
        </NavLink>
      </div>
    </section>
  )
}

function SectionHeader({ sessionCount }: { sessionCount: number }) {
  return (
    <header className="flex items-center gap-3.5 pb-1">
      <h2 className="shrink-0 font-medium text-[15px] text-cyanotype-ink leading-[18px]">
        Recent activity
      </h2>
      <span aria-hidden className="h-px flex-1 bg-cyanotype-border" />
      <span className="shrink-0 text-[11px] text-cyanotype-muted tabular-nums leading-[14px]">
        {sessionCount} {sessionCount === 1 ? 'session' : 'sessions'}
      </span>
    </header>
  )
}

/** Minimal one-line-per-run list for the runs beyond the card grid. */
function ActivityList({ tasks, now }: { tasks: TaskSummary[]; now: number }) {
  const location = useLocation()
  return (
    <ul
      className="divide-y divide-cyanotype-border overflow-hidden rounded-[9px] border border-cyanotype-border bg-card"
      data-testid="recent-activity-list"
    >
      {tasks.map((task) => {
        const isLive = task.status === 'live'
        const isStopped = task.status === 'cancelled'
        return (
          <li key={task.sessionId}>
            <NavLink
              to={`/audit/${encodeURIComponent(task.sessionId)}`}
              state={{ from: location.pathname }}
              data-testid={`activity-row-${task.sessionId}`}
              className="flex items-center gap-3 px-4 py-2.5 transition-colors hover:bg-cyanotype-well/60"
            >
              <span className="w-28 shrink-0 truncate font-medium text-[12px] text-cyanotype-ink leading-4">
                {task.label}
              </span>
              <span className="min-w-0 flex-1 truncate text-[12px] text-cyanotype-soft leading-4">
                {task.name}
              </span>
              {isLive && (
                <span className="shrink-0 rounded-full bg-cyanotype-live px-2 py-0.5 font-semibold text-[10px] text-cyanotype-live-ink">
                  LIVE
                </span>
              )}
              {isStopped && (
                <span className="shrink-0 font-medium text-[10px] text-cyanotype-muted">
                  STOPPED
                </span>
              )}
              <span className="shrink-0 text-[11px] text-cyanotype-muted tabular-nums leading-4">
                {formatDuration(task.durationMs)} · {task.dispatchCount}t ·{' '}
                {formatRelative(task.startedAt, now)}
              </span>
            </NavLink>
          </li>
        )
      })}
    </ul>
  )
}

function ActivityGridSkeleton() {
  return (
    <div className="grid grid-cols-1 gap-5 sm:grid-cols-2 lg:grid-cols-3">
      {['s1', 's2', 's3', 's4', 's5', 's6'].map((id) => (
        <Skeleton key={id} className="min-h-[240px] rounded-[9px]" />
      ))}
    </div>
  )
}

/**
 * LIVE runs always float to the top. Within each status group we
 * sort by `startedAt` descending. Exported for unit tests.
 */
export function orderByLiveThenRecency(tasks: TaskSummary[]): TaskSummary[] {
  return [...tasks].sort((a, b) => {
    if (a.status === 'live' && b.status !== 'live') return -1
    if (b.status === 'live' && a.status !== 'live') return 1
    return b.startedAt - a.startedAt
  })
}
