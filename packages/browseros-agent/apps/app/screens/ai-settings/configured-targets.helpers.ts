import { adapterLabel } from '@/components/agents/AdapterIcon'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'
import type { TargetRowAction } from './ConfiguredTargetRow'

export function providerDescription(
  provider: LlmProviderConfig,
  isBuiltIn: boolean,
): string {
  if (isBuiltIn) return 'BrowserOS-hosted model with strict rate limits'
  return provider.baseUrl
    ? `${provider.modelId} · ${provider.baseUrl}`
    : provider.modelId
}

/** A custom agent identifies itself by its launch command, not its adapter. */
export function agentDescription(agent: AcpAgent): string {
  const isCustom = agent.type === 'custom'
  const primaryLabel =
    isCustom && agent.customConfig?.command
      ? agent.customConfig.command
      : adapterLabel(agent.type)

  return [
    primaryLabel,
    agent.modelId ?? 'Agent default model',
    agent.reasoningEffort,
  ]
    .filter((part): part is string => Boolean(part))
    .join(' · ')
}

/**
 * The first action is always "set as default": the row's radio and its
 * hover button both invoke it, and everything after it lands in the overflow
 * menu. The built-in BrowserOS provider cannot be tested, edited or deleted,
 * so it gets that one action and no menu.
 */
export function buildProviderActions(input: {
  provider: LlmProviderConfig
  isBuiltIn: boolean
  isTesting: boolean
  onSelectProvider: (providerId: string) => void
  onTestProvider: (provider: LlmProviderConfig) => void
  onEditProvider: (provider: LlmProviderConfig) => void
  onDeleteProvider: (provider: LlmProviderConfig) => void
}): TargetRowAction[] {
  const { provider, isBuiltIn, isTesting } = input
  const actions: TargetRowAction[] = [
    {
      label: 'Set as default',
      onSelect: () => input.onSelectProvider(provider.id),
    },
  ]
  if (isBuiltIn) return actions

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

/** Only custom agents are editable, and only when a handler is supplied. */
export function buildAgentActions(input: {
  agent: AcpAgent
  isDeleting: boolean
  onSelectAgent: (agentId: string) => void
  onDeleteAgent: (agent: AcpAgent) => void | Promise<void>
  onEditAgent?: (agent: AcpAgent) => void
}): TargetRowAction[] {
  const { agent, isDeleting } = input
  const actions: TargetRowAction[] = [
    { label: 'Set as default', onSelect: () => input.onSelectAgent(agent.id) },
  ]

  if (agent.type === 'custom' && input.onEditAgent) {
    const onEditAgent = input.onEditAgent
    actions.push({ label: 'Edit', onSelect: () => onEditAgent(agent) })
  }

  actions.push({
    label: 'Delete',
    onSelect: () => void input.onDeleteAgent(agent),
    destructive: true,
    disabled: isDeleting,
  })
  return actions
}
