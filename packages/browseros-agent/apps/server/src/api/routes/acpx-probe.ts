/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import { logger } from '../../lib/logger'
import {
  probeAcpAgent,
  type ServerAcpxProbeInput,
  type ServerAcpxProbeResult,
} from '../services/acpx-probe/probeAgent'

export type ProbeAcpAgentFn = (
  input: ServerAcpxProbeInput,
) => Promise<ServerAcpxProbeResult>

const probeRequestSchema = z
  .object({
    agentId: z.string().optional(),
    command: z.string().optional(),
    cwd: z.string().optional(),
    timeoutMs: z.number().int().min(1_000).max(120_000).optional(),
  })
  .refine((v) => Boolean(v.agentId || v.command), {
    message: 'Either agentId or command is required',
  })

export function createAcpxProbeRoutes(
  options: { probe?: ProbeAcpAgentFn; resourcesDir?: string | null } = {},
) {
  const probe = options.probe ?? probeAcpAgent
  const resourcesDir = options.resourcesDir
  return new Hono().post(
    '/',
    zValidator('json', probeRequestSchema),
    async (c) => {
      const body = c.req.valid('json')
      try {
        const result = await probe({ ...body, resourcesDir })
        return c.json(result, 200)
      } catch (err) {
        // Probe errors from inside acp-probe (spawn_failed, initialize_timeout,
        // auth_required, agent_crashed) flow back through probeAcpAgent as a
        // 200 with a populated `error` field. Reaching this branch means the
        // wrapper itself threw, which is unrecoverable from the dialog.
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
