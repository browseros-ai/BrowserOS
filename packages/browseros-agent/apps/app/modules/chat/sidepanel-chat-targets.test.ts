import { describe, expect, it } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type {
  HarnessAdapterDescriptor,
  HarnessAgent,
} from '@/modules/agents/agent-harness-types'
import {
  buildSidepanelChatTargets,
  clearSidepanelChatTargetSelectionForAgent,
  persistSidepanelChatTargetSelection,
  resolveSidepanelChatTarget,
  type SidepanelChatTargetSelection,
  saveSidepanelChatTargetSelection,
  toLlmProviderConfig,
  watchSidepanelChatTargetSelection,
} from './sidepanel-chat-targets'

const timestamp = 1000

const providers: LlmProviderConfig[] = [
  {
    id: 'browseros',
    type: 'browseros',
    name: 'BrowserOS',
    baseUrl: 'https://api.browseros.com/v1',
    modelId: 'browseros-auto',
    supportsImages: true,
    contextWindow: 200000,
    temperature: 0.2,
    createdAt: timestamp,
    updatedAt: timestamp,
  },
  {
    id: 'anthropic-sonnet',
    type: 'anthropic',
    name: 'Anthropic Sonnet',
    modelId: 'claude-sonnet-4-6',
    apiKey: 'sk-ant',
    supportsImages: true,
    contextWindow: 200000,
    temperature: 0.2,
    createdAt: timestamp,
    updatedAt: timestamp,
  },
]

const localRuntimeProviders: LlmProviderConfig[] = [
  {
    id: 'codex-provider',
    type: 'codex',
    name: 'Codex',
    modelId: 'gpt-5.3-codex',
    supportsImages: false,
    contextWindow: 400000,
    temperature: 0.2,
    createdAt: timestamp,
    updatedAt: timestamp,
  },
  {
    id: 'claude-code-provider',
    type: 'claude-code',
    name: 'Claude Code',
    modelId: 'claude-sonnet-4-6',
    supportsImages: false,
    contextWindow: 200000,
    temperature: 0.2,
    createdAt: timestamp,
    updatedAt: timestamp,
  },
]

const adapters: HarnessAdapterDescriptor[] = [
  {
    id: 'claude',
    name: 'Claude Code',
    defaultModelId: 'haiku',
    defaultReasoningEffort: 'medium',
    modelControl: 'best-effort',
    models: [
      { id: 'sonnet', label: 'Sonnet' },
      { id: 'haiku', label: 'Haiku', recommended: true },
    ],
    reasoningEfforts: [
      { id: 'medium', label: 'Medium', recommended: true },
      { id: 'high', label: 'High' },
    ],
  },
  {
    id: 'codex',
    name: 'Codex',
    defaultModelId: 'gpt-5.5',
    defaultReasoningEffort: 'medium',
    modelControl: 'runtime-supported',
    models: [{ id: 'gpt-5.5', label: 'GPT-5.5', recommended: true }],
    reasoningEfforts: [{ id: 'medium', label: 'Medium', recommended: true }],
  },
]

const agents: HarnessAgent[] = [
  {
    id: 'agent-codex',
    name: 'Review Bot',
    adapter: 'codex',
    modelId: 'gpt-5.5',
    reasoningEffort: 'medium',
    permissionMode: 'approve-all',
    sessionKey: 'agent:agent-codex:main',
    createdAt: timestamp,
    updatedAt: timestamp,
  },
]

describe('buildSidepanelChatTargets', () => {
  it('returns LLM targets plus one ACP target per persisted harness agent', () => {
    const targets = buildSidepanelChatTargets({ providers, adapters, agents })

    expect(targets.map((target) => target.id)).toEqual([
      'browseros',
      'anthropic-sonnet',
      'agent-codex',
    ])
  })

  it('does not emit catalog-only ACP targets without persisted agents', () => {
    const targets = buildSidepanelChatTargets({
      providers,
      adapters,
      agents: [],
    })

    expect(targets.map((target) => target.id)).toEqual([
      'browseros',
      'anthropic-sonnet',
    ])
  })

  it('preserves adapter metadata for created agent targets', () => {
    const targets = buildSidepanelChatTargets({ providers, adapters, agents })
    const codex = targets.find((target) => target.id === 'agent-codex')

    expect(codex).toMatchObject({
      kind: 'acp',
      agentId: 'agent-codex',
      adapter: 'codex',
      adapterName: 'Codex',
      modelId: 'gpt-5.5',
      modelLabel: 'GPT-5.5',
      modelControl: 'runtime-supported',
      recommended: true,
      reasoningEffort: 'medium',
      reasoningEffortLabel: 'Medium',
    })
  })

  it('still returns LLM targets when agents and adapters are unavailable', () => {
    expect(
      buildSidepanelChatTargets({ providers, adapters: [], agents: [] }),
    ).toEqual([
      {
        kind: 'llm',
        id: 'browseros',
        name: 'BrowserOS',
        type: 'browseros',
        provider: providers[0],
      },
      {
        kind: 'llm',
        id: 'anthropic-sonnet',
        name: 'Anthropic Sonnet',
        type: 'anthropic',
        provider: providers[1],
      },
    ])
  })

  it('emits local runtime provider configs as LLM targets so the composer can pick them', () => {
    const targets = buildSidepanelChatTargets({
      providers: [...providers, ...localRuntimeProviders],
      adapters,
      agents,
    })

    expect(targets.map((target) => target.id)).toEqual([
      'browseros',
      'anthropic-sonnet',
      'codex-provider',
      'claude-code-provider',
      'agent-codex',
    ])
  })
})

