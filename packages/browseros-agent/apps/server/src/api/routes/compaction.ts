/**
 * @license
 * Copyright 2025 BrowserOS
 *
 * API route for compaction strategy configuration.
 * Reads/writes the `compaction` field from the server config file.
 */
import fs from 'node:fs'

import { Hono } from 'hono'

import { CompactionStrategySchema } from '../../config'
import { getServerConfigPath } from '../../lib/browseros-dir'
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

function readConfigFile(): Record<string, unknown> {
  const configPath = getServerConfigPath()
  if (!fs.existsSync(configPath)) return {}
  try {
    return JSON.parse(fs.readFileSync(configPath, 'utf-8'))
  } catch {
    return {}
  }
}

function writeConfigFile(config: Record<string, unknown>): void {
  const configPath = getServerConfigPath()
  fs.writeFileSync(configPath, `${JSON.stringify(config, null, 2)}\n`, 'utf-8')
}

export function createCompactionRoutes() {
  return new Hono<Env>()
    .get('/', (c) => {
      const config = readConfigFile()
      const compaction = config.compaction as
        | CompactionConfigResponse['active']
        | undefined

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

      const config = readConfigFile()
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
      delete config.compaction
      writeConfigFile(config)

      const resp: CompactionSaveResponse = { ok: true }
      return c.json(resp)
    })
}
