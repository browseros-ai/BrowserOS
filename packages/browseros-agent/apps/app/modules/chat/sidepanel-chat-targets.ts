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
    adapterName: formatAdapterName(agent.type),
    modelId: agent.modelId ?? 'default',
    modelLabel: agent.modelId ?? 'Agent default',
    reasoningEffort: agent.reasoningEffort ?? 'default',
  }
}

function formatAdapterName(adapter: AcpAgentType): string {
  if (adapter === 'claude') return 'Claude Code'
  if (adapter === 'codex') return 'Codex'
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
