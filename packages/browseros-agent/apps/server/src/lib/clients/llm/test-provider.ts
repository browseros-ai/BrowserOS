/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { TIMEOUTS } from '@browseros/shared/constants/timeouts'
import type { LLMConfig } from '@browseros/shared/schemas/llm'
import { streamText } from 'ai'
import { resolveLLMConfig } from './config'
import { createLLMProvider } from './provider'

export interface ProviderTestConfig extends LLMConfig {
  model: string
  upstreamProvider?: string
}

export interface ProviderTestResult {
  success: boolean
  message: string
  responseTime?: number
}

const TEST_PROMPT = "Respond with exactly: 'ok'"

export async function testProviderConnection(
  config: ProviderTestConfig,
  browserosId?: string,
): Promise<ProviderTestResult> {
  const startTime = performance.now()

  try {
    const resolvedConfig = await resolveLLMConfig(config, browserosId)
    const model = createLLMProvider(resolvedConfig)

    // streamText works for all providers including Codex (which requires streaming)
    const stream = streamText({
      model,
      messages: [{ role: 'user', content: TEST_PROMPT }],
      abortSignal: AbortSignal.timeout(TIMEOUTS.TEST_PROVIDER),
    })
    // stream.text resolves to '' when the underlying provider errors
    // mid-stream (the SDK surfaces streaming errors via `onError`, not
    // by rejecting `.text`). Iterate `textStream` so bad URLs / auth
    // failures / DNS errors throw and land in the catch below instead
    // of being reported as a false-positive "Provider responded".
    const chunks: string[] = []
    for await (const chunk of stream.textStream) {
      chunks.push(chunk)
    }
    const text = chunks.join('')
    const responseTime = Math.round(performance.now() - startTime)

    if (text) {
      const preview = text.length > 100 ? `${text.slice(0, 100)}...` : text
      return {
        success: true,
        message: `Connection successful. Response: "${preview}"`,
        responseTime,
      }
    }

    return {
      success: true,
      message: 'Connection successful. Provider responded.',
      responseTime,
    }
  } catch (error) {
    const responseTime = Math.round(performance.now() - startTime)
    const errorMessage = error instanceof Error ? error.message : String(error)

    return {
      success: false,
      message: `[${config.provider}] ${errorMessage}`,
      responseTime,
    }
  }
}
