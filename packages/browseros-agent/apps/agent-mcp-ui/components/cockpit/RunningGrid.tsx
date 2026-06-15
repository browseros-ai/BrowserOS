import { useNavigate } from 'react-router'
import { isActiveStatus } from '@/lib/status'
import type { AgentRow } from '@/modules/api/agents.hooks'
import { AddAgentTile } from './AddAgentTile'
import { RunningCard } from './RunningCard'

interface RunningGridProps {
  agents: AgentRow[]
}

/**
 * Uniform card grid. Renders one card per agent plus a trailing
 * AddAgentTile so "create another profile" reads as adding to the
 * set, not a floating header CTA. The live-count chip on the header
 * surfaces the most-useful at-a-glance metric.
 */
export function RunningGrid({ agents }: RunningGridProps) {
  const navigate = useNavigate()
  const liveCount = agents.filter((a) => isActiveStatus(a.status)).length

  return (
    <section className="space-y-3">
      <div className="flex items-center gap-2.5">
        <h2 className="font-bold text-base">Running now</h2>
        <span className="inline-flex items-center gap-1.5 rounded-full bg-green-tint px-2 py-0.5 font-bold text-[11px] text-green">
          <span
            aria-hidden
            className="size-1.5 animate-pulse-dot rounded-full bg-green"
          />
          {liveCount} live
        </span>
      </div>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(258px,1fr))] items-start gap-3.5">
        {agents.map((a) => (
          <RunningCard
            key={a.id}
            agent={a}
            onWatch={() => navigate(`/run/${a.id}`)}
          />
        ))}
        <AddAgentTile />
      </div>
    </section>
  )
}
