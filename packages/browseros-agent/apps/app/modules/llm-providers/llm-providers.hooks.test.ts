import { beforeAll, beforeEach, describe, expect, it, mock } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import {
  resolveDefaultProviderId,
  resolveSelectedProvider,
} from '../../lib/llm-providers/provider-selection'
import { planProviderSave } from './llm-providers.helpers'

const storageValues = new Map<string, unknown>()
const putDefaultProviderCalls: string[] = []

mock.module('./llm-providers.api', () => ({
  fetchProviders: async () => [],
  fetchDefaultProviderId: async () => null,
  putProvider: async () => undefined,
  deleteProvider: async () => undefined,
  putDefaultProvider: async (providerId: string) => {
    putDefaultProviderCalls.push(providerId)
  },
}))

mock.module('@wxt-dev/storage', () => ({
  storage: {
    defineItem: <T>(key: string, options?: { fallback?: T }) => ({
      getValue: async () =>
        storageValues.has(key) ? storageValues.get(key) : options?.fallback,
      setValue: async (value: T) => {
        storageValues.set(key, value)
      },
      watch: () => () => {},
    }),
  },
}))

mock.module('@/lib/auth/sessionStorage', () => ({
  sessionStorage: {
    getValue: async () => null,
  },
}))

const browserOSAdapter = {
  getBrowserosVersion: async () => null,
  getPref: async (name: string) =>
    new Promise<{ value?: unknown }>((resolve) => {
      const getPref = globalThis.chrome?.browserOS?.getPref
      if (!getPref) {
        resolve({ value: null })
        return
      }
      getPref(name, resolve)
    }),
  setPref: async () => {},
}

const MockBrowserOSAdapter = {
  getInstance: () => browserOSAdapter,
}

const createBrowserOSProvider = () => ({
  id: 'browseros',
  type: 'browseros',
  name: 'BrowserOS',
  modelId: 'browseros-auto',
  supportsImages: true,
  contextWindow: 200000,
  temperature: 0.2,
  createdAt: 0,
  updatedAt: 0,
})

mock.module('@/lib/browseros/adapter', () => ({
  BrowserOSAdapter: MockBrowserOSAdapter,
  getBrowserOSAdapter: () => browserOSAdapter,
}))

mock.module('@/lib/browseros/prefs', () => ({
  BROWSEROS_PREFS: {
    PROVIDERS: 'browseros.providers',
    MCP_PORT: 'browseros.server.mcp_port',
  },
}))

mock.module('../../lib/llm-providers/storage', () => ({
  DEFAULT_PROVIDER_ID: 'browseros',
  createDefaultBrowserOSProvider: createBrowserOSProvider,
  createDefaultProvidersConfig: () => [createBrowserOSProvider()],
  defaultProviderIdStorage: {
    getValue: async () => storageValues.get('local:default-provider-id'),
    setValue: async (value: string) => {
      storageValues.set('local:default-provider-id', value)
    },
    watch: () => () => {},
  },
  loadProviders: async () =>
    (storageValues.get('local:llm-providers') as LlmProviderConfig[]) ?? [],
  providersStorage: {
    getValue: async () =>
      (storageValues.get('local:llm-providers') as LlmProviderConfig[]) ?? [],
    setValue: async (value: LlmProviderConfig[]) => {
      storageValues.set('local:llm-providers', value)
    },
    watch: () => () => {},
  },
}))

const timestamp = 1000

function providerConfig(
  overrides: Partial<LlmProviderConfig> & Pick<LlmProviderConfig, 'id'>,
): LlmProviderConfig {
  return {
    type: 'openai',
    name: 'OpenAI',
    modelId: 'gpt-5',
    supportsImages: true,
    contextWindow: 400000,
    temperature: 0.2,
    createdAt: timestamp,
    updatedAt: timestamp,
    ...overrides,
  }
}

const providers: LlmProviderConfig[] = [
  {
    id: 'browseros',
    type: 'browseros',
    name: 'BrowserOS',
    modelId: 'browseros-auto',
    supportsImages: true,
    contextWindow: 200000,
    temperature: 0.2,
    createdAt: timestamp,
    updatedAt: timestamp,
  },
  {
    id: 'anthropic-provider',
    type: 'anthropic',
    name: 'Anthropic',
    modelId: 'claude-sonnet-4-6',
    supportsImages: false,
    contextWindow: 200000,
    temperature: 0.2,
    createdAt: timestamp,
    updatedAt: timestamp,
  },
]

