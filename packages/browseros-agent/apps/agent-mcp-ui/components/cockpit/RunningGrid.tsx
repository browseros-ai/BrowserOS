import { useNavigate } from 'react-router'
import type { AgentActivityRecord } from '@/screens/cockpit/cockpit.helpers'
import { AddAgentTile } from './AddAgentTile'
import { AgentRunningCard } from './AgentRunningCard'

interface RunningGridProps {
  agents: AgentActivityRecord[]
}

/**
 * Uniform card grid. PR 3 rolls the per-tab records up by agent so
 * each card represents one logical run across however many tabs that
 * agent currently drives. The trailing AddAgentTile reads as "create
 * another profile" so the section feels like a set.
 */
export function RunningGrid({ agents }: RunningGridProps) {
  const navigate = useNavigate()
  const liveCount = agents.filter((a) => a.status === 'active').length

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
          <AgentRunningCard
            key={a.agentId}
            agent={a}
            onWatch={() => navigate(`/run/${a.currentFocus.targetId}`)}
          />
        ))}
        <AddAgentTile />
      </div>
    </section>
  )
}
