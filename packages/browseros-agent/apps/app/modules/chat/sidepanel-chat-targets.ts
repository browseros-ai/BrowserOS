import { agentBrandKey } from '@/components/agents/agent-brand-marks'
import type { LlmProviderConfig, ProviderType } from '@/lib/llm-providers/types'
import type { AcpAgent, AcpAgentType } from '@/modules/agents/acp-agent-types'
import { resolveChatProvider } from '../../lib/llm-providers/provider-runtime'

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

export type SidepanelChatTargetSelection = Pick<
  SidepanelChatTarget,
  'kind' | 'id'
>

export interface BuildSidepanelChatTargetsInput {
  providers: LlmProviderConfig[]
  agents?: AcpAgent[]
}

export interface ResolveSidepanelChatTargetInput {
  targets: SidepanelChatTarget[]
  defaultProviderId: string
  selection?: SidepanelChatTargetSelection | null
}

export interface SidepanelChatTargetSelectionWriter {
  setValue(value: SidepanelChatTargetSelection | null): Promise<void>
}

export interface SidepanelChatTargetSelectionReader {
  getValue(): Promise<SidepanelChatTargetSelection | null>
}

export interface SidepanelChatTargetSelectionWatcher {
  watch(
    callback: (selection: SidepanelChatTargetSelection | null) => void,
  ): () => void
}

type SidepanelChatTargetSelectionStore = SidepanelChatTargetSelectionReader &
  SidepanelChatTargetSelectionWriter &
  SidepanelChatTargetSelectionWatcher

let sidepanelChatTargetSelectionStorage:
  | SidepanelChatTargetSelectionStore
  | undefined

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

export function resolveSidepanelChatTarget({
  targets,
  defaultProviderId,
  selection,
}: ResolveSidepanelChatTargetInput): SidepanelChatTarget | undefined {
  if (selection) {
    const selected = targets.find(
      (target) => target.kind === selection.kind && target.id === selection.id,
    )
    if (selected) return selected
  }

  const llmTargets = targets.filter((target) => target.kind === 'llm')
  const provider = resolveChatProvider(
    llmTargets.map((target) => target.provider),
    defaultProviderId,
  )
  return provider
    ? llmTargets.find((target) => target.id === provider.id)
    : undefined
}

export type RepairSelectionDecision =
  | { repair: false }
  | { repair: true; selection: SidepanelChatTargetSelection | null }

/**
 * Decides whether a persisted sidebar selection needs repair.
 *
 * Repair exists for one case: the selection names something that has been
 * deleted, so the sidebar would otherwise point at nothing. Absence from the
 * list in hand is not evidence of that. Each extension surface holds its own
 * query cache of a list that lives on the server, so a provider added in
 * another surface is missing here until this one refetches, and rewriting the
 * selection then destroys a choice the user just made. That is what made
 * picking a new provider appear to revert to BrowserOS while picking an agent
 * worked, since ACP selections were already exempt.
 *
 * So a selection is only repaired when the list is known to be complete, and a
 * selection this list cannot confirm is left alone for the next render, when
 * the revision signal will have brought the list up to date.
 * `resolveSidepanelChatTarget` already falls back non-destructively for
 * display, so nothing is broken while that happens. Stale entries are still
 * cleaned on delete by `clearSidepanelChatTargetSelectionForAgent`.
 */
export function resolveRepairedSelection({
  selection,
  resolvedTarget,
  ready,
  knownIds,
}: {
  selection: SidepanelChatTargetSelection | null
  resolvedTarget: SidepanelChatTarget | undefined
  ready: boolean
  /**
   * Every id this surface currently knows about, of either kind. A selection
   * naming something absent from it is treated as not yet loaded rather than
   * as deleted.
   */
  knownIds?: ReadonlySet<string>
}): RepairSelectionDecision {
  if (!ready || !selection) return { repair: false }
  if (selection.kind === 'acp') return { repair: false }
  if (
    resolvedTarget &&
    resolvedTarget.kind === selection.kind &&
    resolvedTarget.id === selection.id
  ) {
    return { repair: false }
  }
  // Inconclusive rather than deleted: this surface has not seen it yet.
  if (knownIds && !knownIds.has(selection.id)) return { repair: false }
  return {
    repair: true,
    selection: resolvedTarget
      ? { kind: resolvedTarget.kind, id: resolvedTarget.id }
      : null,
  }
}

