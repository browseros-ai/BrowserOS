import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router'
import {
  type AgentProfile,
  useAgentProfiles,
  useDeleteAgent,
} from '@/modules/api/agents.hooks'

/**
 * Aggregates the Agents directory's server state. Delete mutation
 * writes back to the `agent-profiles` cache via `setQueryData` so the
 * row disappears immediately without a refetch, per the project
 * convention of treating server-cached lists as the source of truth.
 */
export function useAgentsDirectoryData() {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const { data: profiles = [], isLoading } = useAgentProfiles()

  const deleteAgent = useDeleteAgent({
    onSuccess: ({ id }) => {
      queryClient.setQueryData<AgentProfile[]>(
        useAgentProfiles.getKey(),
        (prev) => (prev ?? []).filter((profile) => profile.id !== id),
      )
    },
  })

  return { profiles, isLoading, deleteAgent, navigate }
}
