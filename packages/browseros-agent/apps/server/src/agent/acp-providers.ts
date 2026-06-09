/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Dependency-light helpers for identifying ACP-backed provider types.
 * Sits in its own module so a caller can ask "is this provider ACP?"
 * without dragging in the ai-sdk factory graph (which transitively
 * imports `simulateReadableStream` from `'ai'` and trips test files
 * that mock that module).
 */

import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'

export const ACP_PROVIDER_TYPES: ReadonlySet<string> = new Set([
  LLM_PROVIDERS.CLAUDE_CODE,
  LLM_PROVIDERS.CODEX,
  LLM_PROVIDERS.ACP_CUSTOM,
])

export function isAcpProvider(provider: string): boolean {
  return ACP_PROVIDER_TYPES.has(provider)
}
