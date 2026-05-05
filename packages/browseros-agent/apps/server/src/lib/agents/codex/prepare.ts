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
import { materializeCodexHome } from '../acpx-runtime-context'

/** Prepares Codex with a contained CODEX_HOME and BrowserOS agent home. */
export async function prepareCodexContext(
  input: PrepareAcpxAgentContextInput,
): Promise<PreparedAcpxAgentContext> {
  const common = await prepareBrowserosManagedContext(input)
  await materializeCodexHome({
    paths: common.paths,
    skillNames: common.skillNames,
  })
  return finishBrowserosManagedContext({
    ...common,
    commandEnv: {
      AGENT_HOME: common.paths.agentHome,
      CODEX_HOME: common.paths.codexHome,
    },
  })
}
