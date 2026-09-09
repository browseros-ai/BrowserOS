import { afterEach, beforeEach, expect, it, mock } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'

mock.module('@/modules/browseros/agent-server-url.helpers', () => ({
  resolveAgentServerUrlWithRetry: async () => 'http://localhost:9000',
}))
mock.module('./llm-providers.revision', () => ({
  bumpProviderRevision: async () => {},
}))
mock.module('@/lib/llm-providers/storage', () => ({
  createDefaultBrowserOSProvider: () => ({}),
}))

const { putProvider, ProviderSaveError } = await import('./llm-providers.api')
const originalFetch = globalThis.fetch
const config: LlmProviderConfig = {
  id: 'submitted',
  type: 'openrouter',
  name: 'Test',
  modelId: 'model',
  supportsImages: true,
  contextWindow: 128000,
  temperature: 0.2,
  createdAt: 1,
  updatedAt: 1,
}
let reply: unknown
let status: number

beforeEach(() => {
  status = 200
  globalThis.fetch = Object.assign(
    async () => Response.json(reply, { status }),
    {
      preconnect: originalFetch.preconnect,
    },
  )
})
afterEach(() => {
  globalThis.fetch = originalFetch
})

it('adopts the saved identity and resolved fields returned by the server', async () => {
  reply = {
    provider: {
      ...config,
      id: 'canonical',
      kind: 'llm',
      baseUrl: 'https://openrouter.ai/api/v1',
      hasApiKey: true,
    },
  }
  expect(await putProvider(config)).toMatchObject({
    id: 'canonical',
    baseUrl: 'https://openrouter.ai/api/v1',
    hasApiKey: true,
  })
})

it('preserves server field errors for the settings form', async () => {
  status = 400
  reply = {
    error: 'Enter an API key.',
    fieldErrors: { apiKey: 'Enter an API key.' },
  }
  const result = await putProvider(config).catch((error) => error)
  expect(result).toBeInstanceOf(ProviderSaveError)
  expect(result.fieldErrors).toEqual({ apiKey: 'Enter an API key.' })
})
