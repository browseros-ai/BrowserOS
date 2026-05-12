import { describe, expect, it } from 'bun:test'
import type { Provider } from '@/components/chat/chatComponentTypes'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { toProviderOption } from '../useChatSessionRequest'
import type {
  HarnessAdapterDescriptor,
  HarnessAgent,
} from '@/entrypoints/app/agents/agent-harness-types'
import {
  buildSidepanelChatTargets,
  type SidepanelChatTarget,
} from '../sidepanel-chat-targets'

const timestamp = 1000

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

function makeLlmProvider(
  overrides: Partial<LlmProviderConfig> = {},
): LlmProviderConfig {
  return {
    id: 'test-llm',
    type: 'openai',
    name: 'Test OpenAI',
    baseUrl: 'https://api.openai.com/v1',
    modelId: 'gpt-4o',
    apiKey: 'sk-test',
    supportsImages: true,
    contextWindow: 128000,
    temperature: 0.2,
    createdAt: timestamp,
    updatedAt: timestamp,
    ...overrides,
  }
}

function makeAcpTarget(
  overrides: Partial<Extract<SidepanelChatTarget, { kind: 'acp' }>> = {},
): SidepanelChatTarget {
  return {
    kind: 'acp',
    id: 'agent-test',
    name: 'Test Agent',
    type: 'acp',
    agentId: 'agent-test',
    adapter: 'claude',
    adapterName: 'Claude Code',
    modelId: 'sonnet',
    modelLabel: 'Sonnet',
    modelControl: 'best-effort',
    reasoningEffort: 'medium',
    ...overrides,
  }
}

function makeLlmTarget(providerOverrides: Partial<LlmProviderConfig> = {}): SidepanelChatTarget {
  const provider = makeLlmProvider(providerOverrides)
  return {
    kind: 'llm',
    id: provider.id,
    name: provider.name,
    type: provider.type,
    provider,
  }
}

// ═══════════════════════════════════════════════════════════════════════════
// toProviderOption — LLM targets
// ═══════════════════════════════════════════════════════════════════════════

describe('toProviderOption — LLM target', () => {
  it('passes modelId from provider config', () => {
    const target = makeLlmTarget({ modelId: 'gpt-4o' })
    const option = toProviderOption(target)
    expect(option.modelId).toBe('gpt-4o')
  })

  it('passes contextWindow from provider config', () => {
    const target = makeLlmTarget({ contextWindow: 200000 })
    const option = toProviderOption(target)
    expect(option.contextWindow).toBe(200000)
  })

  it('passes both modelId and contextWindow together', () => {
    const target = makeLlmTarget({ modelId: 'claude-opus-4-6', contextWindow: 200000 })
    const option = toProviderOption(target)
    expect(option.modelId).toBe('claude-opus-4-6')
    expect(option.contextWindow).toBe(200000)
  })

  it('handles provider with contextWindow = 0', () => {
    const target = makeLlmTarget({ contextWindow: 0 })
    const option = toProviderOption(target)
    expect(option.contextWindow).toBe(0)
  })

  it('handles provider with undefined modelId', () => {
    const target = makeLlmTarget({ modelId: undefined as any })
    const option = toProviderOption(target)
    expect(option.modelId).toBeUndefined()
  })

  it('does NOT set acp-specific fields', () => {
    const target = makeLlmTarget()
    const option = toProviderOption(target)
    expect(option.agentId).toBeUndefined()
    expect(option.adapterName).toBeUndefined()
    expect(option.modelLabel).toBeUndefined()
    expect(option.modelControl).toBeUndefined()
  })
})

// ═══════════════════════════════════════════════════════════════════════════
// toProviderOption — ACP targets
// ═══════════════════════════════════════════════════════════════════════════