describe('resolveSidepanelChatTarget', () => {
  it('resolves selected LLM targets back to their provider config', () => {
    const targets = buildSidepanelChatTargets({ providers, adapters, agents })
    const resolved = resolveSidepanelChatTarget({
      targets,
      defaultProviderId: 'browseros',
      selection: { kind: 'llm', id: 'anthropic-sonnet' },
    })

    expect(resolved?.kind).toBe('llm')
    expect(toLlmProviderConfig(resolved)?.modelId).toBe('claude-sonnet-4-6')
  })

  it('falls back to the current default LLM provider when a persisted ACP target is stale', () => {
    const targets = buildSidepanelChatTargets({
      providers,
      adapters,
      agents: [],
    })

    expect(
      resolveSidepanelChatTarget({
        targets,
        defaultProviderId: 'anthropic-sonnet',
        selection: { kind: 'acp', id: 'agent-codex' },
      }),
    ).toMatchObject({
      kind: 'llm',
      id: 'anthropic-sonnet',
    })
  })

  it('falls back when an old catalog-style ACP target id is persisted', () => {
    const targets = buildSidepanelChatTargets({ providers, adapters, agents })

    expect(
      resolveSidepanelChatTarget({
        targets,
        defaultProviderId: 'anthropic-sonnet',
        selection: { kind: 'acp', id: 'acp:codex:gpt-5.5:medium' },
      }),
    ).toMatchObject({
      kind: 'llm',
      id: 'anthropic-sonnet',
    })
  })

  it('resolves a local runtime provider as the chat target when it is the default', () => {
    const targets = buildSidepanelChatTargets({
      providers: [...localRuntimeProviders, ...providers],
      adapters,
      agents: [],
    })

    expect(
      resolveSidepanelChatTarget({
        targets,
        defaultProviderId: 'codex-provider',
      }),
    ).toMatchObject({
      kind: 'llm',
      id: 'codex-provider',
    })
  })
})

describe('persistSidepanelChatTargetSelection', () => {
  it('stores only target identity and does not mutate LLM provider arrays', async () => {
    let savedSelection: SidepanelChatTargetSelection | null = null
    const originalProviders = providers.map((provider) => ({ ...provider }))
    const targets = buildSidepanelChatTargets({ providers, adapters, agents })
    const target = targets.find((candidate) => candidate.id === 'agent-codex')

    await persistSidepanelChatTargetSelection(target, {
      setValue: async (value) => {
        savedSelection = value
      },
    })

    expect(savedSelection as SidepanelChatTargetSelection | null).toEqual({
      kind: 'acp',
      id: 'agent-codex',
    })
    expect(providers).toEqual(originalProviders)
  })

  it('persists null when no target is selected', async () => {
    const store = createSelectionStore({ kind: 'acp', id: 'agent-codex' })

    await persistSidepanelChatTargetSelection(undefined, store)

    expect(await store.getValue()).toBeNull()
  })
})

function createSelectionStore(
  initial: SidepanelChatTargetSelection | null = null,
) {
  let value = initial
  const watchers = new Set<
    (selection: SidepanelChatTargetSelection | null) => void
  >()
  return {
    getValue: async () => value,
    setValue: async (next: SidepanelChatTargetSelection | null) => {
      value = next
      for (const watcher of watchers) watcher(next)
    },
    watch: (
      callback: (selection: SidepanelChatTargetSelection | null) => void,
    ) => {
      watchers.add(callback)
      return () => {
        watchers.delete(callback)
      }
    },
  }
}

describe('saveSidepanelChatTargetSelection', () => {
  it('writes the selection identity to the store', async () => {
    const store = createSelectionStore()

    await saveSidepanelChatTargetSelection(
      { kind: 'acp', id: 'agent-codex' },
      store,
    )

    expect(await store.getValue()).toEqual({ kind: 'acp', id: 'agent-codex' })
  })

  it('clears the stored selection with null', async () => {
    const store = createSelectionStore({ kind: 'llm', id: 'browseros' })

    await saveSidepanelChatTargetSelection(null, store)

    expect(await store.getValue()).toBeNull()
  })
})

describe('clearSidepanelChatTargetSelectionForAgent', () => {
  it('clears the selection pointing at the deleted agent', async () => {
    const store = createSelectionStore({ kind: 'acp', id: 'agent-codex' })

    await clearSidepanelChatTargetSelectionForAgent('agent-codex', store)

    expect(await store.getValue()).toBeNull()
  })

  it('keeps a selection for a different agent', async () => {
    const store = createSelectionStore({ kind: 'acp', id: 'agent-other' })

    await clearSidepanelChatTargetSelectionForAgent('agent-codex', store)

    expect(await store.getValue()).toEqual({ kind: 'acp', id: 'agent-other' })
  })

  it('keeps an LLM selection even when ids collide', async () => {
    const store = createSelectionStore({ kind: 'llm', id: 'agent-codex' })

    await clearSidepanelChatTargetSelectionForAgent('agent-codex', store)

    expect(await store.getValue()).toEqual({ kind: 'llm', id: 'agent-codex' })
  })
})

describe('watchSidepanelChatTargetSelection', () => {
  it('notifies on selection changes until unsubscribed', async () => {
    const store = createSelectionStore()
    const seen: Array<SidepanelChatTargetSelection | null> = []

    const unsubscribe = watchSidepanelChatTargetSelection(
      (selection) => seen.push(selection),
      store,
    )
    await store.setValue({ kind: 'acp', id: 'agent-codex' })
    unsubscribe()
    await store.setValue(null)

    expect(seen).toEqual([{ kind: 'acp', id: 'agent-codex' }])
  })
})
