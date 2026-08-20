/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { homedir } from 'node:os'
import type { AcpAgentType } from '@browseros/shared/schemas/agent'
import { type AgentProbeResult, probeAgent as runProbe } from 'acp-probe'
import { resolveAcpSpawnCommand } from '../../../lib/agents/host-acp/launcher'
import { getBrowserosDir } from '../../../lib/browseros-dir'
import { logger } from '../../../lib/logger'

export interface ServerAcpxProbeInput {
  type: AcpAgentType
  timeoutMs?: number
  resourcesDir?: string | null
  browserosDir?: string | null
  platform?: NodeJS.Platform
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

// Cold adapter downloads can take two minutes on slow networks.
const DEFAULT_PROBE_TIMEOUT_MS = 120_000
const MAX_PROBE_TIMEOUT_MS = 120_000

function resolveTimeout(requested?: number): number {
  const envValue = Number(process.env.BROWSEROS_ACPX_PROBE_TIMEOUT_MS)
  if (
    Number.isFinite(envValue) &&
    envValue >= 1_000 &&
    envValue <= MAX_PROBE_TIMEOUT_MS
  ) {
    return envValue
  }
  return requested ?? DEFAULT_PROBE_TIMEOUT_MS
}

export async function probeAcpAgent(
  input: ServerAcpxProbeInput,
): Promise<ServerAcpxProbeResult> {
  const timeoutMs = resolveTimeout(input.timeoutMs)

  const launcher = resolveAcpSpawnCommand({
    agentType: input.type,
    browserosDir: input.browserosDir ?? getBrowserosDir(),
    resourcesDir: input.resourcesDir,
    platform: input.platform,
  })
  logger.debug('ACP probe using launcher-resolved command', {
    type: input.type,
    launcherSource: launcher.source,
  })

  // Spawn the adapter in a writable dir: acp-probe defaults to the server's
  // process.cwd(), which is the read-only app bundle in a packaged build, so
  // the adapter fails to create session files. Matches the chat provider's cwd.
  const result = await runProbe({
    argv: launcher.argv,
    cwd: homedir(),
    authPolicy: 'skip',
    timeoutMs,
  })
  return normalizeProbeResult(result)
}

// Codex ACP versions encode effort as either model[effort] or model/effort.
const COMPOUND_MODEL_PATTERN =
  /^(.+?)(?:\[(low|medium|high|xhigh|max)\]|\/(low|medium|high|xhigh|max))$/i

const EFFORT_ORDER = ['low', 'medium', 'high', 'xhigh', 'max']

function stripEffortFromName(name: string | undefined): string | undefined {
  if (!name) return name
  return name.replace(/\s*\((low|medium|high|xhigh|max)\)\s*$/i, '').trim()
}

function stripEffortFromDescription(
  description: string | undefined,
): string | undefined {
  if (!description) return description
  const idx = description.indexOf('. ')
  return idx > 0 ? description.slice(0, idx + 1) : description
}

interface CompoundSplit {
  models: ServerAcpxProbeModel[]
  efforts: string[] | null
}

function splitCompoundModels(raw: AgentProbeResult['models']): CompoundSplit {
  const bareById = new Map<string, ServerAcpxProbeModel>()
  const efforts = new Set<string>()
  let sawCompound = false
  for (const m of raw) {
    const match = COMPOUND_MODEL_PATTERN.exec(m.id)
    if (!match) {
      bareById.set(m.id, {
        id: m.id,
        name: m.name,
        description: m.description,
      })
      continue
    }
    const bareId = match[1]
    const rawEffort = match[2] ?? match[3]
    if (!bareId || !rawEffort) {
      bareById.set(m.id, {
        id: m.id,
        name: m.name,
        description: m.description,
      })
      continue
    }
    sawCompound = true
    const effort = rawEffort.toLowerCase()
    efforts.add(effort)
    if (!bareById.has(bareId)) {
      bareById.set(bareId, {
        id: bareId,
        name: stripEffortFromName(m.name) || bareId,
        description: stripEffortFromDescription(m.description),
      })
    }
  }
  if (!sawCompound) {
    return { models: Array.from(bareById.values()), efforts: null }
  }
  const ordered = EFFORT_ORDER.filter((e) => efforts.has(e))
  for (const e of efforts) {
    if (!ordered.includes(e)) ordered.push(e)
  }
  return { models: Array.from(bareById.values()), efforts: ordered }
}

function normalizeProbeResult(r: AgentProbeResult): ServerAcpxProbeResult {
  const modelOption = r.configOptions.find((o) => o.id === 'model')
  const pickerOptions =
    modelOption?.type === 'select' ? modelOption.options : undefined

  let models: ServerAcpxProbeModel[]
  let inferredEfforts: string[] | null = null

  if (pickerOptions && pickerOptions.length > 0) {
    models = pickerOptions.map((opt) => ({
      id: opt.value,
      name: opt.name,
      description: opt.description,
    }))
  } else {
    const split = splitCompoundModels(r.models)
    models = split.models
    inferredEfforts = split.efforts
  }

  let reasoning: ServerAcpxProbeReasoning | null
  if (r.reasoning) {
    reasoning = {
      values: [...r.reasoning.values],
      defaultValue: r.reasoning.defaultValue,
    }
  } else if (inferredEfforts?.length) {
    reasoning = {
      values: inferredEfforts,
      defaultValue: inferredEfforts.includes('medium')
        ? 'medium'
        : inferredEfforts[0],
    }
  } else {
    reasoning = null
  }

  return {
    models,
    reasoning,
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
