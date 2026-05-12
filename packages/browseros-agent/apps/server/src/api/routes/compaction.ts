/**
 * @license
 * Copyright 2025 BrowserOS
 *
 * API route for compaction strategy configuration.
 * Reads/writes the `compaction` field from the server config file
 * (the one passed via --config, NOT server.json).
 */
import fs from 'node:fs'

import { Hono } from 'hono'

import {
  CompactionStrategySchema,
  getResolvedConfigFilePath,
} from '../../config'
import { logger } from '../../lib/logger'
import type { Env } from '../types'

interface CompactionConfigResponse {
  active: {
    method: 'default' | 'vcc'
    customPrompt?: string
    vccConfig?: {
      maxTranscriptLines?: number
      maxGoalLines?: number
      maxFileEntries?: number
      maxCommitEntries?: number
      maxPreferenceLines?: number
      maxOutstandingLines?: number
    }
  } | null
  defaults: { method: 'default' }
}

interface CompactionSaveResponse {
  ok: boolean
  saved?: CompactionConfigResponse['active']
  errors?: Array<{ key: string; message: string }>
}

function getConfigFilePath(): string | null {
  return getResolvedConfigFilePath()
}

function readConfigFile(): Record<string, unknown> | null {
  const configPath = getConfigFilePath()
  if (!configPath || !fs.existsSync(configPath)) return null
  try {
    return JSON.parse(fs.readFileSync(configPath, 'utf-8'))
  } catch (e) {
    logger.warn('Failed to read config file for compaction API', {
      error: e instanceof Error ? e.message : String(e),
    })
    return null
  }
}

function writeConfigFile(config: Record<string, unknown>): void {
  const configPath = getConfigFilePath()
  if (!configPath) {
    throw new Error('No config file path available')
  }
  fs.writeFileSync(configPath, `${JSON.stringify(config, null, 2)}\n`, 'utf-8')
}

export function createCompactionRoutes() {
  return new Hono<Env>()
    .get('/', (c) => {
      const config = readConfigFile()
      const compaction = config
        ? (config.compaction as CompactionConfigResponse['active'] | undefined)
        : undefined

      const response: CompactionConfigResponse = {
        active: compaction ?? null,
        defaults: { method: 'default' },
      }

      return c.json(response)
    })
    .put('/', async (c) => {
      const body = (await c.req.json().catch(() => null)) as Record<
        string,
        unknown
      > | null

      if (!body || typeof body !== 'object') {
        const resp: CompactionSaveResponse = {
          ok: false,
          errors: [{ key: '_', message: 'Missing request body' }],
        }
        return c.json(resp, 400)
      }

      const result = CompactionStrategySchema.safeParse(body)
      if (!result.success) {
        const errors = result.error.issues.map((issue) => ({
          key: issue.path.join('.') || '_',
          message: issue.message,
        }))
        const resp: CompactionSaveResponse = { ok: false, errors }
        return c.json(resp, 400)
      }

      let config = readConfigFile()
      if (!config) {
        config = {}
      }
      config.compaction = result.data
      writeConfigFile(config)

      const resp: CompactionSaveResponse = {
        ok: true,
        saved: result.data as CompactionConfigResponse['active'],
      }
      return c.json(resp)
    })
    .delete('/', (c) => {
      const config = readConfigFile()
      if (!config) {
        // No config file — nothing to remove
        const resp: CompactionSaveResponse = { ok: true }
        return c.json(resp)
      }

      delete config.compaction
      writeConfigFile(config)

      const resp: CompactionSaveResponse = { ok: true }
      return c.json(resp)
    })
}
