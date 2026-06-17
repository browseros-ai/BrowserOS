import { useQueryClient } from '@tanstack/react-query'
import { useNavigate, useParams } from 'react-router'
import {
  type AgentProfile,
  useAgentProfileDetail,
  useAgentProfiles,
  useCreateAgent,
  useUpdateAgent,
} from '@/modules/api/agents.hooks'

export type AgentWizardMode = 'create' | 'edit'

/**
 * Aggregates everything the wizard needs in either mode. In create
 * mode we lean on useAgentProfiles (for the clone-from card) +
 * useCreateAgent. In edit mode we additionally fetch the full
 * profile detail via useAgentProfileDetail and route updates through
 * useUpdateAgent, whose onSuccess patches the agent-profiles cache
 * so the directory reflects the rename immediately.
 *
 * Clone-from uses the real configured profiles list (not the mocked
 * cockpit running grid) so the card only surfaces when the user
 * actually has agents to copy from.
 */
export function useAgentWizardData(mode: AgentWizardMode) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const { id: paramId } = useParams<{ id: string }>()
  const agentId = mode === 'edit' ? (paramId ?? null) : null

  const { data: existingProfiles = [] } = useAgentProfiles()
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
    existingProfiles,
    queryClient,
    createAgent,
    updateAgent,
    profileDetail,
    navigate,
  }
}
