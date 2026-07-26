/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { createHash } from 'node:crypto'
import { mkdir, readFile, rename, writeFile } from 'node:fs/promises'
import { dirname } from 'node:path'
import type { AgentSessionId } from '../agent-types'

export interface LatestRuntimeState {
  sessionId: AgentSessionId
  runtimeSessionKey: string
  cwd: string
  agentHome: string
  updatedAt: number
}

interface RuntimeStateFile {
  version: 1
  latest: LatestRuntimeState
}

export async function loadLatestRuntimeState(
  filePath: string,
): Promise<LatestRuntimeState | null> {
  try {
    const parsed = JSON.parse(
      await readFile(filePath, 'utf8'),
    ) as RuntimeStateFile
    if (parsed.version !== 1 || !isLatestRuntimeState(parsed.latest)) {
      return null
    }
    return parsed.latest
  } catch {
    return null
  }
}

export async function saveLatestRuntimeState(
  filePath: string,
  latest: LatestRuntimeState,
): Promise<void> {
  await mkdir(dirname(filePath), { recursive: true })
  const tmpPath = `${filePath}.${process.pid}.${Date.now()}.tmp`
  await writeFile(
    tmpPath,
    `${JSON.stringify({ version: 1, latest }, null, 2)}\n`,
    'utf8',
  )
  await rename(tmpPath, filePath)
}

export function deriveRuntimeSessionKey(input: {
  agentId: string
  sessionId: AgentSessionId
  adapter: string
  cwd: string
  agentHome: string
  promptVersion: string
  skillIdentity: string
  commandIdentity: string
}): string {
  const fingerprint = createHash('sha256')
    .update(stableJson(input))
    .digest('hex')
    .slice(0, 16)
  return `agent:${input.agentId}:${input.sessionId}:${fingerprint}`
}

function isLatestRuntimeState(value: unknown): value is LatestRuntimeState {
  if (!value || typeof value !== 'object') return false
  const record = value as Record<string, unknown>
  return (
    typeof record.sessionId === 'string' &&
    record.sessionId.length > 0 &&
    typeof record.runtimeSessionKey === 'string' &&
    typeof record.cwd === 'string' &&
    typeof record.agentHome === 'string' &&
    typeof record.updatedAt === 'number'
  )
}

function stableJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(stableJson).join(',')}]`
  if (value && typeof value === 'object') {
    return `{${Object.entries(value as Record<string, unknown>)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, entry]) => `${JSON.stringify(key)}:${stableJson(entry)}`)
      .join(',')}}`
  }
  return JSON.stringify(value)
}
