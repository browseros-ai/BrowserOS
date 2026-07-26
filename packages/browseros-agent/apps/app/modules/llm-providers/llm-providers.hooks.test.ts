import { beforeAll, beforeEach, describe, expect, it, mock } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import {
  resolveDefaultProviderId,
  resolveSelectedProvider,
} from '../../lib/llm-providers/provider-selection'

const storageValues = new Map<string, unknown>()

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

mock.module('@/lib/llm-providers/uploadLlmProvidersToGraphql', () => ({
  uploadLlmProvidersToGraphql: async () => {},
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

let persistDefaultProviderId: (providerId: string) => Promise<void>
let upsertProviderConfig: typeof import('./llm-providers.hooks').upsertProviderConfig

beforeAll(async () => {
  ;({ persistDefaultProviderId, upsertProviderConfig } = await import(
    './llm-providers.hooks'
  ))
})

beforeEach(() => {
  storageValues.clear()
})

describe('resolveSelectedProvider', () => {
  it('selects a Codex provider config by the persisted default id', () => {
    expect(resolveSelectedProvider(providers, 'codex-provider')).toEqual(
      providers[1],
    )
  })

  it('selects a Claude Code provider config by the persisted default id', () => {
    expect(resolveSelectedProvider(providers, 'claude-code-provider')).toEqual(
      providers[2],
    )
  })
})

describe('persistDefaultProviderId', () => {
  it('writes a Codex provider id to default-provider storage', async () => {
    await persistDefaultProviderId('codex-provider')

    expect(storageValues.get('local:default-provider-id')).toBe(
      'codex-provider',
    )
  })

  it('writes a Claude Code provider id to default-provider storage', async () => {
    await persistDefaultProviderId('claude-code-provider')

    expect(storageValues.get('local:default-provider-id')).toBe(
      'claude-code-provider',
    )
  })
})

describe('upsertProviderConfig', () => {
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

    const result = upsertProviderConfig(
      [providers[0], existing],
      incoming,
      2222,
    )

    expect(result).toHaveLength(2)
    expect(result[1]).toMatchObject({
      id: 'chatgpt-pro-existing',
      type: 'chatgpt-pro',
      name: 'ChatGPT',
      modelId: 'gpt-5.5',
      contextWindow: 1050000,
      createdAt: 1111,
      updatedAt: 2222,
    })
  })

  it('removes extra same-type OAuth rows on save', () => {
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

    const result = upsertProviderConfig([providers[0], first, second], incoming)

    expect(
      result.filter((provider) => provider.type === 'chatgpt-pro'),
    ).toEqual([
      expect.objectContaining({
        id: 'chatgpt-pro-first',
        name: 'Fresh ChatGPT',
      }),
    ])
  })

  it('allows multiple non-OAuth providers of the same type', () => {
    const first = providerConfig({ id: 'openai-first', name: 'OpenAI 1' })
    const second = providerConfig({ id: 'openai-second', name: 'OpenAI 2' })

    const result = upsertProviderConfig([first], second, 2222)

    expect(result.map((provider) => provider.id)).toEqual([
      'openai-first',
      'openai-second',
    ])
  })
})

describe('resolveDefaultProviderId', () => {
  it('keeps a Codex provider id when it exists', () => {
    expect(resolveDefaultProviderId(providers, 'codex-provider')).toBe(
      'codex-provider',
    )
  })

  it('keeps a Claude Code provider id when it exists', () => {
    expect(resolveDefaultProviderId(providers, 'claude-code-provider')).toBe(
      'claude-code-provider',
    )
  })

  it('repairs a stale default id to the first configured provider', () => {
    expect(resolveDefaultProviderId(providers, 'missing-provider')).toBe(
      'browseros',
    )
  })
})
