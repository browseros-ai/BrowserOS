/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type {
  PrepareAcpxAgentContextInput,
  PreparedAcpxAgentContext,
} from './agent-adapter'
import type { AgentRuntimePaths } from './runtime-context'
import {
  BROWSEROS_ACPX_OPERATING_PROMPT_VERSION,
  buildAcpxRuntimePromptPrefix,
  buildBrowserosAcpPrompt,
  ensureAgentHome,
  ensureRuntimeSkills,
  ensureUsableCwd,
  resolveAgentRuntimePaths,
} from './runtime-context'
import {
  deriveRuntimeSessionKey,
  saveLatestRuntimeState,
} from './runtime-state'

export interface BrowserosManagedContext {
  input: PrepareAcpxAgentContextInput
  paths: AgentRuntimePaths
  skillNames: string[]
  promptPrefix: string
}

/** Builds the common BrowserOS-managed home, skills, cwd, and prompt prefix for Claude/Codex. */
export async function prepareBrowserosManagedContext(
  input: PrepareAcpxAgentContextInput,
): Promise<BrowserosManagedContext> {
  const paths = resolveAgentRuntimePaths({
    browserosDir: input.browserosDir,
    agentId: input.agent.id,
    sessionId: input.sessionId,
    cwd: input.cwdOverride,
  })
  await ensureUsableCwd(paths.effectiveCwd, !input.isSelectedCwd)
  await ensureAgentHome(paths)
  const skillNames = await ensureRuntimeSkills(paths.runtimeSkillsDir)
  const promptPrefix = buildAcpxRuntimePromptPrefix({
    agent: input.agent,
    paths,
    skillNames,
  })
  return { input, paths, skillNames, promptPrefix }
}

/** Finalizes BrowserOS-managed prep into the uniform adapter context consumed by AcpxRuntime. */
export async function finishBrowserosManagedContext(input: {
  input: PrepareAcpxAgentContextInput
  paths: AgentRuntimePaths
  skillNames: string[]
  promptPrefix: string
  commandEnv: Record<string, string>
  browserosMcpHost?: string
}): Promise<PreparedAcpxAgentContext> {
  const commandIdentity = stableCommandIdentity(input.commandEnv)
  const runtimeSessionKey = deriveRuntimeSessionKey({
    agentId: input.input.agent.id,
    sessionId: input.input.sessionId,
    adapter: input.input.agent.adapter,
    cwd: input.paths.effectiveCwd,
    agentHome: input.paths.agentHome,
    promptVersion: BROWSEROS_ACPX_OPERATING_PROMPT_VERSION,
    skillIdentity: input.skillNames.join(','),
    commandIdentity,
  })
  const latest = {
    sessionId: input.input.sessionId,
    runtimeSessionKey,
    cwd: input.paths.effectiveCwd,
    agentHome: input.paths.agentHome,
    updatedAt: Date.now(),
  }
  await Promise.all([
    saveLatestRuntimeState(input.paths.runtimeSessionStatePath, latest),
    saveLatestRuntimeState(input.paths.runtimeStatePath, latest),
  ])
  return {
    cwd: input.paths.effectiveCwd,
    runtimeSessionKey,
    runPrompt: buildBrowserosAcpPrompt(input.promptPrefix, input.input.message),
    commandEnv: input.commandEnv,
    commandIdentity,
    useBrowserosMcp: true,
    browserosMcpHost: input.browserosMcpHost,
  }
}

function stableCommandIdentity(env: Record<string, string>): string {
  return Object.entries(env)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, value]) => `${key}=${value}`)
    .join('\n')
}
