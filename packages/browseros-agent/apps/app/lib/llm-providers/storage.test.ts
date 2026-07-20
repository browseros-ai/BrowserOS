import { beforeAll, beforeEach, describe, expect, it, mock } from 'bun:test'
import {
  migrateLlmProvidersToV3,
  normalizeProviderNames,
} from './provider-name-normalization'
import { resolveDefaultProviderId } from './provider-selection'
import type { LlmProviderConfig } from './types'

const storageValues = new Map<string, unknown>()
const migratedStorageKeys = new Set<string>()

interface MockStorageOptions<T> {
  fallback?: T
  migrations?: Record<number, (value: T | null) => T | null>
}

mock.module('@wxt-dev/storage', () => ({
  storage: {
    defineItem: <T>(key: string, options?: MockStorageOptions<T>) => ({
      getValue: async () => {
        let value = storageValues.has(key)
          ? (storageValues.get(key) as T | null)
          : (options?.fallback ?? null)
        if (!migratedStorageKeys.has(key) && options?.migrations) {
          const versions = Object.keys(options.migrations)
            .map(Number)
            .sort((a, b) => a - b)
          for (const version of versions) {
            value = options.migrations[version](value)
          }
          storageValues.set(key, value)
          migratedStorageKeys.add(key)
        }
        return value
      },
      setValue: async (value: T) => {
        storageValues.set(key, value)
        migratedStorageKeys.add(key)
      },
      watch: () => () => {},
    }),
  },
}))

mock.module('@/lib/auth/sessionStorage', () => ({
  sessionStorage: { getValue: async () => null },
}))

mock.module('@/lib/browseros/adapter', () => ({
  getBrowserOSAdapter: () => ({ setPref: async () => {} }),
}))

mock.module('@/lib/browseros/prefs', () => ({
  BROWSEROS_PREFS: { PROVIDERS: 'browseros.providers' },
}))

mock.module('./uploadLlmProvidersToGraphql', () => ({
  uploadLlmProvidersToGraphql: async () => {},
}))

let loadProviders: typeof import('./storage').loadProviders
let providersStorage: typeof import('./storage').providersStorage

beforeAll(async () => {
  ;({ loadProviders, providersStorage } = await import('./storage'))
})

beforeEach(() => {
  storageValues.clear()
  migratedStorageKeys.clear()
})

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
    createdAt: 1000,
    updatedAt: 1000,
    ...overrides,
  }
}

describe('normalizeProviderNames', () => {
  it('normalizes legacy ChatGPT display names', () => {
    const providers = normalizeProviderNames([
      providerConfig({
        id: 'chatgpt-pro-1',
        type: 'chatgpt-pro',
        name: 'ChatGPT Plus/Pro',
      }),
      providerConfig({
        id: 'chatgpt-pro-2',
        type: 'chatgpt-pro',
        name: 'ChatGPT Plus/Pro (user@example.com)',
      }),
    ])

    expect(providers.map((provider) => provider.name)).toEqual([
      'ChatGPT',
      'ChatGPT',
    ])
  })

  it('preserves custom ChatGPT provider names', () => {
    const providers = normalizeProviderNames([
      providerConfig({
        id: 'chatgpt-pro-custom',
        type: 'chatgpt-pro',
        name: 'Work ChatGPT',
      }),
      providerConfig({
        id: 'chatgpt-pro-parenthetical-custom',
        type: 'chatgpt-pro',
        name: 'ChatGPT Plus/Pro (Work)',
      }),
    ])

    expect(providers.map((provider) => provider.name)).toEqual([
      'Work ChatGPT',
      'ChatGPT Plus/Pro (Work)',
    ])
  })
})

describe('migrateLlmProvidersToV3', () => {
  it('migrates legacy ChatGPT display names for direct storage reads', () => {
    const providers = migrateLlmProvidersToV3([
      providerConfig({
        id: 'chatgpt-pro-1',
        type: 'chatgpt-pro',
        name: 'ChatGPT Plus/Pro (user@example.com)',
      }),
    ])

    expect(providers?.map((provider) => provider.name)).toEqual(['ChatGPT'])
  })
})

describe('loadProviders', () => {
  it('migrates an old Remote Hermes config before direct storage reads', async () => {
    const openAI = providerConfig({ id: 'openai-1' })
    const remoteHermes = providerConfig({
      id: 'remote-hermes-1',
      type: 'remote-hermes' as LlmProviderConfig['type'],
      name: 'Remote Hermes',
    })
    storageValues.set('local:llm-providers', [remoteHermes, openAI])

    const providers = await providersStorage.getValue()

    expect(providers).toEqual([openAI])
    expect(resolveDefaultProviderId(providers ?? [], remoteHermes.id)).toBe(
      openAI.id,
    )
  })

  it('drops an old Remote Hermes config and falls back from its default id', async () => {
    const openAI = providerConfig({ id: 'openai-1' })
    const remoteHermes = providerConfig({
      id: 'remote-hermes-1',
      type: 'remote-hermes' as LlmProviderConfig['type'],
      name: 'Remote Hermes',
    })
    await providersStorage.setValue([remoteHermes, openAI])

    const providers = await loadProviders()

    expect(providers).toEqual([openAI])
    expect(await providersStorage.getValue()).toEqual([openAI])
    expect(resolveDefaultProviderId(providers, remoteHermes.id)).toBe(openAI.id)
  })
})
