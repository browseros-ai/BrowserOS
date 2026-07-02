import { ArrowRight, History } from 'lucide-react'
import { NavLink } from 'react-router'
import { Skeleton } from '@/components/ui/skeleton'
import { type TaskSummary, useTasks } from '@/modules/api/audit.hooks'
import { EmptyState } from './EmptyState'
import { LeadRunTile } from './LeadRunTile'
import { RunRow } from './RunRow'
import { SupportingTile } from './SupportingTile'

const HOME_TASK_LIMIT = 10

/**
 * Cockpit editorial layout: lead-story tile + asymmetric supporting
 * bento + typographic tail. LIVE runs always take the lead slot
 * regardless of start time; everything else stacks newest-first.
 *
 * Grid shape (desktop, `lg` and up):
 *
 *   ┌────────────────────────┬────────┬────────┐
 *   │                        │  s1    │  s2    │
 *   │         lead           ├────────┴────────┤
 *   │                        │      s3         │
 *   │                        ├─────────────────┤
 *   │                        │      s4         │
 *   └────────────────────────┴─────────────────┘
 *
 * At `md`: lead full-width above a 2-column supporting strip.
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
          className="group inline-flex items-center gap-1.5 font-mono text-[12px] text-ink-3 uppercase tracking-[0.08em] transition-colors hover:text-ink-1"
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
    <header className="flex flex-wrap items-baseline justify-between gap-3">
      <h2 className="font-semibold text-ink-1 text-lg">Recent activity</h2>
      <div className="flex items-center gap-3 font-mono text-[11.5px] text-ink-3 uppercase tracking-[0.08em]">
        <FilterChip>Recent 24h</FilterChip>
        <FilterChip>All agents</FilterChip>
        <FilterChip>All sites</FilterChip>
      </div>
    </header>
  )
}

function FilterChip({ children }: { children: React.ReactNode }) {
  // Static text-only chip for now. Dropdown wiring is a separate
  // phase; this reserves the visual slot without pretending to
  // be interactive.
  return (
    <span className="cursor-default rounded-md border border-transparent px-2 py-0.5 transition-colors hover:border-border-2 hover:bg-card-tint">
      {children}
    </span>
  )
}

interface BentoGridProps {
  lead: TaskSummary
  supporting: TaskSummary[]
  now: number
}

function BentoGrid({ lead, supporting, now }: BentoGridProps) {
  return (
    <div className="grid grid-cols-1 gap-3 md:grid-cols-12 md:grid-rows-2">
      <LeadRunTile
        task={lead}
        now={now}
        className="md:col-span-7 md:row-span-2 md:min-h-[420px]"
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
  // Four supporting slots arranged asymmetrically inside the right
  // 5 columns of the bento (see the diagram in the RecentActivity
  // docstring). Slots 0 and 1 sit on top side-by-side, slot 2
  // spans both columns beneath them, slot 3 is a safety net that
  // hides on desktop (drops into the tail instead).
  switch (idx) {
    case 0:
      return 'md:col-span-3 md:col-start-8 md:row-start-1'
    case 1:
      return 'md:col-span-2 md:col-start-11 md:row-start-1'
    case 2:
      return 'md:col-span-5 md:col-start-8 md:row-start-2'
    case 3:
      return 'md:hidden'
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
    <div className="grid grid-cols-1 gap-3 md:grid-cols-12 md:grid-rows-2">
      <Skeleton className="rounded-[20px] md:col-span-7 md:row-span-2 md:min-h-[420px]" />
      <Skeleton className="min-h-[180px] rounded-2xl md:col-span-3 md:col-start-8 md:row-start-1" />
      <Skeleton className="min-h-[180px] rounded-2xl md:col-span-2 md:col-start-11 md:row-start-1" />
      <Skeleton className="min-h-[200px] rounded-2xl md:col-span-5 md:col-start-8 md:row-start-2" />
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
