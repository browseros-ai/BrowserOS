/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { LLMConfig } from '@browseros/shared/schemas/llm'
import type { ProviderRow } from '../../lib/db/schema'
import type {
  BrowserOsChatRequest,
  HydratedBrowserOsChatRequest,
} from '../types'

export interface ChatProviderLookup {
  get(id: string): Promise<ProviderRow | null>
  getDefault(): Promise<ProviderRow | null>
}

export type HydrationResult =
  | { ok: true; request: HydratedBrowserOsChatRequest }
  | { ok: false; error: string }

function toLlmConfig(
  row: ProviderRow,
): Partial<LLMConfig> & { model?: string } {
  return {
    provider: row.type as LLMConfig['provider'],
    providerId: row.id,
    model: row.modelId ?? undefined,
    apiKey: row.apiKey ?? undefined,
    baseUrl: row.baseUrl ?? undefined,
    resourceName: row.resourceName ?? undefined,
    region: row.region ?? undefined,
    accessKeyId: row.accessKeyId ?? undefined,
    secretAccessKey: row.secretAccessKey ?? undefined,
    sessionToken: row.sessionToken ?? undefined,
    reasoningEffort: row.reasoningEffort as LLMConfig['reasoningEffort'],
    reasoningSummary: row.reasoningSummary as LLMConfig['reasoningSummary'],
  }
}

/**
 * Fills a chat request's provider configuration from the stored row.
 *
 * The server owns the provider list and which one is selected, so a client only
 * has to name an id, and with none given the selected provider is used. The
 * row wins over anything sent inline, because it is the source of truth and a
 * client may be holding a copy from before an edit.
 *
 * A request that names nothing the server knows keeps whatever it sent, which
 * is how a client from before this change still works: it ships the whole
 * configuration and never relies on the lookup.
 */
export async function hydrateChatProvider(
  request: BrowserOsChatRequest,
  store: ChatProviderLookup,
): Promise<HydrationResult> {
  const namedId = request.target.providerId
  const row = namedId ? await store.get(namedId) : await store.getDefault()

  if (row && row.kind !== 'llm') {
    // Reached by naming an acp agent on the browseros path, or by having one
    // selected while the client sends no target. Falling through would run the
    // conversation on some other provider entirely.
    return {
      ok: false,
      error: `Provider ${row.id} is a coding agent and cannot serve a browseros chat request`,
    }
  }

  const hydrated = row
    ? { ...request, ...toLlmConfig(row) }
    : { ...request, providerId: namedId }

  if (!hydrated.provider) {
    return {
      ok: false,
      error: namedId
        ? `Unknown provider ${namedId}`
        : 'No provider given and none is selected',
    }
  }

  const providerId = row?.id ?? namedId
  if (!providerId) {
    return { ok: false, error: 'No provider given and none is selected' }
  }

  return {
    ok: true,
    request: {
      ...hydrated,
      provider: hydrated.provider,
      contextWindowSize: row?.contextWindow ?? request.contextWindowSize,
      supportsImages: row ? row.supportsImages : request.supportsImages,
      target: { type: 'browseros', providerId },
    },
  }
}
