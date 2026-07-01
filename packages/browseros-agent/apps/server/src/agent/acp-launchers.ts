/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  ensureRuntimeSkills,
  materializeCodexHome,
  resolveAgentRuntimePaths,
} from '../lib/agents/acpx/runtime-context'
import { resolveAcpSpawnCommand } from '../lib/agents/host-acp/launcher'

const BUILT_IN_ACP_AGENT_IDS = ['claude', 'codex'] as const

export interface ResolveBuiltInAcpAgentRegistryOverridesInput {
  readonly browserosDir: string
  readonly resourcesDir?: string | null
}

export async function resolveBuiltInAcpAgentRegistryOverrides(
  input: ResolveBuiltInAcpAgentRegistryOverridesInput,
): Promise<Record<string, string>> {
  const overrides: Record<string, string> = {}
  for (const agentId of BUILT_IN_ACP_AGENT_IDS) {
    const command = await resolveBuiltInAcpCommand({ ...input, agentId })
    if (command) overrides[agentId] = command
  }
  return overrides
}

async function resolveBuiltInAcpCommand(input: {
  readonly agentId: (typeof BUILT_IN_ACP_AGENT_IDS)[number]
  readonly browserosDir: string
  readonly resourcesDir?: string | null
}): Promise<string | null> {
  if (input.agentId === 'codex') {
    return resolveCodexCommand(input)
  }
  const launcher = resolveAcpSpawnCommand({
    agentType: input.agentId,
    browserosDir: input.browserosDir,
    resourcesDir: input.resourcesDir,
  })
  return launcher?.source === 'bundled-bun' ? launcher.command : null
}

async function resolveCodexCommand(input: {
  readonly browserosDir: string
  readonly resourcesDir?: string | null
}): Promise<string | null> {
  const baseLauncher = resolveAcpSpawnCommand({
    agentType: 'codex',
    browserosDir: input.browserosDir,
    resourcesDir: input.resourcesDir,
  })
  if (baseLauncher?.source !== 'bundled-bun') return null

  const paths = resolveAgentRuntimePaths({
    browserosDir: input.browserosDir,
    agentId: 'codex-provider',
  })
  const skillNames = await ensureRuntimeSkills(paths.runtimeSkillsDir)
  await materializeCodexHome({ paths, skillNames })

  const launcher = resolveAcpSpawnCommand({
    agentType: 'codex',
    browserosDir: input.browserosDir,
    resourcesDir: input.resourcesDir,
    env: { ...process.env, CODEX_HOME: paths.codexHome },
  })
  return launcher?.source === 'bundled-bun' ? launcher.command : null
}
