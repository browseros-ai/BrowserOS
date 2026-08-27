import type { FC } from 'react'
import { InlineErrorAlert } from '@/components/agents/PageAlerts'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'
import { CodingAgentCard } from './CodingAgentCard'
import type { CodingAgentsController } from './coding-agents.hooks'

export interface CodingAgentsListProps {
  controller: CodingAgentsController
  selectedAgentId: string | null
  onSelectAgent: (agentId: string) => void
  onEditAgent?: (agent: AcpAgent) => void
}

export const CodingAgentsList: FC<CodingAgentsListProps> = ({
  controller,
  selectedAgentId,
  onSelectAgent,
  onEditAgent,
}) => {
  const { agents, pageError, dismissPageError, deletingAgentId, handleDelete } =
    controller

  if (agents.length === 0 && !pageError) return null

  return (
    <div className="space-y-3">
      {pageError ? (
        <InlineErrorAlert message={pageError} onDismiss={dismissPageError} />
      ) : null}
      {agents.map((agent) => (
        <CodingAgentCard
          key={agent.id}
          agent={agent}
          isSelected={selectedAgentId === agent.id}
          deleting={deletingAgentId === agent.id}
          onSelect={() => onSelectAgent(agent.id)}
          onDelete={(target) => void handleDelete(target)}
          onEdit={onEditAgent}
        />
      ))}
    </div>
  )
}
