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

  // Create: invalidate the profiles list so the directory picks up
  // the new row (and its server-derived fields like loginScopeLabel,
  // blockedActionCount, alwaysAllowCount) on next render.
  const createAgent = useCreateAgent({
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: useAgentProfiles.getKey(),
      })
    },
  })

  const profileDetail = useAgentProfileDetail({
    variables: { id: agentId ?? '' },
    enabled: mode === 'edit' && agentId !== null,
  })

  // Update: optimistic patch keeps instant feedback for the cheap
  // fields (name, harness) and an invalidate catches every
  // server-derived field that we'd otherwise leave stale: approvals
  // counts, ACL rule count, slug/mcpUrl rotation after a rename.
  // Detail cache for THIS id also gets invalidated so a re-open of
  // the edit wizard rehydrates from the server.
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
      void queryClient.invalidateQueries({
        queryKey: useAgentProfiles.getKey(),
      })
      void queryClient.invalidateQueries({
        queryKey: useAgentProfileDetail.getKey({ id: variables.id }),
      })
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
