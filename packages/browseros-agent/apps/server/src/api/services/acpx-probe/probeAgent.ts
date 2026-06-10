/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { type AgentProbeResult, probeAgent as runProbe } from 'acp-probe'

export interface ServerAcpxProbeInput {
  agentId?: string
  command?: string
  cwd?: string
  timeoutMs?: number
}

export interface ServerAcpxProbeModel {
  id: string
  name?: string
  description?: string
}

export interface ServerAcpxProbeReasoning {
  values: string[]
  defaultValue?: string
}

export interface ServerAcpxProbeError {
  code: string
  message: string
  acpErrorCode?: number
}

export interface ServerAcpxProbeResult {
  models: ServerAcpxProbeModel[]
  reasoning: ServerAcpxProbeReasoning | null
  supportsConfigOption: boolean
  agentInfo: { name?: string; title?: string; version?: string } | null
  protocolVersion: number
  error?: ServerAcpxProbeError
}

const DEFAULT_PROBE_TIMEOUT_MS = 10_000

function resolveTimeout(requested?: number): number {
  const envValue = Number(process.env.BROWSEROS_ACPX_PROBE_TIMEOUT_MS)
  if (Number.isFinite(envValue) && envValue >= 1_000 && envValue <= 60_000) {
    return envValue
  }
  return requested ?? DEFAULT_PROBE_TIMEOUT_MS
}

export async function probeAcpAgent(
  input: ServerAcpxProbeInput,
): Promise<ServerAcpxProbeResult> {
  if (!input.agentId && !input.command) {
    throw new Error('Either agentId or command is required')
  }
  const timeoutMs = resolveTimeout(input.timeoutMs)
  const result = await runProbe({
    agent: input.agentId,
    command: input.command,
    cwd: input.cwd,
    authPolicy: 'skip',
    timeoutMs,
  })
  return normalizeProbeResult(result)
}

function normalizeProbeResult(r: AgentProbeResult): ServerAcpxProbeResult {
  // Prefer the settable list (modelConfig.values). Falls back to the
  // advertised models when modelConfig is null so the dialog can still
  // render something for agents that have no settable picker.
  const settable = r.modelConfig ? new Set(r.modelConfig.values) : null
  const models = (
    settable ? r.models.filter((m) => settable.has(m.id)) : r.models
  ).map((m) => ({
    id: m.id,
    name: m.name,
    description: m.description,
  }))
  return {
    models,
    reasoning: r.reasoning
      ? {
          values: [...r.reasoning.values],
          defaultValue: r.reasoning.defaultValue,
        }
      : null,
    supportsConfigOption: r.supportsConfigOption,
    agentInfo: r.agentInfo,
    protocolVersion: r.protocolVersion,
    error: r.error
      ? {
          code: r.error.code,
          message: r.error.message,
          acpErrorCode: r.error.acpError?.code,
        }
      : undefined,
  }
}
