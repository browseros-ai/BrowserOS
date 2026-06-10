import { useEffect, useMemo, useState } from 'react'
import {
  AGENT_CREATED_EVENT,
  AGENT_DELETED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'
import { sentry } from '@/lib/sentry/sentry'
import type {
  HarnessAdapterDescriptor,
  HarnessAgent,
  HarnessAgentAdapter,
} from '@/modules/agents/agent-harness-types'
import {
  useAgentAdapters,
  useCreateHarnessAgent,
  useDeleteHarnessAgent,
  useHarnessAgents,
} from '@/modules/agents/agents.hooks'
import { useDefaultAgentName } from '@/modules/agents/agents-page.hooks'
import type { AgentListItem } from '@/modules/agents/agents-page-types'
import { toHarnessListItem } from '@/modules/agents/agents-page-utils'
import { clearSidepanelChatTargetSelectionForAgent } from '@/modules/chat/sidepanel-chat-targets'

type AgentActivity = Record<
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
  createOpen: boolean
  createAdapter: HarnessAdapterDescriptor | undefined
  createAdapterId: HarnessAgentAdapter | null
  newName: string
  modelId: string
  reasoningEffort: string
  createError: string | null
  creating: boolean
  openCreate: (adapterId: HarnessAgentAdapter) => void
  closeCreate: () => void
  handleCreate: () => Promise<void>
  handleDelete: (item: AgentListItem) => Promise<void>
  setNewName: (value: string) => void
  setModelId: (value: string) => void
  setReasoningEffort: (value: string) => void
}

/**
 * Owns all state for the quick coding-agent (Claude Code / Codex) surface so
 * the trigger cards (in the provider-templates grid) and the management list
 * (at the bottom of the pane) can share one create dialog. Hermes is filtered
 * out here regardless of capability — it's hidden for now.
 */
export function useCodingAgents(): CodingAgentsController {
  const { adapters: allAdapters } = useAgentAdapters()
  const { harnessAgents, loading } = useHarnessAgents()
  const createHarnessAgent = useCreateHarnessAgent()
  const deleteHarnessAgent = useDeleteHarnessAgent()

  const adapters = useMemo(
    () => allAdapters.filter((adapter) => adapter.id !== 'hermes'),
    [allAdapters],
  )
  const adapterIds = useMemo(
    () => new Set(adapters.map((adapter) => adapter.id)),
    [adapters],
  )
  const agents = useMemo(
    () => harnessAgents.filter((agent) => adapterIds.has(agent.adapter)),
    [harnessAgents, adapterIds],
  )

  const [createAdapterId, setCreateAdapterId] =
    useState<HarnessAgentAdapter | null>(null)
  const [newName, setNewName] = useState('')
  const [modelId, setModelId] = useState('')
  const [reasoningEffort, setReasoningEffort] = useState('')
  const [createError, setCreateError] = useState<string | null>(null)
  const [pageError, setPageError] = useState<string | null>(null)
  const [deletingAgentKey, setDeletingAgentKey] = useState<string | null>(null)

  const createOpen = createAdapterId !== null
  const createAdapter = adapters.find(
    (adapter) => adapter.id === createAdapterId,
  )

  useDefaultAgentName(createOpen, setNewName)
  // Seed model/reasoning from the chosen adapter's defaults when the dialog opens.
  useEffect(() => {
    if (!createOpen || !createAdapter) return
    setModelId((current) => current || createAdapter.defaultModelId)
    setReasoningEffort(
      (current) => current || createAdapter.defaultReasoningEffort,
    )
  }, [createOpen, createAdapter])

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

  const openCreate = (adapterId: HarnessAgentAdapter) => {
    setCreateError(null)
    setModelId('')
    setReasoningEffort('')
    setCreateAdapterId(adapterId)
  }

  const closeCreate = () => {
    setCreateAdapterId(null)
    setCreateError(null)
    setNewName('')
    createHarnessAgent.reset()
  }

  const handleCreate = async () => {
    if (!newName.trim() || !createAdapterId) return
    setCreateError(null)
    try {
      await createHarnessAgent.mutateAsync({
        name: newName.trim(),
        adapter: createAdapterId,
        modelId: modelId || undefined,
        reasoningEffort: reasoningEffort || undefined,
      })
      track(AGENT_CREATED_EVENT, {
        runtime: createAdapterId,
        model_id: modelId || undefined,
        reasoning_effort: reasoningEffort || undefined,
      })
      setCreateAdapterId(null)
      setNewName('')
    } catch (err) {
      setCreateError(err instanceof Error ? err.message : String(err))
    }
  }

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
    createOpen,
    createAdapter,
    createAdapterId,
    newName,
    modelId,
    reasoningEffort,
    createError,
    creating: createHarnessAgent.isPending,
    openCreate,
    closeCreate,
    handleCreate,
    handleDelete,
    setNewName,
    setModelId,
    setReasoningEffort,
  }
}
