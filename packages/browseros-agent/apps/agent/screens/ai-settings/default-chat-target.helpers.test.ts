import { describe, expect, it } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import {
  decodeTargetValue,
  encodeTargetValue,
  resolveEffectiveDefaultTarget,
} from './default-chat-target.helpers'

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

const agents = [{ id: 'agent-cc-1' }, { id: 'agent-codex-1' }]

describe('resolveEffectiveDefaultTarget', () => {
  it('returns the acp selection when the agent exists', () => {
    expect(
      resolveEffectiveDefaultTarget({
        providers,
        agents,
        selection: { kind: 'acp', id: 'agent-cc-1' },
        defaultProviderId: 'browseros',
      }),
    ).toEqual({ kind: 'acp', id: 'agent-cc-1' })
  })

  it('falls back to the default provider when the acp selection is stale', () => {
    expect(
      resolveEffectiveDefaultTarget({
        providers,
        agents,
        selection: { kind: 'acp', id: 'agent-deleted' },
        defaultProviderId: 'anthropic-sonnet',
      }),
    ).toEqual({ kind: 'llm', id: 'anthropic-sonnet' })
  })

  it('returns the llm selection when the provider exists', () => {
    expect(
      resolveEffectiveDefaultTarget({
        providers,
        agents,
        selection: { kind: 'llm', id: 'anthropic-sonnet' },
        defaultProviderId: 'browseros',
      }),
    ).toEqual({ kind: 'llm', id: 'anthropic-sonnet' })
  })

  it('falls back to the default provider when the llm selection is stale', () => {
    expect(
      resolveEffectiveDefaultTarget({
        providers,
        agents,
        selection: { kind: 'llm', id: 'provider-deleted' },
        defaultProviderId: 'browseros',
      }),
    ).toEqual({ kind: 'llm', id: 'browseros' })
  })

  it('resolves a null selection to the default provider', () => {
    expect(
      resolveEffectiveDefaultTarget({
        providers,
        agents,
        selection: null,
        defaultProviderId: 'anthropic-sonnet',
      }),
    ).toEqual({ kind: 'llm', id: 'anthropic-sonnet' })
  })

  it('repairs a stale default provider id to the first provider', () => {
    expect(
      resolveEffectiveDefaultTarget({
        providers,
        agents,
        selection: null,
        defaultProviderId: 'provider-deleted',
      }),
    ).toEqual({ kind: 'llm', id: 'browseros' })
  })
})

describe('encodeTargetValue / decodeTargetValue', () => {
  it('round-trips llm and acp selections', () => {
    expect(
      decodeTargetValue(encodeTargetValue({ kind: 'llm', id: 'browseros' })),
    ).toEqual({ kind: 'llm', id: 'browseros' })
    expect(
      decodeTargetValue(encodeTargetValue({ kind: 'acp', id: 'agent-cc-1' })),
    ).toEqual({ kind: 'acp', id: 'agent-cc-1' })
  })

  it('preserves ids that contain separators', () => {
    expect(
      decodeTargetValue(
        encodeTargetValue({ kind: 'acp', id: 'acp:codex:gpt-5.5' }),
      ),
    ).toEqual({ kind: 'acp', id: 'acp:codex:gpt-5.5' })
  })

  it('returns null for malformed values', () => {
    expect(decodeTargetValue('')).toBeNull()
    expect(decodeTargetValue('bogus')).toBeNull()
    expect(decodeTargetValue('http:provider')).toBeNull()
    expect(decodeTargetValue('llm:')).toBeNull()
  })
})
