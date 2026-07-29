import type { CockpitStats } from '@browseros/claw-api'
import type { TaskSummary } from '@/modules/api/audit.hooks'
import { RecentPeek } from './RecentPeek'
import { StatGlance } from './StatGlance'

interface AgentGlanceProps {
  stats?: CockpitStats
  statsPending: boolean
  recent: TaskSummary[]
  recentPending: boolean
}

/** Quiet idle strip: the value stat plus a short peek at recent runs. */
export function AgentGlance({
  stats,
  statsPending,
  recent,
  recentPending,
}: AgentGlanceProps) {
  return (
    <section className="border-border-2 border-t pt-5">
      <StatGlance stats={stats} isPending={statsPending} />
      <div className="mt-4">
        <RecentPeek tasks={recent} isPending={recentPending} />
      </div>
    </section>
  )
}
