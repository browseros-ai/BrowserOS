import { useMemo, useState } from 'react'
import { AGENT_DELETED_EVENT } from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'
import { sentry } from '@/lib/sentry/sentry'
import type {
  HarnessAdapterDescriptor,
  HarnessAgent,
} from '@/modules/agents/agent-harness-types'
import {
  useAgentAdapters,
  useDeleteHarnessAgent,
  useHarnessAgents,
} from '@/modules/agents/agents.hooks'
import type { AgentListItem } from '@/modules/agents/agents-page-types'
import { toHarnessListItem } from '@/modules/agents/agents-page-utils'
import { clearSidepanelChatTargetSelectionForAgent } from '@/modules/chat/sidepanel-chat-targets'

export type AgentActivity = Record<
  string,
  { status: 'working' | 'idle' | 'asleep' | 'error'; lastUsedAt: number | null }
>

export interface CodingAgentsController {
  adapters: HarnessAdapterDescriptor[]
  agents: HarnessAgent[]
  listItems: AgentListItem[]
  activity: AgentActivity
  harnessAgentLookup: Map<string, HarnessAgent>
  loading: boolean
  pageError: string | null
  dismissPageError: () => void
  deletingAgentKey: string | null
  deleteIsPending: boolean
  handleDelete: (item: AgentListItem) => Promise<void>
}

/** Owns state for Claude Code / Codex agent listing and deletion in AI settings. */
export function useCodingAgents(): CodingAgentsController {
  const { adapters: allAdapters } = useAgentAdapters()
  const { harnessAgents, loading } = useHarnessAgents()
  const deleteHarnessAgent = useDeleteHarnessAgent()

  const adapters = allAdapters
  const adapterIds = useMemo(
    () => new Set(adapters.map((adapter) => adapter.id)),
    [adapters],
  )
  const agents = useMemo(
    () => harnessAgents.filter((agent) => adapterIds.has(agent.adapter)),
    [harnessAgents, adapterIds],
  )

  const [pageError, setPageError] = useState<string | null>(null)
  const [deletingAgentKey, setDeletingAgentKey] = useState<string | null>(null)

  const listItems = useMemo<AgentListItem[]>(
    () => agents.map(toHarnessListItem),
    [agents],
  )
  const harnessAgentLookup = useMemo(() => {
    const map = new Map<string, HarnessAgent>()
    for (const agent of agents) map.set(agent.id, agent)
    return map
  }, [agents])
  const activity = useMemo<AgentActivity>(() => {
    const map: AgentActivity = {}
    for (const agent of agents) {
      if (!agent.status) continue
      map[agent.id] = {
        status: agent.status,
        lastUsedAt: agent.lastUsedAt ?? null,
      }
    }
    return map
  }, [agents])

  const handleDelete = async (item: AgentListItem) => {
    setDeletingAgentKey(item.key)
    setPageError(null)
    try {
      await deleteHarnessAgent.mutateAsync(item.agentId)
      track(AGENT_DELETED_EVENT, {
        runtime: item.source,
        agent_id: item.agentId,
      })
      // Storage cleanup must not surface as a delete failure — the agent is gone.
      await clearSidepanelChatTargetSelectionForAgent(item.agentId).catch(
        (error) => {
          sentry.captureException(error, {
            extra: {
              message: 'Failed to clear chat-target selection after delete',
              agentId: item.agentId,
            },
          })
        },
      )
    } catch (err) {
      setPageError(err instanceof Error ? err.message : String(err))
    } finally {
      setDeletingAgentKey(null)
    }
  }

  return {
    adapters,
    agents,
    listItems,
    activity,
    harnessAgentLookup,
    loading,
    pageError,
    dismissPageError: () => setPageError(null),
    deletingAgentKey,
    deleteIsPending: deleteHarnessAgent.isPending,
    handleDelete,
  }
}
