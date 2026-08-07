import { beforeAll, describe, expect, it, mock } from 'bun:test'
import * as _providerTemplates from '../../lib/llm-providers/providerTemplates'
import type { LlmProviderConfig } from '../../lib/llm-providers/types'
import type { OAuthProviderFlowConfig } from './oauth-provider-flow.hooks'

// sonner is an npm package; total-replacement is intentional.
mock.module('sonner', () => ({
  toast: {
    error: () => {},
    info: () => {},
    success: () => {},
  },
}))

// Bun's module registry is process-scoped, so complete replacements are
// checked against the real module shape and partial mocks pass through exports.
const trackMock = {
  track: () => {},
} satisfies typeof import('@/lib/metrics/track')
mock.module('@/lib/metrics/track', () => trackMock)

const clientOauthMock = {
  requestDeviceCode: async () => {
    throw new Error('not used')
  },
  startTokenPolling: () => {},
} satisfies typeof import('@/lib/llm-providers/client-oauth')
mock.module('@/lib/llm-providers/client-oauth', () => clientOauthMock)

const providerDisplayNamesMock = {
  CHATGPT_PROVIDER_DISPLAY_NAME: 'ChatGPT',
} satisfies typeof import('@/lib/llm-providers/provider-display-names')
mock.module(
  '@/lib/llm-providers/provider-display-names',
  () => providerDisplayNamesMock,
)

mock.module('@/lib/llm-providers/providerTemplates', () => ({
  ..._providerTemplates,
  getProviderTemplate: (providerType: string) =>
    providerType === 'chatgpt-pro'
      ? {
          defaultModelId: 'gpt-5.5',
          supportsImages: true,
          contextWindow: 1050000,
        }
      : undefined,
}))

const oauthStatusHooksMock = {
  useOAuthStatus: () => ({
    status: null,
    isPolling: false,
    startPolling: () => {},
    stopPolling: () => {},
    refresh: async () => null,
    disconnect: async () => {},
  }),
} satisfies typeof import('@/modules/llm-providers/oauth-status.hooks')
mock.module(
  '@/modules/llm-providers/oauth-status.hooks',
  () => oauthStatusHooksMock,
)

const chatgptConfig: OAuthProviderFlowConfig = {
  providerType: 'chatgpt-pro',
  displayName: 'ChatGPT',
  startedEvent: 'settings.chatgpt_pro.oauth_started',
  completedEvent: 'settings.chatgpt_pro.oauth_completed',
  disconnectedEvent: 'settings.chatgpt_pro.oauth_disconnected',
}

let saveOAuthProviderFromStatus: typeof import('./oauth-provider-flow.hooks').saveOAuthProviderFromStatus

beforeAll(async () => {
  ;({ saveOAuthProviderFromStatus } = await import(
    './oauth-provider-flow.hooks'
  ))
})

describe('saveOAuthProviderFromStatus', () => {
  it('waits for provider storage before resolving', async () => {
    let resolveSave: (() => void) | undefined
    let settled = false
    let savedProvider: LlmProviderConfig | undefined

    const promise = saveOAuthProviderFromStatus({
      config: chatgptConfig,
      status: { email: 'user@example.com' },
      now: 1234,
      saveProvider: async (provider) => {
        savedProvider = provider
        await new Promise<void>((resolve) => {
          resolveSave = resolve
        })
      },
    })
    promise.then(
      () => {
        settled = true
      },
      () => {
        settled = true
      },
    )

    await Promise.resolve()

    expect(settled).toBe(false)
    expect(savedProvider).toMatchObject({
      id: 'chatgpt-pro-1234',
      type: 'chatgpt-pro',
      name: 'ChatGPT',
      modelId: 'gpt-5.5',
      contextWindow: 1050000,
      reasoningEffort: 'medium',
      reasoningSummary: 'auto',
    })

    resolveSave?.()
    const provider = await promise

    expect(settled).toBe(true)
    if (!savedProvider) throw new Error('Provider was not saved')
    expect(provider).toEqual(savedProvider)
  })

  it('surfaces storage failures to the caller', async () => {
    await expect(
      saveOAuthProviderFromStatus({
        config: chatgptConfig,
        status: {},
        now: 1234,
        saveProvider: async () => {
          throw new Error('storage failed')
        },
      }),
    ).rejects.toThrow('storage failed')
  })
})
