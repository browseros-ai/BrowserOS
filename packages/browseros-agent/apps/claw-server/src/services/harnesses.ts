/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Live harness labels shared by connection route validation and the
 * agent-mcp-manager integration. Keep the labels in sync with
 * apps/claw-app/components/harness/harness.types.ts.
 */

import type { AgentId } from '@browseros/agent-mcp-manager'
import { z } from 'zod'

export const harnessEnum = z.enum([
  'Claude Code',
  'Codex',
  'Cursor',
  'OpenCode',
  'Antigravity',
  'VS Code',
  'Zed',
])
export type Harness = z.infer<typeof harnessEnum>

export const HARNESS_TO_AGENT_ID: Record<Harness, AgentId> = {
  'Claude Code': 'claude-code',
  Codex: 'codex',
  Cursor: 'cursor',
  OpenCode: 'opencode',
  Antigravity: 'antigravity',
  'VS Code': 'vscode',
  Zed: 'zed',
}
