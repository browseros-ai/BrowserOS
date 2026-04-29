import type {
  HarnessAdapterDescriptor,
  HarnessAgentAdapter,
} from '@/entrypoints/app/agents/agent-harness-types'
import type { LlmProviderConfig, ProviderType } from '@/lib/llm-providers/types'

export type SidepanelTargetKind = 'llm' | 'acp'

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

type SidepanelChatTargetSelectionStore = SidepanelChatTargetSelectionReader &
  SidepanelChatTargetSelectionWriter

let sidepanelChatTargetSelectionStorage:
  | SidepanelChatTargetSelectionStore
  | undefined

export function buildSidepanelChatTargets({
  providers,
  adapters,
}: BuildSidepanelChatTargetsInput): SidepanelChatTarget[] {
  return [
    ...providers.map(toLlmTarget),
    ...adapters.flatMap((adapter) =>
      adapter.models.map((model) => {
        const reasoning = adapter.reasoningEfforts.find(
          (effort) => effort.id === adapter.defaultReasoningEffort,
        )
        const reasoningEffort =
          reasoning?.id ?? adapter.defaultReasoningEffort ?? 'medium'
        return {
          kind: 'acp' as const,
          id: buildAcpTargetId(adapter.id, model.id, reasoningEffort),
          name: `${adapter.name} ${model.label}`,
          type: 'acp' as const,
          adapter: adapter.id,
          adapterName: adapter.name,
          modelId: model.id,
          modelLabel: model.label,
          modelControl: adapter.modelControl,
          recommended: model.recommended,
          reasoningEffort,
          reasoningEffortLabel: reasoning?.label,
        }
      }),
    ),
  ]
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

  return (
    targets.find(
      (target) => target.kind === 'llm' && target.id === defaultProviderId,
    ) ?? targets.find((target) => target.kind === 'llm')
  )
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
  const targetStore = store ?? (await getSidepanelChatTargetSelectionStorage())
  await targetStore.setValue(
    target ? { kind: target.kind, id: target.id } : null,
  )
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

export function buildAcpTargetId(
  adapter: HarnessAgentAdapter,
  modelId: string,
  reasoningEffort: string,
): string {
  return `acp:${adapter}:${modelId}:${reasoningEffort}`
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