describe('toProviderOption — ACP target', () => {
  it('passes modelId from target (not provider)', () => {
    const target = makeAcpTarget({ modelId: 'haiku' })
    const option = toProviderOption(target)
    expect(option.modelId).toBe('haiku')
  })

  it('does NOT pass contextWindow for ACP targets', () => {
    const target = makeAcpTarget()
    const option = toProviderOption(target)
    expect(option.contextWindow).toBeUndefined()
  })

  it('passes ACP-specific fields', () => {
    const target = makeAcpTarget({
      agentId: 'agent-123',
      adapterName: 'Codex',
      modelLabel: 'GPT-5.5',
      modelControl: 'runtime-supported',
    })
    const option = toProviderOption(target)
    expect(option.agentId).toBe('agent-123')
    expect(option.adapterName).toBe('Codex')
    expect(option.modelLabel).toBe('GPT-5.5')
    expect(option.modelControl).toBe('runtime-supported')
  })

  it('handles ACP target with empty modelId', () => {
    const target = makeAcpTarget({ modelId: '' })
    const option = toProviderOption(target)
    expect(option.modelId).toBe('')
  })
})

// ═══════════════════════════════════════════════════════════════════════════
// toProviderOption — full integration via buildSidepanelChatTargets
// ═══════════════════════════════════════════════════════════════════════════

