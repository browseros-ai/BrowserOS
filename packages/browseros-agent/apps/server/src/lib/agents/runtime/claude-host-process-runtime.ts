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
import { resolveAgentRuntimePaths } from '../acpx/runtime-context'
import { HostProcessAgentRuntime } from './host-process-agent-runtime'
import { getAgentRuntimeRegistry } from './registry'
import type { RuntimeDescriptor } from './types'

const CLAUDE_BINARY = 'claude'

export interface ClaudeRuntimeConfig {
  browserosDir: string
}

export class ClaudeRuntime extends HostProcessAgentRuntime {
  readonly descriptor: RuntimeDescriptor & { kind: 'host-process' } = {
    adapterId: 'claude',
    displayName: 'Claude Code',
    kind: 'host-process',
    platforms: ['darwin', 'linux'],
  }

  private readonly claudeConfig: ClaudeRuntimeConfig

  constructor(
    deps: ConstructorParameters<typeof HostProcessAgentRuntime>[0],
    config: ClaudeRuntimeConfig,
  ) {
    super(deps)
    this.claudeConfig = config
  }

  getPerAgentHomeDir(agentId: string): string {
    return resolveAgentRuntimePaths({
      browserosDir: this.claudeConfig.browserosDir,
      agentId,
    }).agentHome
  }

  prepareTurnContext(
    input: PrepareAcpxAgentContextInput,
  ): Promise<PreparedAcpxAgentContext> {
    return prepareClaudeCodeContext(input)
  }
}

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

export interface ConfigureClaudeRuntimeOptions {
  browserosDir?: string
}

export function configureClaudeRuntime(
  options: ConfigureClaudeRuntimeOptions = {},
): ClaudeRuntime {
  const browserosDir = options.browserosDir ?? getBrowserosDir()
  const runtime = new ClaudeRuntime(
    { binaryName: CLAUDE_BINARY },
    { browserosDir },
  )
  getAgentRuntimeRegistry().register(runtime)
  logger.debug('ClaudeRuntime registered', { binary: CLAUDE_BINARY })
  return runtime
}

export function getClaudeRuntime(): ClaudeRuntime | null {
  const r = getAgentRuntimeRegistry().get('claude')
  return r instanceof ClaudeRuntime ? r : null
}
