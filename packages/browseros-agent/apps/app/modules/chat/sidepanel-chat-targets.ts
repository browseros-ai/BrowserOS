import { agentBrandKey } from '@/components/agents/agent-brand-marks'
import type { LlmProviderConfig, ProviderType } from '@/lib/llm-providers/types'
import type { AcpAgent, AcpAgentType } from '@/modules/agents/acp-agent-types'

export type SidepanelChatTarget =
  | {
      kind: 'llm'
      id: string
      name: string
      type: ProviderType
      provider: LlmProviderConfig
    }
  | {
      kind: 'acp'
      id: string
      name: string
      type: 'acp'
      agentId: string
      agentType: AcpAgentType
      /** Brand id for the agent's logo (its type, or a popular-agent id). */
      brandKey?: string
      adapterName: string
      modelId: string
      modelLabel: string
      reasoningEffort: string
    }

/** A chat target named by kind and id, derived from the lists in hand. */
export type ChatTargetRef = Pick<SidepanelChatTarget, 'kind' | 'id'>

export interface BuildSidepanelChatTargetsInput {
  providers: LlmProviderConfig[]
  agents?: AcpAgent[]
}

export interface ResolveSidepanelChatTargetInput {
  targets: SidepanelChatTarget[]
  /**
   * The chosen target, which names a row of either kind. Named a target id
   * rather than a provider id because calling it a provider is what led a
   * caller to resolve it through the LLM-only list first.
   */
  defaultTargetId: string | null
}

export function buildSidepanelChatTargets({
  providers,
  agents = [],
}: BuildSidepanelChatTargetsInput): SidepanelChatTarget[] {
  return [...providers.map(toLlmTarget), ...agents.map(toAcpTargetForAgent)]
}

function toAcpTargetForAgent(agent: AcpAgent): SidepanelChatTarget {
  return {
    kind: 'acp',
    id: agent.id,
    name: agent.name,
    type: 'acp',
    agentId: agent.id,
    agentType: agent.type,
    brandKey: agentBrandKey(agent),
    adapterName: formatAdapterName(agent.type),
    modelId: agent.modelId ?? 'default',
    modelLabel: agent.modelId ?? 'Agent default',
    reasoningEffort: agent.reasoningEffort ?? 'default',
  }
}

function formatAdapterName(adapter: AcpAgentType): string {
  if (adapter === 'claude') return 'Claude Code'
  if (adapter === 'codex') return 'Codex'
  if (adapter === 'custom') return 'Custom agent'
  return adapter
}

/**
 * The target a surface should use: the chosen one, otherwise the first there is.
 *
 * The fallback spans both kinds. It used to consider only LLM providers, which
 * was invisible while a built-in provider was always seeded and guaranteed one.
 * With nothing seeded, someone who connected only a coding agent would resolve
 * to nothing at all. The default pointer names a row of either kind, so
 * matching on id across the whole list is both simpler and closer to what it
 * means.
 */
export function resolveSidepanelChatTarget({
  targets,
  defaultTargetId,
}: ResolveSidepanelChatTargetInput): SidepanelChatTarget | undefined {
  if (defaultTargetId) {
    const named = targets.find((target) => target.id === defaultTargetId)
    if (named) return named
  }
  return targets[0]
}

export function toLlmProviderConfig(
  target: SidepanelChatTarget | undefined,
): LlmProviderConfig | undefined {
  return target?.kind === 'llm' ? target.provider : undefined
}

function toLlmTarget(provider: LlmProviderConfig): SidepanelChatTarget {
  return {
    kind: 'llm',
    id: provider.id,
    name: provider.name,
    type: provider.type,
    provider,
  }
}
