import { describe, expect, it } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { resolveEffectiveDefaultTarget } from './default-chat-target.helpers'

const timestamp = 1000

const providers: LlmProviderConfig[] = [
  {
    id: 'openai-1',
    type: 'openai',
    name: 'OpenAI',
    baseUrl: 'https://api.openai.com/v1',
    modelId: 'gpt-5',
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

const agents = [{ id: 'agent-cc-1' }, { id: 'agent-codex-1' }]

describe('resolveEffectiveDefaultTarget', () => {
  it('reads the kind off the agent list when the id names an agent', () => {
    expect(
      resolveEffectiveDefaultTarget({
        providers,
        agents,
        defaultTargetId: 'agent-cc-1',
      }),
    ).toEqual({ kind: 'acp', id: 'agent-cc-1' })
  })

  it('reads the kind off the provider list when the id names a provider', () => {
    expect(
      resolveEffectiveDefaultTarget({
        providers,
        agents,
        defaultTargetId: 'anthropic-sonnet',
      }),
    ).toEqual({ kind: 'llm', id: 'anthropic-sonnet' })
  })

  it('falls back to the first provider when the id names nothing', () => {
    expect(
      resolveEffectiveDefaultTarget({
        providers,
        agents,
        defaultTargetId: 'deleted-row',
      }),
    ).toEqual({ kind: 'llm', id: 'openai-1' })
  })

  it('falls back to the first provider when no id is stored', () => {
    expect(
      resolveEffectiveDefaultTarget({
        providers,
        agents,
        defaultTargetId: null,
      }),
    ).toEqual({ kind: 'llm', id: 'openai-1' })
  })

  it('returns null when nothing is configured, so no row reads as selected', () => {
    expect(
      resolveEffectiveDefaultTarget({
        providers: [],
        agents: [],
        defaultTargetId: null,
      }),
    ).toBeNull()
  })

  it('returns null rather than a provider when only agents are gone', () => {
    // An id naming a deleted agent must not silently become an llm provider
    // the user never chose while the list still holds one.
    expect(
      resolveEffectiveDefaultTarget({
        providers: [],
        agents,
        defaultTargetId: 'agent-deleted',
      }),
    ).toBeNull()
  })
})
