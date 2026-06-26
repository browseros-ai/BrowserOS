import { useCancelAgent } from '@/modules/api/cancel.hooks'
import { useFocusAgent } from '@/modules/api/focus.hooks'
import type { AgentActivityRecord } from '@/screens/cockpit/cockpit.helpers'
import { AgentRunningCard } from './AgentRunningCard'

interface RunningGridProps {
  agents: AgentActivityRecord[]
}

/** Renders live agent cards and switches to the agent's focus tab on Watch. */
export function RunningGrid({ agents }: RunningGridProps) {
  const focus = useFocusAgent()
  const cancel = useCancelAgent()
  const liveCount = agents.filter((a) => a.status === 'active').length

  if (agents.length === 0) return null

  const onWatch = (agent: AgentActivityRecord) => {
    focus.mutate(
      { agentId: agent.agentId, focusUrl: agent.currentFocus.url },
      {
        onError: (err) => {
          // No toast surface in v2 yet; surface a console line so the
          // operator can read it from devtools while developing.
          // eslint-disable-next-line no-console
          console.warn('focus agent failed', { agentId: agent.agentId, err })
        },
      },
    )
  }
  const onStop = (agentId: string) => {
    cancel.mutate(
      { agentId },
      {
        onError: (err) => {
          // eslint-disable-next-line no-console
          console.warn('cancel agent failed', { agentId, err })
        },
      },
    )
  }
  const pendingAgentId =
    focus.isPending && focus.variables ? focus.variables.agentId : null
  const cancelPendingAgentId =
    cancel.isPending && cancel.variables ? cancel.variables.agentId : null

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
            onWatch={() => onWatch(a)}
            onStop={() => onStop(a.agentId)}
            isFocusPending={pendingAgentId === a.agentId}
            isCancelPending={cancelPendingAgentId === a.agentId}
          />
        ))}
      </div>
    </section>
  )
}