describe('toProviderOption — integration with buildSidepanelChatTargets', () => {
  const providers: LlmProviderConfig[] = [
    makeLlmProvider({
      id: 'browseros',
      type: 'browseros',
      name: 'BrowserOS',
      modelId: 'browseros-auto',
      contextWindow: 200000,
      baseUrl: 'https://api.browseros.com/v1',
    }),
    makeLlmProvider({
      id: 'anthropic-sonnet',
      type: 'anthropic',
      name: 'Anthropic Sonnet',
      modelId: 'claude-sonnet-4-6',
      contextWindow: 200000,
      apiKey: 'sk-ant',
    }),
  ]

  const adapters: HarnessAdapterDescriptor[] = [
    {
      id: 'claude',
      name: 'Claude Code',
      defaultModelId: 'sonnet',
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
  ]

  const agents: HarnessAgent[] = [
    {
      id: 'agent-claude',
      name: 'Review Bot',
      adapter: 'claude',
      modelId: 'sonnet',
      reasoningEffort: 'medium',
      permissionMode: 'approve-all',
      sessionKey: 'agent:agent-claude:main',
      createdAt: timestamp,
      updatedAt: timestamp,
    },
  ]

  it('LLM target gets modelId and contextWindow from provider', () => {
    const targets = buildSidepanelChatTargets({ providers, adapters, agents })
    const llmTarget = targets.find((t) => t.kind === 'llm' && t.id === 'anthropic-sonnet')!
    const option = toProviderOption(llmTarget)

    expect(option.modelId).toBe('claude-sonnet-4-6')
    expect(option.contextWindow).toBe(200000)
  })

  it('ACP target gets modelId but NOT contextWindow', () => {
    const targets = buildSidepanelChatTargets({ providers, adapters, agents })
    const acpTarget = targets.find((t) => t.kind === 'acp')!
    const option = toProviderOption(acpTarget)

    expect(option.modelId).toBe('sonnet')
    expect(option.contextWindow).toBeUndefined()
  })

  it('browseros built-in provider gets modelId and contextWindow', () => {
    const targets = buildSidepanelChatTargets({ providers, adapters, agents })
    const browserOSTarget = targets.find((t) => t.id === 'browseros')!
    const option = toProviderOption(browserOSTarget)

    expect(option.modelId).toBe('browseros-auto')
    expect(option.contextWindow).toBe(200000)
  })
})

// ═══════════════════════════════════════════════════════════════════════════
// Badge rendering logic (pure function tests — no DOM)
// ═══════════════════════════════════════════════════════════════════════════

describe('Badge rendering conditions (pure logic)', () => {
  /**
   * Simulates the ChatHeader badge rendering logic:
   * - LLM (kind !== 'acp'): shows model badge if modelId is truthy
   * - ACP (kind === 'acp'): shows model label if modelLabel is truthy
   * - Context size shown only for LLM with contextWindow > 0
   */

  function shouldShowModelBadge(provider: Provider): boolean {
    return !!(provider.modelId && provider.kind !== 'acp')
  }

  function shouldShowContextBadge(provider: Provider): boolean {
    return !!(
      provider.modelId &&
      provider.kind !== 'acp' &&
      provider.contextWindow &&
      provider.contextWindow > 0
    )
  }

  function shouldShowAcpLabel(provider: Provider): boolean {
    return !!(provider.kind === 'acp' && provider.modelLabel)
  }

  // ── LLM providers ────────────────────────────────────────────────────

  it('LLM with modelId and contextWindow shows both badges', () => {
    const provider: Provider = {
      id: 'test', name: 'Test', type: 'openai', kind: 'llm',
      modelId: 'gpt-4o', contextWindow: 128000,
    }
    expect(shouldShowModelBadge(provider)).toBe(true)
    expect(shouldShowContextBadge(provider)).toBe(true)
    expect(shouldShowAcpLabel(provider)).toBe(false)
  })

  it('LLM with modelId but no contextWindow shows model badge only', () => {
    const provider: Provider = {
      id: 'test', name: 'Test', type: 'openai', kind: 'llm',
      modelId: 'gpt-4o',
    }
    expect(shouldShowModelBadge(provider)).toBe(true)
    expect(shouldShowContextBadge(provider)).toBe(false)
  })

  it('LLM with contextWindow=0 shows model badge but NOT context badge', () => {
    const provider: Provider = {
      id: 'test', name: 'Test', type: 'openai', kind: 'llm',
      modelId: 'gpt-4o', contextWindow: 0,
    }
    expect(shouldShowModelBadge(provider)).toBe(true)
    expect(shouldShowContextBadge(provider)).toBe(false)
  })

  it('LLM without modelId shows no badges', () => {
    const provider: Provider = {
      id: 'test', name: 'Test', type: 'openai', kind: 'llm',
    }
    expect(shouldShowModelBadge(provider)).toBe(false)
    expect(shouldShowContextBadge(provider)).toBe(false)
    expect(shouldShowAcpLabel(provider)).toBe(false)
  })

  it('LLM with empty modelId string shows no badges', () => {
    const provider: Provider = {
      id: 'test', name: 'Test', type: 'openai', kind: 'llm',
      modelId: '',
    }
    expect(shouldShowModelBadge(provider)).toBe(false)
  })

  // ── ACP providers ────────────────────────────────────────────────────

  it('ACP with modelLabel shows ACP label badge only', () => {
    const provider: Provider = {
      id: 'agent-1', name: 'Bot', type: 'acp', kind: 'acp',
      modelLabel: 'Sonnet', modelId: 'sonnet',
    }
    expect(shouldShowModelBadge(provider)).toBe(false) // kind === 'acp'
    expect(shouldShowContextBadge(provider)).toBe(false)
    expect(shouldShowAcpLabel(provider)).toBe(true)
  })

  it('ACP without modelLabel shows nothing', () => {
    const provider: Provider = {
      id: 'agent-1', name: 'Bot', type: 'acp', kind: 'acp',
      modelId: 'sonnet',
    }
    expect(shouldShowModelBadge(provider)).toBe(false)
    expect(shouldShowContextBadge(provider)).toBe(false)
    expect(shouldShowAcpLabel(provider)).toBe(false)
  })

  // ── BrowserOS built-in ───────────────────────────────────────────────

  it('BrowserOS provider shows model badge (browseros-auto 200K)', () => {
    const provider: Provider = {
      id: 'browseros', name: 'BrowserOS', type: 'browseros', kind: 'llm',
      modelId: 'browseros-auto', contextWindow: 200000,
    }
    expect(shouldShowModelBadge(provider)).toBe(true)
    expect(shouldShowContextBadge(provider)).toBe(true)
  })

  // ── Negative contextWindow ───────────────────────────────────────────

  it('LLM with negative contextWindow shows model badge but NOT context badge', () => {
    const provider: Provider = {
      id: 'test', name: 'Test', type: 'openai', kind: 'llm',
      modelId: 'gpt-4o', contextWindow: -1,
    }
    expect(shouldShowModelBadge(provider)).toBe(true)
    expect(shouldShowContextBadge(provider)).toBe(false)
  })
})
