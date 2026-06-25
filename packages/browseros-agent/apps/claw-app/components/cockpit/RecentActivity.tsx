import { ArrowRight, History } from 'lucide-react'
import { NavLink } from 'react-router'
import { Skeleton } from '@/components/ui/skeleton'
import { useTasks } from '@/modules/api/audit.hooks'
import { EmptyState } from './EmptyState'
import { TaskCard } from './TaskCard'

const HOME_TASK_LIMIT = 5

/**
 * Homepage Recent activity strip. Renders one TaskCard per recent
 * MCP session (newest first). Skeleton while loading; the empty
 * state appears only when the loaded page is empty.
 */
export function RecentActivity() {
  const query = useTasks({ variables: { limit: HOME_TASK_LIMIT } })
  const tasks = (query.data?.pages ?? [])
    .flatMap((p) => p.tasks)
    .slice(0, HOME_TASK_LIMIT)
  const now = Date.now()

  return (
    <section className="space-y-3">
      <div className="flex items-center gap-2.5">
        <h2 className="font-bold text-base">Recent activity</h2>
        <div className="flex-1" />
        <NavLink
          to="/audit"
          className="group inline-flex items-center gap-1 rounded-md px-2 py-1 font-medium text-[12.5px] text-ink-3 transition hover:bg-card-tint hover:text-ink-1"
        >
          View all activity
          <ArrowRight className="size-3.5 transition group-hover:translate-x-0.5" />
        </NavLink>
      </div>
      {query.isPending ? (
        <div className="space-y-3">
          {['s1', 's2', 's3'].map((id) => (
            <Skeleton key={id} className="h-28 w-full rounded-2xl" />
          ))}
        </div>
      ) : tasks.length === 0 ? (
        <EmptyState
          title="No recent activity"
          hint="Tool calls from connected agents will appear here."
          icon={<History className="size-5" />}
        />
      ) : (
        <div className="space-y-3">
          {tasks.map((task) => (
            <TaskCard key={task.sessionId} task={task} now={now} />
          ))}
        </div>
      )}
    </section>
  )
}