export function toLlmProviderConfig(
  target: SidepanelChatTarget | undefined,
): LlmProviderConfig | undefined {
  return target?.kind === 'llm' ? target.provider : undefined
}

export async function persistSidepanelChatTargetSelection(
  target: SidepanelChatTarget | undefined,
  store?: SidepanelChatTargetSelectionWriter,
): Promise<void> {
  await saveSidepanelChatTargetSelection(
    target ? { kind: target.kind, id: target.id } : null,
    store,
  )
}

export async function saveSidepanelChatTargetSelection(
  selection: SidepanelChatTargetSelection | null,
  store?: SidepanelChatTargetSelectionWriter,
): Promise<void> {
  const targetStore = store ?? (await getSidepanelChatTargetSelectionStorage())
  await targetStore.setValue(selection)
}

/**
 * The single "change the selected chat target" side effect, shared by every
 * surface (sidebar, home, settings). Persists the selection and, for an LLM
 * target, also updates the default-provider id so both stores stay consistent.
 * Keeping this in one place is what prevents surfaces from drifting apart.
 */
/**
 * Records the chosen chat target.
 *
 * One write, for either kind. While llm providers and acp agents were separate
 * tables the default could only name an llm one, so this wrote the selection
 * unconditionally and the default only when the kind happened to be llm.
 * Choosing an agent left the default pointing at whichever provider was
 * selected before it, a stale shadow of the real choice.
 */
export async function commitChatTargetSelection(
  selection: SidepanelChatTargetSelection | null,
  deps: { setDefaultProvider: (providerId: string) => Promise<void> },
  store?: SidepanelChatTargetSelectionWriter,
): Promise<void> {
  await saveSidepanelChatTargetSelection(selection, store)
  if (selection) await deps.setDefaultProvider(selection.id)
}

export async function clearSidepanelChatTargetSelectionForAgent(
  agentId: string,
  store?: SidepanelChatTargetSelectionReader &
    SidepanelChatTargetSelectionWriter,
): Promise<void> {
  const targetStore = store ?? (await getSidepanelChatTargetSelectionStorage())
  const selection = await targetStore.getValue()
  if (selection?.kind === 'acp' && selection.id === agentId) {
    await targetStore.setValue(null)
  }
}

export function watchSidepanelChatTargetSelection(
  callback: (selection: SidepanelChatTargetSelection | null) => void,
  store?: SidepanelChatTargetSelectionWatcher,
): () => void {
  if (store) return store.watch(callback)

  let cancelled = false
  let unwatch: (() => void) | undefined
  getSidepanelChatTargetSelectionStorage()
    .then((targetStore) => {
      if (cancelled) return
      unwatch = targetStore.watch(callback)
    })
    .catch(() => undefined)
  return () => {
    cancelled = true
    unwatch?.()
  }
}

export async function loadSidepanelChatTargetSelection(
  store?: SidepanelChatTargetSelectionReader,
): Promise<SidepanelChatTargetSelection | null> {
  const targetStore = store ?? (await getSidepanelChatTargetSelectionStorage())
  return targetStore.getValue()
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

async function getSidepanelChatTargetSelectionStorage(): Promise<SidepanelChatTargetSelectionStore> {
  if (sidepanelChatTargetSelectionStorage) {
    return sidepanelChatTargetSelectionStorage
  }

  const { storage } = await import('@wxt-dev/storage')
  sidepanelChatTargetSelectionStorage =
    storage.defineItem<SidepanelChatTargetSelection | null>(
      'local:sidepanel-chat-target-selection',
      { fallback: null },
    )
  return sidepanelChatTargetSelectionStorage
}
