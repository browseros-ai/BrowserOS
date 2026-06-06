/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { TIMEOUTS } from '@browseros/shared/constants/timeouts'
import type { LLMConfig } from '@browseros/shared/schemas/llm'
import { generateText } from 'ai'
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

    // Use generateText for testing to get clear API errors (streamText wraps
    // APICallError in NoOutputGeneratedError and loses responseBody details).
    const result = await generateText({
      model,
      messages: [{ role: 'user', content: TEST_PROMPT }],
      maxRetries: 0,
      abortSignal: AbortSignal.timeout(TIMEOUTS.TEST_PROVIDER),
    })
    const text = result.text
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
    const errorMessage = extractProviderErrorMessage(error, config.provider)

    return {
      success: false,
      message: `[${config.provider}] ${errorMessage}`,
      responseTime,
    }
  }
}

function extractProviderErrorMessage(
  error: unknown,
  _provider: string,
): string {
  // generateText preserves APICallError directly, so responseBody is available
  // on the error object and usually carries the provider's real message.
  if (
    error != null &&
    typeof error === 'object' &&
    'responseBody' in error &&
    typeof (error as { responseBody?: string }).responseBody === 'string'
  ) {
    try {
      const parsed = JSON.parse(
        (error as { responseBody: string }).responseBody,
      )
      return (
        parsed?.error?.message ||
        parsed?.message ||
        parsed?.error?.code ||
        (error instanceof Error ? error.message : String(error))
      )
    } catch {
      // Not valid JSON, fall through
    }
  }

  if (error instanceof Error) {
    return error.message
  }
  return String(error)
}
