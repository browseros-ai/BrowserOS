import { CockpitHero } from '@/components/cockpit/CockpitHero'
import { RecentActivity } from '@/components/cockpit/RecentActivity'
import { RunningGrid } from '@/components/cockpit/RunningGrid'
import { useCockpitData } from './cockpit.data'

/**
 * Cockpit home. v2 ships three stacked sections: hero, running grid,
 * recent activity. The waiting strip (approvals + handoffs) is hidden
 * because v2 has no server-side signal source yet; the WaitingStrip
 * component stays on disk with a TODO header naming what brings it
 * back when the permission gate ships in a later phase.
 */
export function Cockpit() {
  const { agents, activity } = useCockpitData()

  return (
    <div className="mx-auto flex max-w-5xl flex-col gap-8 px-8 pt-10 pb-20">
      <CockpitHero />
      <RunningGrid agents={agents} />
      <RecentActivity rows={activity} />
    </div>
  )
}
