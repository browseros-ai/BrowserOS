import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router'
import {
  type AgentProfile,
  useAgentProfileDetail,
  useAgentProfiles,
  useRegenerateMcpUrl,
} from '@/modules/api/agents.hooks'

export function useMcpRegistryData() {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const { data: profiles = [], isLoading } = useAgentProfiles()

  // Regenerate rotates the slug and therefore the mcpUrl. Patch the
  // list optimistically so the UI shows the new URL instantly, then
  // invalidate the detail cache for this id so a future Edit visit
  // rehydrates the slug-derived fields from the server.
  const regenerate = useRegenerateMcpUrl({
    onSuccess: ({ id, mcpUrl }) => {
      queryClient.setQueryData<AgentProfile[]>(
        useAgentProfiles.getKey(),
        (prev) =>
          (prev ?? []).map((profile) =>
            profile.id === id ? { ...profile, mcpUrl } : profile,
          ),
      )
      void queryClient.invalidateQueries({
        queryKey: useAgentProfileDetail.getKey({ id }),
      })
    },
  })

  return { profiles, isLoading, regenerate, navigate }
}