let persistDefaultProviderId: (providerId: string) => Promise<void>

beforeAll(async () => {
  ;({ persistDefaultProviderId } = await import('./llm-providers.hooks'))
})

beforeEach(() => {
  storageValues.clear()
})

describe('resolveSelectedProvider', () => {
  it('selects a configured provider by the persisted default id', () => {
    expect(resolveSelectedProvider(providers, 'anthropic-provider')).toEqual(
      providers[1],
    )
  })
})

describe('persistDefaultProviderId', () => {
  // The selection moved to the server when the two provider tables merged, so
  // it can name a coding agent as readily as an llm provider. It used to be an
  // extension storage write, which is why it could only ever name the latter.
  it('sends the provider id to the server', async () => {
    await persistDefaultProviderId('anthropic-provider')

    expect(putDefaultProviderCalls).toEqual(['anthropic-provider'])
  })
})

describe('planProviderSave', () => {
  // These are OAuth providers, where the credential is one grant per account,
  // so a second copy is always a duplicate of the first rather than a choice.
  it('replaces an existing OAuth provider by type while preserving its id', () => {
    const existing = providerConfig({
      id: 'chatgpt-pro-existing',
      type: 'chatgpt-pro',
      name: 'Old ChatGPT',
      modelId: 'gpt-5.1-codex',
      createdAt: 1111,
      updatedAt: 1111,
    })
    const incoming = providerConfig({
      id: 'chatgpt-pro-9999',
      type: 'chatgpt-pro',
      name: 'ChatGPT',
      modelId: 'gpt-5.5',
      contextWindow: 1050000,
    })

    const { saved, removedIds } = planProviderSave(
      [providers[0], existing],
      incoming,
      2222,
    )

    expect(saved).toMatchObject({
      id: 'chatgpt-pro-existing',
      type: 'chatgpt-pro',
      name: 'ChatGPT',
      modelId: 'gpt-5.5',
      contextWindow: 1050000,
      createdAt: 1111,
      updatedAt: 2222,
    })
    expect(removedIds).toEqual([])
  })

  // Writing the whole list used to drop these implicitly. Over HTTP each one
  // needs its own DELETE, so the plan has to name them.
  it('names the extra same-type OAuth rows for deletion', () => {
    const first = providerConfig({
      id: 'chatgpt-pro-first',
      type: 'chatgpt-pro',
      name: 'First ChatGPT',
    })
    const second = providerConfig({
      id: 'chatgpt-pro-second',
      type: 'chatgpt-pro',
      name: 'Second ChatGPT',
    })
    const incoming = providerConfig({
      id: 'chatgpt-pro-new',
      type: 'chatgpt-pro',
      name: 'Fresh ChatGPT',
    })

    const { saved, removedIds } = planProviderSave(
      [providers[0], first, second],
      incoming,
    )

    expect(saved).toMatchObject({
      id: 'chatgpt-pro-first',
      name: 'Fresh ChatGPT',
    })
    expect(removedIds).toEqual(['chatgpt-pro-second'])
  })

  it('allows multiple non-OAuth providers of the same type', () => {
    const first = providerConfig({ id: 'openai-first', name: 'OpenAI 1' })
    const second = providerConfig({ id: 'openai-second', name: 'OpenAI 2' })

    const { saved, removedIds } = planProviderSave([first], second, 2222)

    expect(saved.id).toBe('openai-second')
    expect(removedIds).toEqual([])
  })

  it('stamps a creation time on a provider that is new', () => {
    const { saved } = planProviderSave(
      [],
      providerConfig({ id: 'openai-new' }),
      3333,
    )

    expect(saved.createdAt).toBe(3333)
    expect(saved.updatedAt).toBe(3333)
  })

  it('keeps the original creation time when updating in place', () => {
    const existing = providerConfig({ id: 'openai-1', createdAt: 1111 })
    const { saved } = planProviderSave(
      [existing],
      { ...existing, name: 'Renamed' },
      4444,
    )

    expect(saved.createdAt).toBe(1111)
    expect(saved.updatedAt).toBe(4444)
  })
})

describe('resolveDefaultProviderId', () => {
  it('keeps a provider id when it exists', () => {
    expect(resolveDefaultProviderId(providers, 'anthropic-provider')).toBe(
      'anthropic-provider',
    )
  })

  it('repairs a stale default id to the first configured provider', () => {
    expect(resolveDefaultProviderId(providers, 'missing-provider')).toBe(
      'browseros',
    )
  })
})
