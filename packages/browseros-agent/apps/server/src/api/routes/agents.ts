/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { AcpAgentTypeSchema } from '@browseros/shared/schemas/agent'
import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import {
  type AcpAgentStore,
  DbAcpAgentStore,
} from '../../lib/agents/storage/acp-agent-store'
import { logger } from '../../lib/logger'
import type { Env } from '../types'

const AgentIdParamsSchema = z.object({ agentId: z.string().uuid() })

const CreateAcpAgentSchema = z
  .object({
    name: z.string().trim().min(1).max(80),
    type: AcpAgentTypeSchema,
    modelId: z.string().trim().min(1).optional(),
    reasoningEffort: z.string().trim().min(1).optional(),
    workingDirectory: z.string().trim().min(1).optional(),
  })
  .strict()

type AgentRouteStore = Pick<AcpAgentStore, 'list' | 'get' | 'create' | 'delete'>

export function createAgentRoutes(
  options: {
    store?: AgentRouteStore
    onDelete?: (agentId: string) => Promise<unknown>
  } = {},
) {
  const store = options.store ?? new DbAcpAgentStore()

  return new Hono<Env>()
    .get('/', async (c) => c.json({ agents: await store.list() }))
    .post('/', zValidator('json', CreateAcpAgentSchema), async (c) =>
      c.json({ agent: await store.create(c.req.valid('json')) }, 201),
    )
    .get('/:agentId', zValidator('param', AgentIdParamsSchema), async (c) => {
      const agent = await store.get(c.req.valid('param').agentId)
      if (!agent) return c.json({ error: 'Unknown agent' }, 404)
      return c.json({ agent })
    })
    .delete(
      '/:agentId',
      zValidator('param', AgentIdParamsSchema),
      async (c) => {
        const { agentId } = c.req.valid('param')
        const deleted = await store.delete(agentId)
        if (!deleted) return c.json({ error: 'Unknown agent' }, 404)
        await options.onDelete?.(agentId).catch((error) => {
          logger.warn('Failed to close deleted ACP agent sessions', {
            agentId,
            error: error instanceof Error ? error.message : String(error),
          })
        })
        return c.json({ success: true })
      },
    )
}
