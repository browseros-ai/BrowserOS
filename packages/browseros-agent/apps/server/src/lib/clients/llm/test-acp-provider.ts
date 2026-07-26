/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import {
  probeAcpAgent,
  type ServerAcpxProbeInput,
  type ServerAcpxProbeResult,
} from '../../../api/services/acpx-probe/probeAgent'
import type { ProviderTestResult } from './test-provider'

export type ProbeAcpAgentFn = (
  input: ServerAcpxProbeInput,
) => Promise<ServerAcpxProbeResult>

export interface TestAcpProviderInput {
  provider: string
  model?: string
  acpAgentId?: string
  acpCommand?: string
  acpFixedWorkspacePath?: string
}

const BUILT_IN_AGENT_BY_PROVIDER: Record<string, string> = {
  [LLM_PROVIDERS.CLAUDE_CODE]: 'claude',
  [LLM_PROVIDERS.CODEX]: 'codex',
}

function resolveAgentId(input: TestAcpProviderInput): string | undefined {
  if (input.provider === LLM_PROVIDERS.ACP_CUSTOM) return input.acpAgentId
  return input.acpAgentId ?? BUILT_IN_AGENT_BY_PROVIDER[input.provider]
}

function humanizeProbeError(code: string, fallback: string): string {
  switch (code) {
    case 'spawn_failed':
      return 'Agent binary could not be spawned. Make sure it is installed and on your PATH.'
    case 'initialize_timeout':
      return 'Agent did not respond to initialize in time. Try again, or raise BROWSEROS_ACPX_PROBE_TIMEOUT_MS.'
    case 'session_new_timeout':
      return 'Agent did not respond to session/new in time.'
    case 'auth_required':
      return 'Agent declined session/new because it needs credentials. Sign in via the agent CLI and try again.'
    case 'agent_crashed':
      return 'Agent crashed during the probe. Re-run; if it persists, check the agent CLI directly.'
    case 'protocol_mismatch':
      return 'Agent reported an ACP protocol version this build does not support.'
    default:
      return fallback || `Probe failed: ${code}`
  }
}

export async function testAcpProvider(
  input: TestAcpProviderInput,
  options: {
    probe?: ProbeAcpAgentFn
    resourcesDir?: string | null
  } = {},
): Promise<ProviderTestResult> {
  const probe = options.probe ?? probeAcpAgent
  const startTime = performance.now()
  const agentId = resolveAgentId(input)
  if (input.provider === LLM_PROVIDERS.ACP_CUSTOM) {
    if (!agentId || !input.acpCommand) {
      return {
        success: false,
        message: 'Custom ACP agent requires an agent id and a command.',
        responseTime: Math.round(performance.now() - startTime),
      }
    }
  } else if (!agentId) {
    return {
      success: false,
      message: `Unknown ACP provider type: ${input.provider}`,
      responseTime: Math.round(performance.now() - startTime),
    }
  }
  const probeResult = await probe({
    agentId,
    command: input.acpCommand,
    cwd: input.acpFixedWorkspacePath,
    resourcesDir: options.resourcesDir,
  })
  const responseTime = Math.round(performance.now() - startTime)
  if (probeResult.error) {
    return {
      success: false,
      message: humanizeProbeError(
        probeResult.error.code,
        probeResult.error.message,
      ),
      responseTime,
    }
  }
  if (probeResult.models.length === 0) {
    return {
      success: false,
      message: 'Agent responded but did not advertise any settable models.',
      responseTime,
    }
  }
  const expected = input.model
  if (expected) {
    const settable = new Set(probeResult.models.map((m) => m.id))
    if (!settable.has(expected)) {
      const available = probeResult.models.map((m) => m.id).join(', ')
      return {
        success: false,
        message: `Agent connected but model "${expected}" is not advertised. Available: ${available}.`,
        responseTime,
      }
    }
  }
  const agentLabel =
    probeResult.agentInfo?.title ??
    probeResult.agentInfo?.name ??
    agentId ??
    'agent'
  return {
    success: true,
    message: `Connected to ${agentLabel}; ${probeResult.models.length} model(s) available.`,
    responseTime,
  }
}
