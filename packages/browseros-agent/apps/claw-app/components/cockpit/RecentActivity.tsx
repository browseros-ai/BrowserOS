import { ArrowRight, History } from 'lucide-react'
import { NavLink } from 'react-router'
import { Skeleton } from '@/components/ui/skeleton'
import { type TaskSummary, useTasks } from '@/modules/api/audit.hooks'
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
 * Rows are locked to `auto-rows-[150px]` so every supporting cell
 * is the same size regardless of internal content. The lead spans
 * both rows so it doubles that height (~300px) without ballooning.
 * At mobile: everything single-column.
 */
export function RecentActivity() {
  const query = useTasks({ variables: { limit: HOME_TASK_LIMIT } })
  const tasks = (query.data?.pages ?? [])
    .flatMap((p) => p.tasks)
    .slice(0, HOME_TASK_LIMIT)
  const now = Date.now()
  const ordered = orderByLiveThenRecency(tasks)
  const lead = ordered[0]
  const supporting = ordered.slice(1, 5)
  const tail = ordered.slice(5)

  return (
    <section className="space-y-4">
      <SectionHeader />
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
          {tail.length > 0 && <Tail tail={tail} now={now} />}
        </>
      )}
      <div className="pt-1">
        <NavLink
          to="/audit"
          className="group inline-flex items-center gap-1.5 font-mono text-[12px] text-ink-3 uppercase tracking-[0.08em] transition-colors hover:text-ink"
        >
          View all activity
          <ArrowRight className="size-3.5 transition-transform group-hover:translate-x-0.5" />
        </NavLink>
      </div>
    </section>
  )
}

function SectionHeader() {
  return (
    <header className="flex items-baseline gap-3">
      <h2 className="font-semibold text-ink text-lg">Recent activity</h2>
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
    <div className="grid grid-cols-1 gap-3 md:grid-cols-12 md:grid-rows-[200px_200px]">
      <LeadRunTile
        task={lead}
        now={now}
        className="md:col-span-6 md:row-span-2"
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

function Tail({ tail, now }: { tail: TaskSummary[]; now: number }) {
  return (
    <div className="pt-2">
      {tail.map((task) => (
        <RunRow key={task.sessionId} task={task} now={now} />
      ))}
    </div>
  )
}

function BentoSkeleton() {
  return (
    <div className="grid grid-cols-1 gap-3 md:grid-cols-12 md:grid-rows-[200px_200px]">
      <Skeleton className="rounded-[18px] md:col-span-6 md:row-span-2" />
      <Skeleton className="rounded-2xl md:col-span-3 md:col-start-7 md:row-start-1" />
      <Skeleton className="rounded-2xl md:col-span-3 md:col-start-10 md:row-start-1" />
      <Skeleton className="rounded-2xl md:col-span-3 md:col-start-7 md:row-start-2" />
      <Skeleton className="rounded-2xl md:col-span-3 md:col-start-10 md:row-start-2" />
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
