import { CockpitHero } from '@/components/cockpit/CockpitHero'
import { RecentActivity } from '@/components/cockpit/RecentActivity'
import { RunningGrid } from '@/components/cockpit/RunningGrid'
import { WaitingStrip } from '@/components/cockpit/WaitingStrip'
import { useCockpitData } from './cockpit.data'

/**
 * Cockpit home. Four stacked sections matching the design's dashboard
 * order: hero, waiting strip (sticky-attention surface), running
 * grid (the agents themselves), recent activity (cross-agent log).
 *
 * PR 1 wires `RunningGrid` and `RecentActivity` to the real
 * `GET /cockpit/tabs/activity` registry; `WaitingStrip`'s approvals
 * and handoffs remain on their mocked hooks until later PRs supply
 * them.
 */
export function Cockpit() {
  const { agents, activity, approvals, handoffs } = useCockpitData()

  return (
    <div className="mx-auto flex max-w-5xl flex-col gap-8 px-8 pt-10 pb-20">
      <CockpitHero />
      <WaitingStrip approvals={approvals} handoffs={handoffs} />
      <RunningGrid agents={agents} />
      <RecentActivity rows={activity} />
    </div>
  )
}
