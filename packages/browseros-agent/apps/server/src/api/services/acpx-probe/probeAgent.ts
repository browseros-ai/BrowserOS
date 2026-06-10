/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { type AgentProbeResult, probeAgent as runProbe } from 'acp-probe'
import { resolveAcpSpawnCommand } from '../../../lib/agents/host-acp/launcher'
import { logger } from '../../../lib/logger'

export interface ServerAcpxProbeInput {
  agentId?: string
  command?: string
  cwd?: string
  timeoutMs?: number
  /**
   * BrowserOS resources directory. When set, the probe prefers the
   * bundled Bun launcher at <resourcesDir>/bin/third_party/bun for
   * built-in agents so end-user installs without Node still resolve
   * the spawn correctly. Production callers thread this from the
   * HttpServerConfig.
   */
  resourcesDir?: string | null
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

// 120s gives the cold-cache tarball fetch + extract enough headroom on
// slow networks (corp VPN, antivirus scanning extracted files) without
// stranding the user behind a smaller deadline. Warm-cache spawns still
// return in well under a second so the ceiling is invisible in steady
// state. Env override is clamped to [1s, 120s] for the same reason.
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
  if (!input.agentId && !input.command) {
    throw new Error('Either agentId or command is required')
  }
  const timeoutMs = resolveTimeout(input.timeoutMs)

  // Built-in agent ids (claude, codex) get rewritten to an explicit
  // command when the bundled-Bun launcher resolves. When the launcher
  // would fall back to `npx -y …` we deliberately leave the agentId
  // alone so acpx's built-in registry produces the same command via
  // its own code path; this keeps callers that have not threaded
  // resourcesDir yet on the exact pre-existing behaviour.
  let agentId = input.agentId
  let command = input.command
  if (!command && agentId) {
    const launcher = resolveAcpSpawnCommand({
      agentType: agentId,
      resourcesDir: input.resourcesDir,
      platform: input.platform,
    })
    if (launcher?.source === 'bundled-bun') {
      command = launcher.command
      agentId = undefined
      logger.debug('ACP probe using bundled-bun launcher', {
        originalAgentId: input.agentId,
      })
    }
  }

  const result = await runProbe({
    agent: agentId,
    command,
    cwd: input.cwd,
    authPolicy: 'skip',
    timeoutMs,
  })
  return normalizeProbeResult(result)
}

// codex-acp encodes effort into the advertised model id when it does not
// expose a settable configOptions[id=model] picker. Older builds use
// `model[effort]`; newer builds use `model/effort`. Both forms appear in
// the wild so we match either.
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
  // Priority for the model dropdown source:
  //   1. configOptions[id=model].options (bare picker, names + descriptions)
  //   2. r.models, split when compound `model[effort]` / `model/effort`
  //      ids are present. Falls through to the raw list when ids are
  //      already bare (e.g. claude).
  // Effort dropdown source:
  //   1. r.reasoning.values when the agent exposes configOptions[category=thought_level]
  //   2. Efforts extracted from compound model ids
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
  } else if (inferredEfforts && inferredEfforts.length) {
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
