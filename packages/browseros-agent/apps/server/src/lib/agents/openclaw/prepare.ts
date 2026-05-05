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
  buildBrowserosAcpPrompt,
  ensureUsableCwd,
  resolveAgentRuntimePaths,
} from '../acpx-runtime-context'

export { maybeHandleOpenClawTurn } from './image-turn'

const OPENCLAW_BROWSEROS_ACP_INSTRUCTIONS =
  '<role>You are running inside BrowserOS through the OpenClaw ACP adapter. Use your OpenClaw identity, memory, and browser tools.</role>'

/**
 * Prepares OpenClaw without BrowserOS SOUL/MEMORY or BrowserOS MCP.
 * OpenClaw runs inside the gateway VM/container, so a selected host cwd is not visible there.
 */
export async function prepareOpenClawContext(
  input: PrepareAcpxAgentContextInput,
): Promise<PreparedAcpxAgentContext> {
  const paths = resolveAgentRuntimePaths({
    browserosDir: input.browserosDir,
    agentId: input.agent.id,
  })
  await ensureUsableCwd(paths.effectiveCwd, true)
  return {
    cwd: paths.effectiveCwd,
    runtimeSessionKey: input.sessionKey,
    runPrompt: buildBrowserosAcpPrompt(
      OPENCLAW_BROWSEROS_ACP_INSTRUCTIONS,
      input.message,
    ),
    commandEnv: {},
    commandIdentity: 'openclaw',
    useBrowserosMcp: false,
    openclawSessionKey: input.sessionKey,
  }
}
