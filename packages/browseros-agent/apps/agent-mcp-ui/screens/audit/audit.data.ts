import { useState } from 'react'
import { type ToolDispatchRow, useDispatches } from '@/modules/api/audit.hooks'
import { type AgentChip, agentChipsFor } from './audit.helpers'

export interface AuditScreenData {
  rows: ToolDispatchRow[]
  chips: AgentChip[]
  isLoading: boolean
  isError: boolean
  hasNextPage: boolean
  isFetchingNextPage: boolean
  fetchNextPage: () => void
  selectedAgentId: string | null
  setSelectedAgentId: (id: string | null) => void
  now: number
}

/**
 * Single data hook for the audit screen. Wires the infinite-query
 * hook, flattens pages, derives per-agent chips for the filter rail.
 * The agent filter is a screen-local state; clearing it returns the
 * unfiltered view.
 */
export function useAuditScreenData(): AuditScreenData {
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null)
  const query = useDispatches({
    variables: selectedAgentId ? { agentId: selectedAgentId } : undefined,
  })
  const rows = (query.data?.pages ?? []).flatMap((p) => p.rows)
  const now = Date.now()

  return {
    rows,
    chips: agentChipsFor(rows),
    isLoading: query.isPending,
    isError: query.isError,
    hasNextPage: Boolean(query.hasNextPage),
    isFetchingNextPage: query.isFetchingNextPage,
    fetchNextPage: () => {
      void query.fetchNextPage()
    },
    selectedAgentId,
    setSelectedAgentId,
    now,
  }
}
