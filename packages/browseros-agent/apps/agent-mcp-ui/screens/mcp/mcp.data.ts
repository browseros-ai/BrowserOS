import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router'
import {
  type AgentProfile,
  useAgentProfiles,
  useRegenerateMcpUrl,
} from '@/modules/api/agents.hooks'

export function useMcpRegistryData() {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const { data: profiles = [], isLoading } = useAgentProfiles()

  const regenerate = useRegenerateMcpUrl({
    onSuccess: ({ id, mcpUrl }) => {
      queryClient.setQueryData<AgentProfile[]>(
        useAgentProfiles.getKey(),
        (prev) =>
          (prev ?? []).map((profile) =>
            profile.id === id ? { ...profile, mcpUrl } : profile,
          ),
      )
    },
  })

  return { profiles, isLoading, regenerate, navigate }
}
