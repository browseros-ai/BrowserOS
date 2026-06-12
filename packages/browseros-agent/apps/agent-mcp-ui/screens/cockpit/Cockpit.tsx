import { CockpitHero } from '@/components/cockpit/CockpitHero'
import { RecentActivity } from '@/components/cockpit/RecentActivity'
import { RunningGrid } from '@/components/cockpit/RunningGrid'
import { WaitingStrip } from '@/components/cockpit/WaitingStrip'
import { useRecentActivity } from '@/modules/api/activity.hooks'
import { useAgents } from '@/modules/api/agents.hooks'
import { useApprovals, useHandoffs } from '@/modules/api/waiting.hooks'

/**
 * Cockpit home. Four stacked sections matching the design's dashboard
 * order: hero, waiting strip (sticky-attention surface), running
 * grid (the agents themselves), recent activity (cross-agent log).
 *
 * Data comes from mock hooks for now; each hook is `react-query-kit`
 * with a setTimeout fetcher so loading states render and the eventual
 * swap to the real agent-mcp-interface endpoints is a fetcher-body
 * change rather than a refactor.
 */
export function Cockpit() {
  const agents = useAgents()
  const activity = useRecentActivity()
  const approvals = useApprovals()
  const handoffs = useHandoffs()

  return (
    <div className="mx-auto flex max-w-5xl flex-col gap-8 px-8 pt-10 pb-20">
      <CockpitHero />
      <WaitingStrip
        approvals={approvals.data ?? []}
        handoffs={handoffs.data ?? []}
      />
      <RunningGrid agents={agents.data ?? []} />
      <RecentActivity rows={activity.data ?? []} />
    </div>
  )
}
