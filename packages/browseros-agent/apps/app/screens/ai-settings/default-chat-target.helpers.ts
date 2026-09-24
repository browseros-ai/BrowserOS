import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { ChatTargetRef } from '@/modules/chat/sidepanel-chat-targets'
// Relative (not `@/`) so this module stays loadable under `bun test`, which
// resolves tsconfig `@/` aliases for erased type imports only, not values.
import { resolveDefaultProviderId } from '../../lib/llm-providers/provider-selection'

export interface ResolveEffectiveDefaultTargetInput {
  providers: LlmProviderConfig[]
  agents: ReadonlyArray<{ id: string }>
  /** The stored id verbatim, which names a row of either kind. */
  defaultTargetId: string | null
}

/**
 * Which single row (LLM provider or coding agent) the pane shows as selected.
 *
 * The kind is read off whichever list holds the id rather than stored beside
 * it. Agents are checked first only because the id is unique across both, so
 * order decides nothing. Falling back through `resolveDefaultProviderId` keeps
 * the LLM-only behaviour for an id that names nothing.
 *
 * Null when nothing is configured, so the radio group shows no selection rather
 * than one naming a row that is not there.
 */
export function resolveEffectiveDefaultTarget({
  providers,
  agents,
  defaultTargetId,
}: ResolveEffectiveDefaultTargetInput): ChatTargetRef | null {
  if (defaultTargetId) {
    if (agents.some((agent) => agent.id === defaultTargetId)) {
      return { kind: 'acp', id: defaultTargetId }
    }
    if (providers.some((provider) => provider.id === defaultTargetId)) {
      return { kind: 'llm', id: defaultTargetId }
    }
  }
  const resolvedId = resolveDefaultProviderId(providers, defaultTargetId)
  return resolvedId ? { kind: 'llm', id: resolvedId } : null
}
