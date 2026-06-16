import { useQueryClient } from '@tanstack/react-query'
import { useNavigate, useParams } from 'react-router'
import {
  type AgentProfile,
  useAgentProfileDetail,
  useAgentProfiles,
  useAgents,
  useCreateAgent,
  useUpdateAgent,
} from '@/modules/api/agents.hooks'

export type AgentWizardMode = 'create' | 'edit'

/**
 * Aggregates everything the wizard needs in either mode. In create
 * mode we lean on useAgents (for the clone-from card) + useCreateAgent.
 * In edit mode we additionally fetch the full profile detail via
 * useAgentProfileDetail and route updates through useUpdateAgent,
 * whose onSuccess patches the agent-profiles cache so the directory
 * reflects the rename immediately.
 */
export function useAgentWizardData(mode: AgentWizardMode) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const { id: paramId } = useParams<{ id: string }>()
  const agentId = mode === 'edit' ? (paramId ?? null) : null

  const { data: agents = [] } = useAgents()
  const createAgent = useCreateAgent()

  const profileDetail = useAgentProfileDetail({
    variables: { id: agentId ?? '' },
    enabled: mode === 'edit' && agentId !== null,
  })

  const updateAgent = useUpdateAgent({
    onSuccess: (variables) => {
      queryClient.setQueryData<AgentProfile[]>(
        useAgentProfiles.getKey(),
        (prev) =>
          (prev ?? []).map((profile) =>
            profile.id === variables.id
              ? { ...profile, name: variables.name, harness: variables.harness }
              : profile,
          ),
      )
    },
  })

  return {
    mode,
    agentId,
    agents,
    createAgent,
    updateAgent,
    profileDetail,
    navigate,
  }
}
