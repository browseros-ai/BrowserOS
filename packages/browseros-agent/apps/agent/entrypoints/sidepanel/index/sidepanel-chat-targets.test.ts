import { describe, expect, it } from 'bun:test'
import type { HarnessAdapterDescriptor } from '@/entrypoints/app/agents/agent-harness-types'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import {
  buildSidepanelChatTargets,
  persistSidepanelChatTargetSelection,
  resolveSidepanelChatTarget,
  type SidepanelChatTargetSelection,
  toLlmProviderConfig,
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

describe('buildSidepanelChatTargets', () => {
  it('returns LLM targets plus one ACP target per adapter model', () => {
    const targets = buildSidepanelChatTargets({ providers, adapters })

    expect(targets.map((target) => target.id)).toEqual([
      'browseros',
      'anthropic-sonnet',
      'acp:claude:sonnet:medium',
      'acp:claude:haiku:medium',
      'acp:codex:gpt-5.5:medium',
    ])
  })

  it('preserves ACP model-control and recommendation metadata', () => {
    const targets = buildSidepanelChatTargets({ providers, adapters })
    const haiku = targets.find(
      (target) => target.id === 'acp:claude:haiku:medium',
    )

    expect(haiku).toMatchObject({
      kind: 'acp',
      adapter: 'claude',
      modelId: 'haiku',
      modelControl: 'best-effort',
      recommended: true,
      reasoningEffort: 'medium',
    })
  })

  it('still returns LLM targets when ACP adapters are unavailable', () => {
    expect(buildSidepanelChatTargets({ providers, adapters: [] })).toEqual([
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
})

describe('resolveSidepanelChatTarget', () => {
  it('resolves selected LLM targets back to their provider config', () => {
    const targets = buildSidepanelChatTargets({ providers, adapters })
    const resolved = resolveSidepanelChatTarget({
      targets,
      defaultProviderId: 'browseros',
      selection: { kind: 'llm', id: 'anthropic-sonnet' },
    })

    expect(resolved?.kind).toBe('llm')
    expect(toLlmProviderConfig(resolved)?.modelId).toBe('claude-sonnet-4-6')
  })

  it('falls back to the current default LLM provider when a persisted ACP target is stale', () => {
    const targets = buildSidepanelChatTargets({ providers, adapters: [] })

    expect(
      resolveSidepanelChatTarget({
        targets,
        defaultProviderId: 'anthropic-sonnet',
        selection: { kind: 'acp', id: 'acp:claude:haiku:medium' },
      }),
    ).toMatchObject({
      kind: 'llm',
      id: 'anthropic-sonnet',
    })
  })
})

describe('persistSidepanelChatTargetSelection', () => {
  it('stores only target identity and does not mutate LLM provider arrays', async () => {
    let savedSelection: SidepanelChatTargetSelection | null = null
    const originalProviders = providers.map((provider) => ({ ...provider }))
    const targets = buildSidepanelChatTargets({ providers, adapters })
    const target = targets.find(
      (candidate) => candidate.id === 'acp:codex:gpt-5.5:medium',
    )

    await persistSidepanelChatTargetSelection(target, {
      setValue: async (value) => {
        savedSelection = value
      },
    })

    expect(savedSelection as SidepanelChatTargetSelection | null).toEqual({
      kind: 'acp',
      id: 'acp:codex:gpt-5.5:medium',
    })
    expect(providers).toEqual(originalProviders)
  })
})
