/**
 * TODO(v2-restore-per-agent): the v2 MCP page reads from
 * `connections.hooks`, not the agent-profile directory. This hook
 * stays on disk for the day per-agent profiles return.
 */

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

  // Regenerate rotates `slug`, `mcpUrl`, and `updatedAt` server-side
  // (the GET /agents list is sorted by `updatedAt` DESC, so the
  // rotated row needs to jump to the top). Patch the visible `mcpUrl`
  // first so the row updates without a network round-trip, then
  // invalidate the list so the next read reconciles the sort order.
  // GET /agents/:id never carries the rotated fields, so its detail
  // cache does not need invalidation.
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
        queryKey: useAgentProfiles.getKey(),
      })
    },
  })

  return { profiles, isLoading, regenerate, navigate }
}
