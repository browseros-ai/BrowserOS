/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Registry of OpenClaw CLI-backed providers. Add entries here as we
 * enable more (Gemini CLI, Codex CLI, etc.).
 */

import { CLAUDE_CLI_PROVIDER } from './claude-cli'
import type { OpenClawCliProvider } from './types'

export const OPENCLAW_CLI_PROVIDERS: readonly OpenClawCliProvider[] = [
  CLAUDE_CLI_PROVIDER,
]

export function getOpenClawCliProvider(
  id: string,
): OpenClawCliProvider | undefined {
  return OPENCLAW_CLI_PROVIDERS.find((provider) => provider.id === id)
}

export function isOpenClawCliProviderId(id: string): boolean {
  return OPENCLAW_CLI_PROVIDERS.some((provider) => provider.id === id)
}

export function buildOpenClawCliProviderModelRef(
  providerId: string,
  modelId: string,
): string {
  return `${providerId}/${modelId}`
}
