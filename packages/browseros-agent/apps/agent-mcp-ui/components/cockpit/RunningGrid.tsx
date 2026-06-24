import { Link, useNavigate } from 'react-router'
import type { AgentActivityRecord } from '@/screens/cockpit/cockpit.helpers'
import { AgentRunningCard } from './AgentRunningCard'
import { EmptyState } from './EmptyState'

interface RunningGridProps {
  agents: AgentActivityRecord[]
}

/**
 * One uniform card per rolled-up agent. v2 has no per-agent profile
 * directory, so the trailing "New profile" tile is gone; the
 * AddAgentTile file stays on disk with a TODO header that names what
 * brings it back. When the registry is empty, the section keeps its
 * header and renders a centred empty-state card pointing the operator
 * at the MCP page.
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
      {agents.length === 0 ? (
        <EmptyState
          title="No agents connected"
          hint={
            <>
              Open the{' '}
              <Link
                to="/mcp"
                className="font-semibold text-accent-ink underline-offset-2 hover:underline"
              >
                MCP page
              </Link>{' '}
              for the URL to paste into Claude Code, Cursor, or Codex.
            </>
          }
        />
      ) : (
        <div className="grid grid-cols-[repeat(auto-fill,minmax(258px,1fr))] items-start gap-3.5">
          {agents.map((a) => (
            <AgentRunningCard
              key={a.agentId}
              agent={a}
              onWatch={() => navigate(`/run/${a.currentFocus.targetId}`)}
            />
          ))}
        </div>
      )}
    </section>
  )
}
