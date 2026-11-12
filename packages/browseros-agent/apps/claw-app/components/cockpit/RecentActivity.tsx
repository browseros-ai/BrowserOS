import { ArrowRight } from 'lucide-react'
import { NavLink } from 'react-router'
import { TaskRow } from '@/components/audit/TaskRow'
import { Card } from '@/components/motion/card'
import { Skeleton } from '@/components/ui/skeleton'
import { useSessions } from '@/modules/api/audit.hooks'

const RECENT_TASK_COUNT = 4

/** The last few tasks as readable rows, with the way into History. */
export function RecentActivity() {
  const query = useSessions({
    variables: { limit: RECENT_TASK_COUNT },
    // Home is the new tab page: poll so finished tasks appear without a reload.
    refetchInterval: 3000,
  })
  const tasks = (query.data?.pages ?? [])
    .flatMap((page) => page.items)
    .slice(0, RECENT_TASK_COUNT)
  const now = Date.now()

  return (
    <section
      aria-labelledby="recent-tasks"
      className="ph-no-capture flex flex-col gap-3.5"
    >
      <div className="flex items-baseline justify-between gap-4">
        <h2
          id="recent-tasks"
          className="font-bold text-[19px] text-cyanotype-ink tracking-[-0.015em]"
        >
          Recent tasks
        </h2>
        <NavLink
          to="/audit"
          className="inline-flex items-center gap-1.5 font-semibold text-[14px] text-primary underline-offset-4 hover:underline"
        >
          See all in History
          <ArrowRight aria-hidden className="size-[15px]" />
        </NavLink>
      </div>
      <Card
        size="compact"
        className="gap-0 overflow-hidden border-border-2 bg-card py-0 shadow-card"
      >
        {query.isPending ? (
          <RowsSkeleton />
        ) : query.isError ? (
          <p className="px-5 py-6 text-[14.5px] text-ink-2">
            Couldn’t load your recent tasks. Check that BrowserOS is running.
          </p>
        ) : tasks.length === 0 ? (
          <p className="px-5 py-6 text-[14.5px] text-ink-2">
            Finished tasks show up here, with what the agent did.
          </p>
        ) : (
          <div className="flex flex-col divide-y divide-border-2">
            {tasks.map((task) => (
              <TaskRow key={task.sessionId} task={task} now={now} />
            ))}
          </div>
        )}
      </Card>
    </section>
  )
}

// ---------------------------------------------------------------------------

function RowsSkeleton() {
  return (
    <div className="flex flex-col divide-y divide-border-2">
      {['r1', 'r2', 'r3', 'r4'].map((id) => (
        <div
          key={id}
          className="grid grid-cols-[72px_minmax(0,1fr)] items-center gap-4 px-4 py-3.5 sm:grid-cols-[112px_minmax(0,1fr)] sm:px-[18px]"
        >
          <Skeleton className="aspect-[16/10] rounded-md" />
          <div className="flex flex-col gap-2">
            <Skeleton className="h-4 w-2/3" />
            <Skeleton className="h-3.5 w-11/12" />
            <Skeleton className="h-3 w-1/3" />
          </div>
        </div>
      ))}
    </div>
  )
}
