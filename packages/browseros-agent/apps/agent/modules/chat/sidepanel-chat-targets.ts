import type { LlmProviderConfig, ProviderType } from '@/lib/llm-providers/types'
import type {
  HarnessAdapterDescriptor,
  HarnessAgent,
  HarnessAgentAdapter,
} from '@/modules/agents/agent-harness-types'
// Relative (not `@/`) so this module stays loadable under `bun test`, which
// resolves tsconfig `@/` aliases for erased type imports only, not values.
import { visibleHarnessAgents } from '../../lib/chat/adapter-visibility'
import {
  isChatProviderType,
  resolveChatProvider,
} from '../../lib/llm-providers/provider-runtime'

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
      adapter: HarnessAgentAdapter
      adapterName: string
      modelId: string
      modelLabel: string
      modelControl: HarnessAdapterDescriptor['modelControl']
      recommended?: boolean
      reasoningEffort: string
      reasoningEffortLabel?: string
    }

export type SidepanelChatTargetSelection = Pick<
  SidepanelChatTarget,
  'kind' | 'id'
>

interface BuildSidepanelChatTargetsInput {
  providers: LlmProviderConfig[]
  adapters: HarnessAdapterDescriptor[]
  agents?: HarnessAgent[]
  hermesAgentSupported?: boolean
}

interface ResolveSidepanelChatTargetInput {
  targets: SidepanelChatTarget[]
  defaultProviderId: string
  selection?: SidepanelChatTargetSelection | null
}

interface SidepanelChatTargetSelectionWriter {
  setValue(value: SidepanelChatTargetSelection | null): Promise<void>
}

interface SidepanelChatTargetSelectionReader {
  getValue(): Promise<SidepanelChatTargetSelection | null>
}

interface SidepanelChatTargetSelectionWatcher {
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
  adapters,
  agents = [],
  hermesAgentSupported = false,
}: BuildSidepanelChatTargetsInput): SidepanelChatTarget[] {
  return [
    ...providers
      .filter((provider) => isChatProviderType(provider.type))
      .map(toLlmTarget),
    ...visibleHarnessAgents(agents, hermesAgentSupported).map((agent) =>
      toAcpTargetForAgent(agent, adapters),
    ),
  ]
}

function toAcpTargetForAgent(
  agent: HarnessAgent,
  adapters: HarnessAdapterDescriptor[],
): SidepanelChatTarget {
  const adapter = adapters.find((entry) => entry.id === agent.adapter)
  const modelId = agent.modelId ?? adapter?.defaultModelId ?? 'default'
  const reasoningEffort =
    agent.reasoningEffort ?? adapter?.defaultReasoningEffort ?? 'medium'
  const model = adapter?.models.find((entry) => entry.id === modelId)
  const reasoning = adapter?.reasoningEfforts.find(
    (effort) => effort.id === reasoningEffort,
  )

  return {
    kind: 'acp',
    id: agent.id,
    name: agent.name,
    type: 'acp',
    agentId: agent.id,
    adapter: agent.adapter,
    adapterName: adapter?.name ?? formatAdapterName(agent.adapter),
    modelId,
    modelLabel: model?.label ?? modelId,
    modelControl: adapter?.modelControl ?? 'best-effort',
    recommended: model?.recommended,
    reasoningEffort,
    reasoningEffortLabel: reasoning?.label,
  }
}

function formatAdapterName(adapter: HarnessAgentAdapter): string {
  if (adapter === 'claude') return 'Claude Code'
  if (adapter === 'codex') return 'Codex'
  if (adapter === 'hermes') return 'Hermes'
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

/** Writes a selection identity (or null to clear) without needing a full target. */
export async function saveSidepanelChatTargetSelection(
  selection: SidepanelChatTargetSelection | null,
  store?: SidepanelChatTargetSelectionWriter,
): Promise<void> {
  const targetStore = store ?? (await getSidepanelChatTargetSelectionStorage())
  await targetStore.setValue(selection)
}

/** Clears the persisted selection only when it points at the given agent. */
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

/**
 * Subscribes to selection changes. The production store loads lazily, so the
 * subscription may attach a tick later; the returned unsubscribe is always
 * synchronous and safe to call before attachment completes.
 */
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
    // Failed storage import leaves the watch inert; this module stays
    // sentry-free for bun-test loadability, and the load path surfaces the
    // same failure to callers, who report it.
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
