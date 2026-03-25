/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Internal types for LLM client.
 */

import type { LLMConfig } from '@browseros/shared/schemas/llm'

export interface ResolvedLLMConfig extends LLMConfig {
  model: string
  baseUrl?: string
  apiKey?: string
  resourceName?: string
  upstreamProvider?: string
  browserosId?: string
  accountId?: string
}
