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
  runStreamText: typeof streamText = streamText,
): Promise<ProviderTestResult> {
  const startTime = performance.now()

  try {
    const resolvedConfig = await resolveLLMConfig(config, browserosId)
    const model = createLLMProvider(resolvedConfig)

    // AI SDK reports provider failures through onError while textStream ends cleanly.
    let capturedError: unknown = null
    const stream = runStreamText({
      model,
      messages: [{ role: 'user', content: TEST_PROMPT }],
      abortSignal: AbortSignal.timeout(TIMEOUTS.TEST_PROVIDER),
      onError: ({ error }) => {
        capturedError = error
      },
    })
    const chunks: string[] = []
    for await (const chunk of stream.textStream) {
      chunks.push(chunk)
    }
    if (capturedError) throw capturedError
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
