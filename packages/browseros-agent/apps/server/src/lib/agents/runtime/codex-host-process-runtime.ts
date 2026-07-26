/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { getBrowserosDir } from '../../browseros-dir'
import { logger } from '../../logger'
import type {
  PrepareAcpxAgentContextInput,
  PreparedAcpxAgentContext,
} from '../acpx/agent-adapter'
import {
  finishBrowserosManagedContext,
  prepareBrowserosManagedContext,
} from '../acpx/agent-common'
import {
  materializeCodexHome,
  resolveAgentRuntimePaths,
} from '../acpx/runtime-context'
import { HostProcessAgentRuntime } from './host-process-agent-runtime'
import { getAgentRuntimeRegistry } from './registry'
import type { RuntimeDescriptor } from './types'

const CODEX_BINARY = 'codex'

export interface CodexRuntimeConfig {
  browserosDir: string
}

export class CodexRuntime extends HostProcessAgentRuntime {
  readonly descriptor: RuntimeDescriptor & { kind: 'host-process' } = {
    adapterId: 'codex',
    displayName: 'Codex',
    kind: 'host-process',
    platforms: ['darwin', 'linux'],
  }

  private readonly codexConfig: CodexRuntimeConfig

  constructor(
    deps: ConstructorParameters<typeof HostProcessAgentRuntime>[0],
    config: CodexRuntimeConfig,
  ) {
    super(deps)
    this.codexConfig = config
  }

  getPerAgentHomeDir(agentId: string): string {
    return resolveAgentRuntimePaths({
      browserosDir: this.codexConfig.browserosDir,
      agentId,
    }).agentHome
  }

  prepareTurnContext(
    input: PrepareAcpxAgentContextInput,
  ): Promise<PreparedAcpxAgentContext> {
    return prepareCodexContext(input)
  }
}

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

export interface ConfigureCodexRuntimeOptions {
  browserosDir?: string
}

export function configureCodexRuntime(
  options: ConfigureCodexRuntimeOptions = {},
): CodexRuntime {
  const browserosDir = options.browserosDir ?? getBrowserosDir()
  const runtime = new CodexRuntime(
    { binaryName: CODEX_BINARY },
    { browserosDir },
  )
  getAgentRuntimeRegistry().register(runtime)
  logger.debug('CodexRuntime registered', { binary: CODEX_BINARY })
  return runtime
}

export function getCodexRuntime(): CodexRuntime | null {
  const r = getAgentRuntimeRegistry().get('codex')
  return r instanceof CodexRuntime ? r : null
}
