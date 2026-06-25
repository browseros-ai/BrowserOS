import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { SidepanelChatTargetSelection } from '@/modules/chat/sidepanel-chat-targets'
// Relative (not `@/`) so this module stays loadable under `bun test`, which
// resolves tsconfig `@/` aliases for erased type imports only, not values.
import { resolveDefaultProviderId } from '../../lib/llm-providers/provider-selection'

export interface ResolveEffectiveDefaultTargetInput {
  providers: LlmProviderConfig[]
  agents: ReadonlyArray<{ id: string }>
  selection: SidepanelChatTargetSelection | null
  defaultProviderId: string
}

/**
 * Resolves which single row (LLM provider or coding agent) the AI-settings
 * pane shows as selected: the persisted chat-target selection when it still
 * points at an existing row, otherwise the resolved default provider —
 * mirroring `resolveSidepanelChatTarget`'s fallback.
 */
export function resolveEffectiveDefaultTarget({
  providers,
  agents,
  selection,
  defaultProviderId,
}: ResolveEffectiveDefaultTargetInput): SidepanelChatTargetSelection {
  if (
    selection?.kind === 'acp' &&
    agents.some((agent) => agent.id === selection.id)
  ) {
    return { kind: 'acp', id: selection.id }
  }
  if (
    selection?.kind === 'llm' &&
    providers.some((provider) => provider.id === selection.id)
  ) {
    return { kind: 'llm', id: selection.id }
  }
  return {
    kind: 'llm',
    id: resolveDefaultProviderId(providers, defaultProviderId),
  }
}

/** Encodes a selection as a Select item value; ids may themselves contain ':'. */
export function encodeTargetValue(
  selection: SidepanelChatTargetSelection,
): string {
  return `${selection.kind}:${selection.id}`
}

export function decodeTargetValue(
  value: string,
): SidepanelChatTargetSelection | null {
  const separatorIndex = value.indexOf(':')
  if (separatorIndex === -1) return null
  const kind = value.slice(0, separatorIndex)
  const id = value.slice(separatorIndex + 1)
  if ((kind !== 'llm' && kind !== 'acp') || !id) return null
  return { kind, id }
}
