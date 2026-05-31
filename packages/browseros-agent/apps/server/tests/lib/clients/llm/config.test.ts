/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { beforeEach, describe, expect, it, mock } from 'bun:test'
import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'

const tokenManager = {
  refreshIfExpired: mock(async () => ({
    accessToken: 'access-token',
    refreshToken: 'refresh-token',
    expiresAt: Date.now() + 3600_000,
    accountId: 'account-id',
  })),
}

mock.module('../../../../src/lib/clients/oauth', () => ({
  getOAuthTokenManager: () => tokenManager,
}))

const { resolveLLMConfig } = await import(
  '../../../../src/lib/clients/llm/config'
)

describe('resolveLLMConfig', () => {
  beforeEach(() => {
    tokenManager.refreshIfExpired.mockClear()
  })

  it('defaults ChatGPT Plus/Pro OAuth providers to GPT-5.5', async () => {
    const resolved = await resolveLLMConfig(
      { provider: LLM_PROVIDERS.CHATGPT_PRO },
      'browseros-id',
    )

    expect(tokenManager.refreshIfExpired).toHaveBeenCalledWith('chatgpt-pro')
    expect(resolved).toMatchObject({
      provider: LLM_PROVIDERS.CHATGPT_PRO,
      model: 'gpt-5.5',
      apiKey: 'access-token',
      upstreamProvider: 'openai',
      accountId: 'account-id',
    })
  })
})
