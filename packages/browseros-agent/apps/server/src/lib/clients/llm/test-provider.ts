/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { TIMEOUTS } from '@browseros/shared/constants/timeouts'
import type { LLMConfig } from '@browseros/shared/schemas/llm'
import { generateText } from 'ai'
import { logger } from '../../logger'
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
    logger.info('testProviderConnection start', {
      provider: config.provider,
      model: config.model,
      baseUrl: config.baseUrl,
      hasApiKey: !!config.apiKey,
    })
    const resolvedConfig = await resolveLLMConfig(config, browserosId)
    logger.info('testProviderConnection resolved', {
      provider: resolvedConfig.provider,
      model: resolvedConfig.model,
      baseUrl: resolvedConfig.baseUrl,
    })
    const model = createLLMProvider(resolvedConfig)
    logger.info('testProviderConnection model created', {
      provider: resolvedConfig.provider,
    })

    // Use generateText for testing to get clear API errors (streamText wraps
    // APICallError in NoOutputGeneratedError and loses responseBody details).
    const result = await generateText({
      model,
      messages: [{ role: 'user', content: TEST_PROMPT }],
      maxRetries: 0,
      abortSignal: AbortSignal.timeout(TIMEOUTS.TEST_PROVIDER),
    })
    const responseTime = Math.round(performance.now() - startTime)

    const text = result.text
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
    logger.info('testProviderConnection caught error', {
      provider: config.provider,
      errorType: typeof error,
      errorName: error instanceof Error ? error.name : undefined,
      errorMessage: error instanceof Error ? error.message : String(error),
    })
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
  // Check for API call error with response body (generateText preserves
  // APICallError directly, so responseBody is available on the error object)
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
      const msg =
        parsed?.error?.message ||
        parsed?.message ||
        parsed?.error?.code ||
        (error instanceof Error ? error.message : String(error))
      return msg
    } catch {
      // Not valid JSON, fall through
    }
  }

  if (error instanceof Error) {
    return error.message
  }

  return String(error)
}
