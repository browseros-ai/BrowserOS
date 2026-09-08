import { describe, expect, it } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import {
  type ProviderRow,
  removedProviderIds,
  toProviderConfig,
  toProviderConfigs,
  toProviderPayload,
} from './llm-providers.helpers'

function row(overrides: Partial<ProviderRow> = {}): ProviderRow {
  return {
    id: 'provider-1',
    type: 'openai',
    name: 'My OpenAI',
    kind: 'llm',
    baseUrl: null,
    modelId: 'gpt-5.5',
    supportsImages: true,
    contextWindow: 200000,
    temperature: 0.2,
    hasApiKey: false,
    hasAccessKeyId: false,
    hasSecretAccessKey: false,
    hasSessionToken: false,
    resourceName: null,
    region: null,
    reasoningEffort: null,
    reasoningSummary: null,
    createdAt: 10,
    updatedAt: 20,
    ...overrides,
  }
}

function config(overrides: Partial<LlmProviderConfig> = {}) {
  return {
    id: 'provider-1',
    type: 'openai',
    name: 'My OpenAI',
    modelId: 'gpt-5.5',
    supportsImages: true,
    contextWindow: 200000,
    temperature: 0.2,
    createdAt: 10,
    updatedAt: 20,
    ...overrides,
  } as LlmProviderConfig
}

describe('toProviderConfig', () => {
  it('preserves headers through the settings read and write payloads', () => {
    const headers = { 'x-opencode-session': '{{conversationId}}' }
    const converted = toProviderConfig(row({ headers }))
    expect(converted?.headers).toEqual(headers)
    expect(toProviderPayload(converted as LlmProviderConfig).headers).toEqual(
      headers,
    )
  })

  // The column is nullable but the config type uses undefined, and the two are
  // not interchangeable to anything doing `'apiKey' in provider`.
  it('turns absent columns into undefined rather than null', () => {
    const converted = toProviderConfig(row())

    expect(converted?.baseUrl).toBeUndefined()
    expect(converted?.reasoningSummary).toBeUndefined()
  })

  // Credentials never leave the server now, so the row reports only whether
  // one is set and the config carries that instead.
  it('carries the credential flags, not the credentials', () => {
    const converted = toProviderConfig(
      row({ hasApiKey: true, region: 'us-east-1' }),
    )

    expect(converted).toMatchObject({ hasApiKey: true, region: 'us-east-1' })
    expect(converted?.apiKey).toBeUndefined()
  })

  // Coding agents share this table and this endpoint. Filtering on kind is
  // what excludes them; the unknown-type guard below used to do it by accident.
  it('rejects a coding agent', () => {
    expect(
      toProviderConfig(row({ kind: 'acp', type: 'claude', modelId: null })),
    ).toBeNull()
  })

  // The row survives in the database and comes back on upgrade. Showing it
  // would push an unknown key through the icon map and template lookup, both
  // keyed by the provider union.
  it('rejects a type this build does not know', () => {
    expect(toProviderConfig(row({ type: 'some-future-provider' }))).toBeNull()
  })

  it('keeps a recognised reasoning summary and drops an unrecognised one', () => {
    expect(
      toProviderConfig(row({ reasoningSummary: 'concise' }))?.reasoningSummary,
    ).toBe('concise')
    expect(
      toProviderConfig(row({ reasoningSummary: 'verbose' }))?.reasoningSummary,
    ).toBeUndefined()
  })
})

describe('toProviderConfigs', () => {
  it('drops unusable rows without losing the rest', () => {
    const converted = toProviderConfigs([
      row(),
      row({ id: 'provider-2', type: 'some-future-provider' }),
    ])

    expect(converted.map((provider) => provider.id)).toEqual(['provider-1'])
  })
})

describe('toProviderPayload', () => {
  // id travels in the path, so sending it in the body too would let the two
  // disagree.
  it('leaves the id out of the body', () => {
    expect('id' in toProviderPayload(config())).toBe(false)
  })

  it('preserves the creation time so a save does not reset it', () => {
    expect(toProviderPayload(config()).createdAt).toBe(10)
  })
})

describe('removedProviderIds', () => {
  it('names the ids that a save displaced', () => {
    const before = [config(), config({ id: 'provider-2' })]
    const after = [config()]

    expect(removedProviderIds(before, after)).toEqual(['provider-2'])
  })

  it('names nothing when the save displaced nothing', () => {
    const before = [config()]
    expect(removedProviderIds(before, before)).toEqual([])
  })
})
