import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router'
import {
  type AgentProfile,
  useAgentProfileDetail,
  useAgentProfiles,
  useDeleteAgent,
} from '@/modules/api/agents.hooks'

/**
 * Aggregates the Agents directory's server state. Delete mutation
 * writes back to the `agent-profiles` cache via `setQueryData` so the
 * row disappears immediately without a refetch, then invalidates the
 * detail cache for the removed id so a stale snapshot can't show up
 * if the user navigates back via history.
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
      void queryClient.invalidateQueries({
        queryKey: useAgentProfileDetail.getKey({ id }),
      })
    },
  })

  return { profiles, isLoading, deleteAgent, navigate }
}
