import { useNavigate } from 'react-router'
import { useAgents, useCreateAgent } from '@/modules/api/agents.hooks'

export function useNewAgentData() {
  const { data: agents = [] } = useAgents()
  const createAgent = useCreateAgent()
  const navigate = useNavigate()
  return { agents, createAgent, navigate }
}
