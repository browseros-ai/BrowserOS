/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { homedir } from 'node:os'
import type {
  AcpxMcpServerConfig,
  SessionAgentOptions,
} from '@browseros/acpx-ai-provider'
import type { BrowserContext } from '@browseros/shared/schemas/browser-context'
import type { AcpAgentDefinition } from '../agent-types'
import { DANGEROUS_ALLOW_MODE_CANDIDATES } from '../host-acp/config'
import { resolveAcpSpawnCommand } from '../host-acp/launcher'
import { deriveAcpSessionKey } from '../storage/acp-agent-store'
import { loadBrowserOsSkill } from './browseros-skill'
import { buildAcpMcpServers } from './mcp-servers'

export interface BuildAcpAgentPolicyInput {
  agent: AcpAgentDefinition
  conversationId: string
  serverPort: number
  browserContext?: BrowserContext
  resourcesDir?: string | null
  browserosDir?: string | null
}

export interface AcpAgentPolicy {
  adapter: string
  cwd: string
  sessionKey: string
  agentRegistryOverrides: Record<string, string | string[]>
  mcpServers: AcpxMcpServerConfig[]
  sessionOptions: SessionAgentOptions
  fullAccessModeCandidates: readonly string[]
}

export async function buildAcpAgentPolicy(
  input: BuildAcpAgentPolicyInput,
): Promise<AcpAgentPolicy> {
  const skill = await loadBrowserOsSkill(input.resourcesDir)
  // Custom agents get a per-agent registry id so two concurrent custom agents
  // with different commands never collide on a shared 'custom' key.
  const adapter =
    input.agent.type === 'custom'
      ? `custom:${input.agent.id}`
      : input.agent.type
  const launcher = resolveAcpSpawnCommand({
    agentType: input.agent.type,
    customCommand:
      input.agent.type === 'custom'
        ? input.agent.customConfig?.command
        : undefined,
    browserosDir: input.browserosDir,
    resourcesDir: input.resourcesDir,
    spawnEnv: buildSpawnEnvironment(input.agent, skill),
  })

  return {
    adapter,
    cwd: input.agent.workingDirectory?.trim() || homedir(),
    sessionKey: deriveAcpSessionKey(input.agent.id, input.conversationId),
    agentRegistryOverrides: { [adapter]: launcher.argv },
    mcpServers: buildAcpMcpServers({
      serverPort: input.serverPort,
      conversationId: input.conversationId,
      browserContext: input.browserContext,
    }),
    sessionOptions: buildSessionOptions(input.agent, skill),
    fullAccessModeCandidates: resolveFullAccessModeCandidates(input.agent),
  }
}

function resolveFullAccessModeCandidates(
  agent: AcpAgentDefinition,
): readonly string[] {
  if (agent.type === 'custom') return agent.customConfig?.fullAccessModes ?? []
  return DANGEROUS_ALLOW_MODE_CANDIDATES[agent.type]
}

function buildSessionOptions(
  agent: AcpAgentDefinition,
  skill: string,
): SessionAgentOptions {
  if (agent.type === 'claude') {
    return {
      ...(agent.modelId ? { model: agent.modelId } : {}),
      systemPrompt: { append: skill },
    }
  }

  if (agent.type === 'custom') {
    const append = agent.customConfig?.systemPromptAppend
    return {
      ...(agent.modelId ? { model: agent.modelId } : {}),
      ...(append ? { systemPrompt: { append } } : {}),
    }
  }

  return {}
}

function buildSpawnEnvironment(
  agent: AcpAgentDefinition,
  skill: string,
): Record<string, string> | undefined {
  if (agent.type === 'custom') return agent.customConfig?.env

  if (agent.type !== 'codex') return undefined

  // acpx applies its snake_case record policy to SessionAgentOptions.env, so
  // uppercase process variables must stay at the process-launch boundary.
  return {
    CODEX_CONFIG: JSON.stringify(buildCodexConfig(agent, skill)),
    INITIAL_AGENT_MODE: 'agent-full-access',
  }
}

function buildCodexConfig(
  agent: AcpAgentDefinition,
  skill: string,
): Record<string, unknown> {
  return {
    developer_instructions: skill,
    ...(agent.modelId ? { model: agent.modelId } : {}),
    ...(agent.reasoningEffort
      ? { model_reasoning_effort: agent.reasoningEffort }
      : {}),
    plugins: {
      'browser@openai-bundled': { enabled: false },
      'chrome@openai-bundled': { enabled: false },
      'computer-use@openai-bundled': { enabled: false },
    },
  }
}
