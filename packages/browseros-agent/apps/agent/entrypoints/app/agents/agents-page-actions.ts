import type { NavigateFunction } from 'react-router'
import {
  AGENT_CREATED_EVENT,
  AGENT_DELETED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'
import type { HarnessAgent, HarnessAgentAdapter } from './agent-harness-types'
import type {
  AgentListItem,
  CreateAgentRuntime,
  ProviderOption,
} from './agents-page-types'
import { findOpenClawCliProviderById } from './openclaw-cli-providers'
import type {
  AgentEntry,
  OpenClawAgentMutationInput,
  OpenClawSetupInput,
} from './useOpenClaw'

export interface AgentPageActionInput {
  createProviderId: string
  createRuntime: CreateAgentRuntime
  harnessModelId: string
  harnessReasoningEffort: string
  navigate: NavigateFunction
  newName: string
  selectableOpenClawProviders: ProviderOption[]
  setupProviderId: string
  createHarnessAgent: (input: {
    name: string
    adapter: HarnessAgentAdapter
    modelId?: string
    reasoningEffort?: string
  }) => Promise<HarnessAgent>
  createOpenClawAgent: (
    input: OpenClawAgentMutationInput,
  ) => Promise<{ agent: AgentEntry }>
  deleteHarnessAgent: (agentId: string) => Promise<unknown>
  deleteOpenClawAgent: (agentId: string) => Promise<unknown>
  setCliAuthModalOpen: (open: boolean) => void
  setCreateError: (error: string | null) => void
  setCreateOpen: (open: boolean) => void
  setDeletingAgentKey: (key: string | null) => void
  setNewName: (name: string) => void
  setPageError: (error: string | null) => void
  setSetupOpen: (open: boolean) => void
  setupOpenClaw: (input: OpenClawSetupInput) => Promise<unknown>
}

export function createAgentPageActions(input: AgentPageActionInput) {
  const runWithPageErrorHandling = async (fn: () => Promise<unknown>) => {
    input.setPageError(null)
    try {
      await fn()
    } catch (err) {
      input.setPageError(err instanceof Error ? err.message : String(err))
    }
  }

  const handleSetup = async () => {
    const option = input.selectableOpenClawProviders.find(
      (item) => item.id === input.setupProviderId,
    )
    const isCli = !!option && !!findOpenClawCliProviderById(option.type)
    const llmOption = !isCli && option ? option : undefined

    await runWithPageErrorHandling(async () => {
      await input.setupOpenClaw({
        providerType: option?.type,
        providerName: isCli ? undefined : option?.name,
        baseUrl: llmOption?.baseUrl,
        apiKey: llmOption?.apiKey,
        modelId: option?.modelId,
      })
      input.setSetupOpen(false)
      if (isCli) input.setCliAuthModalOpen(true)
    })
  }

  const handleOpenClawCreate = async () => {
    if (!input.newName.trim()) return
    const option = input.selectableOpenClawProviders.find(
      (item) => item.id === input.createProviderId,
    )
    const normalizedName = input.newName
      .trim()
      .toLowerCase()
      .replace(/\s+/g, '-')
    const isCli = !!option && !!findOpenClawCliProviderById(option.type)
    const llmOption = !isCli && option ? option : undefined

    input.setCreateError(null)
    try {
      const result = await input.createOpenClawAgent({
        name: normalizedName,
        providerType: option?.type,
        providerName: isCli ? undefined : option?.name,
        baseUrl: llmOption?.baseUrl,
        apiKey: llmOption?.apiKey,
        modelId: option?.modelId,
      })
      input.setCreateOpen(false)
      input.setNewName('')
      track(AGENT_CREATED_EVENT, {
        runtime: 'openclaw',
        provider_type: option?.type,
      })
      input.navigate(`/agents/${result.agent.agentId}`)
    } catch (err) {
      input.setCreateError(err instanceof Error ? err.message : String(err))
    }
  }

  const handleHarnessCreate = async () => {
    if (!input.newName.trim()) return

    input.setCreateError(null)
    try {
      const agent = await input.createHarnessAgent({
        name: input.newName.trim(),
        adapter: input.createRuntime as HarnessAgentAdapter,
        modelId: input.harnessModelId || undefined,
        reasoningEffort: input.harnessReasoningEffort || undefined,
      })
      input.setCreateOpen(false)
      input.setNewName('')
      track(AGENT_CREATED_EVENT, {
        runtime: input.createRuntime,
        model_id: input.harnessModelId || undefined,
        reasoning_effort: input.harnessReasoningEffort || undefined,
      })
      input.navigate(`/agents/${agent.id}`)
    } catch (err) {
      input.setCreateError(err instanceof Error ? err.message : String(err))
    }
  }

  const handleCreate = () => {
    const createByRuntime: Record<CreateAgentRuntime, () => Promise<void>> = {
      openclaw: handleOpenClawCreate,
      claude: handleHarnessCreate,
      codex: handleHarnessCreate,
    }
    void createByRuntime[input.createRuntime]()
  }

  const handleDelete = async (agent: AgentListItem) => {
    input.setDeletingAgentKey(agent.key)
    await runWithPageErrorHandling(async () => {
      const deleteBySource: Record<
        AgentListItem['source'],
        (agentId: string) => Promise<unknown>
      > = {
        openclaw: (agentId) => input.deleteOpenClawAgent(agentId),
        'agent-harness': (agentId) => input.deleteHarnessAgent(agentId),
      }
      await deleteBySource[agent.source](agent.agentId)
      track(AGENT_DELETED_EVENT, {
        runtime: agent.source,
        agent_id: agent.agentId,
      })
    })
    input.setDeletingAgentKey(null)
  }

  return {
    handleCreate,
    handleDelete,
    handleSetup,
    runWithPageErrorHandling,
  }
}
