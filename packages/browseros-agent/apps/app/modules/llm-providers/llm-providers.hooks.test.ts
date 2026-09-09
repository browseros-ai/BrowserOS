import { beforeAll, beforeEach, describe, expect, it, mock } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import {
  resolveDefaultProviderId,
  resolveSelectedProvider,
} from '../../lib/llm-providers/provider-selection'

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
