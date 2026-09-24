import { useState } from 'react'
import { AGENT_DELETED_EVENT } from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'
import { useAcpAgents, useDeleteAcpAgent } from '@/modules/agents/agents.hooks'

export interface CodingAgentsController {
  agents: AcpAgent[]
  loading: boolean
  pageError: string | null
  dismissPageError: () => void
  deletingAgentId: string | null
  handleDelete: (agent: AcpAgent) => Promise<void>
}

export function useCodingAgents(): CodingAgentsController {
  const { agents, loading } = useAcpAgents()
  const deleteAgent = useDeleteAcpAgent()
  const [pageError, setPageError] = useState<string | null>(null)
  const [deletingAgentId, setDeletingAgentId] = useState<string | null>(null)

  const handleDelete = async (agent: AcpAgent) => {
    setDeletingAgentId(agent.id)
    setPageError(null)
    try {
      await deleteAgent.mutateAsync(agent.id)
      track(AGENT_DELETED_EVENT, {
        runtime: 'acp',
        agent_id: agent.id,
      })
      // Nothing to unselect by hand: an agent is a row in the same table the
      // default points into, so deleting it takes the default with it.
    } catch (error) {
      setPageError(error instanceof Error ? error.message : String(error))
    } finally {
      setDeletingAgentId(null)
    }
  }

  return {
    agents,
    loading,
    pageError,
    dismissPageError: () => setPageError(null),
    deletingAgentId,
    handleDelete,
  }
}
