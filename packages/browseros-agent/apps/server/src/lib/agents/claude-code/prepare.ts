/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type {
  PrepareAcpxAgentContextInput,
  PreparedAcpxAgentContext,
} from '../acpx-agent-adapter'
import {
  finishBrowserosManagedContext,
  prepareBrowserosManagedContext,
} from '../acpx-agent-common'

/** Prepares Claude Code with BrowserOS agent home while preserving host Claude auth. */
export async function prepareClaudeCodeContext(
  input: PrepareAcpxAgentContextInput,
): Promise<PreparedAcpxAgentContext> {
  const common = await prepareBrowserosManagedContext(input)
  return finishBrowserosManagedContext({
    ...common,
    commandEnv: {
      AGENT_HOME: common.paths.agentHome,
    },
  })
}
