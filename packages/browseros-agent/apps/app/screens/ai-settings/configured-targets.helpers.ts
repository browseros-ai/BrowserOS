import { adapterLabel } from '@/components/agents/AdapterIcon'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'
import type { TargetRowAction } from './ConfiguredTargetRow'

export function providerDescription(provider: LlmProviderConfig): string {
  return provider.baseUrl
    ? `${provider.modelId} · ${provider.baseUrl}`
    : provider.modelId
}

export function agentDescription(agent: AcpAgent): string {
  return [
    adapterLabel(agent.type),
    agent.modelId ?? 'Agent default model',
    agent.reasoningEffort,
  ]
    .filter((part): part is string => Boolean(part))
    .join(' · ')
}

/**
 * The first action is always "set as default": the row's radio and its hover
 * button both invoke it, and everything after it lands in the overflow menu.
 */
export function buildProviderActions(input: {
  provider: LlmProviderConfig
  isTesting: boolean
  onSelectProvider: (providerId: string) => void
  onTestProvider: (provider: LlmProviderConfig) => void
  onEditProvider: (provider: LlmProviderConfig) => void
  onDeleteProvider: (provider: LlmProviderConfig) => void
}): TargetRowAction[] {
  const { provider, isTesting } = input
  const actions: TargetRowAction[] = [
    {
      label: 'Set as default',
      onSelect: () => input.onSelectProvider(provider.id),
    },
  ]

  actions.push(
    {
      label: isTesting ? 'Testing...' : 'Test connection',
      onSelect: () => input.onTestProvider(provider),
      disabled: isTesting,
    },
    { label: 'Edit', onSelect: () => input.onEditProvider(provider) },
    {
      label: 'Delete',
      onSelect: () => input.onDeleteProvider(provider),
      destructive: true,
    },
  )
  return actions
}

export function buildAgentActions(input: {
  agent: AcpAgent
  isDeleting: boolean
  onSelectAgent: (agentId: string) => void
  onDeleteAgent: (agent: AcpAgent) => void | Promise<void>
}): TargetRowAction[] {
  const { agent, isDeleting } = input
  const actions: TargetRowAction[] = [
    { label: 'Set as default', onSelect: () => input.onSelectAgent(agent.id) },
  ]

  actions.push({
    label: 'Delete',
    onSelect: () => void input.onDeleteAgent(agent),
    destructive: true,
    disabled: isDeleting,
  })
  return actions
}
