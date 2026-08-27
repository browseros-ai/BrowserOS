/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { AcpAgentTypeSchema } from '@browseros/shared/schemas/agent'
import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import { logger } from '../../lib/logger'
import {
  probeAcpAgent,
  type ServerAcpxProbeInput,
  type ServerAcpxProbeResult,
} from '../services/acpx-probe/probeAgent'
import type { Env } from '../types'

export type ProbeAcpAgentFn = (
  input: ServerAcpxProbeInput,
) => Promise<ServerAcpxProbeResult>

const probeRequestSchema = z
  .object({
    type: AcpAgentTypeSchema,
    command: z.string().trim().min(1).optional(),
    env: z.record(z.string(), z.string()).optional(),
    cwd: z.string().trim().min(1).optional(),
    timeoutMs: z.number().int().min(1_000).max(120_000).optional(),
  })
  .strict()
  .superRefine((value, ctx) => {
    if (value.type === 'custom' && !value.command) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'command is required to probe a custom agent',
        path: ['command'],
      })
    }
  })

export function createAcpxProbeRoutes(
  options: { probe?: ProbeAcpAgentFn; resourcesDir?: string | null } = {},
) {
  const probe = options.probe ?? probeAcpAgent
  const resourcesDir = options.resourcesDir
  return new Hono<Env>().post(
    '/',
    zValidator('json', probeRequestSchema),
    async (c) => {
      const body = c.req.valid('json')
      try {
        const result = await probe({ ...body, resourcesDir })
        return c.json(result, 200)
      } catch (err) {
        logger.warn('ACP probe wrapper crashed', {
          error: err instanceof Error ? err.message : String(err),
        })
        return c.json(
          {
            error: {
              code: 'wrapper_error',
              message: err instanceof Error ? err.message : String(err),
            },
          },
          500,
        )
      }
    },
  )
}
