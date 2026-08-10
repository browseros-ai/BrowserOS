import { History } from 'lucide-react'
import { NavLink } from 'react-router'
import { Skeleton } from '@/components/ui/skeleton'
import { type TaskSummary, useSessions } from '@/modules/api/audit.hooks'
import { EmptyState } from './EmptyState'
import { LeadRunTile } from './LeadRunTile'
import { RunRow } from './RunRow'
import { SupportingTile } from './SupportingTile'

const HOME_TASK_LIMIT = 12

/**
 * Cockpit editorial layout: lead-story tile + a 2x2 supporting
 * grid + typographic tail. LIVE runs always take the lead slot
 * regardless of start time; everything else stacks newest-first.
 *
 * Grid shape (md and up):
 *
 *   ┌────────────────────────┬────────┬────────┐
 *   │                        │  s1    │  s2    │
 *   │         lead           ├────────┼────────┤
 *   │                        │  s3    │  s4    │
 *   └────────────────────────┴────────┴────────┘
 *
 * Rows are locked to 216px at the bento breakpoint. At mobile the
 * cards stack into a single column and keep an explicit media area.
 */
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
  const lead = ordered[0]
  const supporting = ordered.slice(1, 5)
  const tail = ordered.slice(5)

  return (
    <section className="ph-no-capture space-y-5">
      <SectionHeader sessionCount={ordered.length} />
      {query.isPending ? (
        <BentoSkeleton />
      ) : !lead ? (
        <EmptyState
          title="No recent activity"
          hint="Tool calls from connected agents will appear here."
          icon={<History className="size-5" />}
        />
      ) : (
        <>
          <BentoGrid lead={lead} supporting={supporting} now={now} />
          {tail.length > 0 && <ActivityTable tail={tail} now={now} />}
        </>
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

interface BentoGridProps {
  lead: TaskSummary
  supporting: TaskSummary[]
  now: number
}

function BentoGrid({ lead, supporting, now }: BentoGridProps) {
  return (
    <div className="grid grid-cols-1 gap-3 md:grid-cols-12 md:grid-rows-[216px_216px]">
      <LeadRunTile
        task={lead}
        now={now}
        captionTone="blue"
        className="min-h-[360px] md:col-span-6 md:row-span-2 md:min-h-0"
      />
      {supporting.map((task, idx) => (
        <SupportingTile
          key={task.sessionId}
          task={task}
          now={now}
          className={supportingSlotClass(idx)}
        />
      ))}
    </div>
  )
}

function supportingSlotClass(idx: number): string {
  // Uniform 2x2 grid of supporting cells (3 cols wide, 1 row tall
  // each) to the right of the lead. Every tile has the same
  // footprint so the visual weight of the row is even.
  switch (idx) {
    case 0:
      return 'md:col-span-3 md:col-start-7 md:row-start-1'
    case 1:
      return 'md:col-span-3 md:col-start-10 md:row-start-1'
    case 2:
      return 'md:col-span-3 md:col-start-7 md:row-start-2'
    case 3:
      return 'md:col-span-3 md:col-start-10 md:row-start-2'
    default:
      return 'md:hidden'
  }
}

function ActivityTable({ tail, now }: { tail: TaskSummary[]; now: number }) {
  return (
    <div
      className="overflow-hidden rounded-[9px] border border-cyanotype-border bg-card"
      data-testid="recent-activity-table"
    >
      <div>
        <div className="grid grid-cols-[minmax(0,0.9fr)_minmax(0,1.4fr)_64px_64px] items-center gap-3 bg-cyanotype-blue px-4 py-2 text-[11px] text-white leading-[14px] md:grid-cols-[236px_minmax(0,1fr)_240px_72px_64px] md:gap-4">
          <span>Agent</span>
          <span>Target</span>
          <span className="hidden md:block">Tool chain</span>
          <span className="text-right">Duration</span>
          <span className="text-right">When</span>
        </div>
        <div>
          {tail.map((task) => (
            <RunRow key={task.sessionId} task={task} now={now} />
          ))}
        </div>
      </div>
    </div>
  )
}

function BentoSkeleton() {
  return (
    <div className="grid grid-cols-1 gap-3 md:grid-cols-12 md:grid-rows-[216px_216px]">
      <Skeleton className="min-h-[360px] rounded-[9px] md:col-span-6 md:row-span-2 md:min-h-0" />
      <Skeleton className="min-h-[216px] rounded-[9px] md:col-span-3 md:col-start-7 md:row-start-1 md:min-h-0" />
      <Skeleton className="min-h-[216px] rounded-[9px] md:col-span-3 md:col-start-10 md:row-start-1 md:min-h-0" />
      <Skeleton className="min-h-[216px] rounded-[9px] md:col-span-3 md:col-start-7 md:row-start-2 md:min-h-0" />
      <Skeleton className="min-h-[216px] rounded-[9px] md:col-span-3 md:col-start-10 md:row-start-2 md:min-h-0" />
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
