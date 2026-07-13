/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { createAnthropic } from '@ai-sdk/anthropic'
import { createOpenAICompatible } from '@ai-sdk/openai-compatible'
import type { LanguageModel } from 'ai'

interface MinimaxProviderConfig {
  apiKey?: string
  baseUrl?: string
}

export function createMinimaxProvider(
  config: MinimaxProviderConfig,
): (modelId: string) => LanguageModel {
  if (!config.baseUrl) throw new Error('MiniMax provider requires baseUrl')
  if (!config.apiKey) throw new Error('MiniMax provider requires apiKey')

  const baseUrl = config.baseUrl.replace(/\/+$/, '')
  if (baseUrl.endsWith('/anthropic')) {
    return createAnthropic({
      name: 'minimax',
      baseURL: `${baseUrl}/v1`,
      apiKey: config.apiKey,
    })
  }

  return createOpenAICompatible({
    name: 'minimax',
    baseURL: baseUrl,
    apiKey: config.apiKey,
  })
}
