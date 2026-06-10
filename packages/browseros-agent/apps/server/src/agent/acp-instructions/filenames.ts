/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'

/**
 * Picks the workspace-rooted markdown file that the spawned ACP agent
 * reads at session start. Claude Code reads `CLAUDE.md`; everything
 * else (Codex, custom ACP agents) follows the OpenAI / Codex CLI
 * `AGENTS.md` convention which has become the de-facto fallback.
 */
export function instructionFilenameFor(providerType: string): string {
  if (providerType === LLM_PROVIDERS.CLAUDE_CODE) return 'CLAUDE.md'
  return 'AGENTS.md'
}
