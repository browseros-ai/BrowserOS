import { beforeAll, describe, expect, it, mock } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { OAuthProviderFlowConfig } from './oauth-provider-flow.hooks'

mock.module('sonner', () => ({
  toast: {
    error: () => {},
    info: () => {},
    success: () => {},
  },
}))

mock.module('@/lib/metrics/track', () => ({
  track: () => {},
}))

mock.module('@/lib/llm-providers/client-oauth', () => ({
  requestDeviceCode: async () => {
    throw new Error('not used')
  },
  startTokenPolling: () => {},
}))

mock.module('@/lib/llm-providers/provider-display-names', () => ({
  CHATGPT_PROVIDER_DISPLAY_NAME: 'ChatGPT',
}))

mock.module('@/lib/llm-providers/providerTemplates', () => ({
  getProviderTemplate: (providerType: string) =>
    providerType === 'chatgpt-pro'
      ? {
          defaultModelId: 'gpt-5.5',
          supportsImages: true,
          contextWindow: 1050000,
        }
      : undefined,
}))

mock.module('@/modules/llm-providers/oauth-status.hooks', () => ({
  useOAuthStatus: () => ({
    status: null,
    startPolling: () => {},
    disconnect: async () => {},
  }),
}))

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
