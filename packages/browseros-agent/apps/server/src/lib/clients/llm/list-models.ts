/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { TIMEOUTS } from '@browseros/shared/constants/timeouts'

export interface ListModelsConfig {
  provider: string
  baseUrl?: string
  apiKey?: string
}

export interface ProviderModelInfo {
  modelId: string
  contextLength?: number
}

/** Strips a trailing slash and a trailing `/v1` so `${base}/v1/models` is well formed. */
function normalizeBaseUrl(raw: string): string {
  let base = raw.trim().replace(/\/+$/, '')
  if (base.endsWith('/v1')) base = base.slice(0, -3)
  return base
}

export async function listProviderModels(
  config: ListModelsConfig,
): Promise<ProviderModelInfo[]> {
  if (!config.baseUrl) {
    throw new Error(`[${config.provider}] baseUrl is required to list models`)
  }

  const url = `${normalizeBaseUrl(config.baseUrl)}/v1/models`
  const response = await fetch(url, {
    headers: config.apiKey ? { Authorization: `Bearer ${config.apiKey}` } : {},
    signal: AbortSignal.timeout(TIMEOUTS.TEST_PROVIDER),
  })

  if (!response.ok) {
    throw new Error(
      `[${config.provider}] ${response.status} ${response.statusText || ''}`.trim(),
    )
  }

  const body = (await response.json()) as {
    data?: Array<{ id?: unknown; context_length?: unknown }>
  }

  return (body.data ?? [])
    .filter((m) => typeof m.id === 'string' && m.id)
    .map((m) => ({
      modelId: m.id as string,
      contextLength:
        typeof m.context_length === 'number' ? m.context_length : undefined,
    }))
}
