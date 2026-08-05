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
import type { Env } from '../types'

const AgentIdParamsSchema = z.object({ agentId: z.string().uuid() })

const CreateAcpAgentSchema = z.object({
  name: z.string().trim().min(1).max(80),
  type: AcpAgentTypeSchema,
  modelId: z.string().trim().min(1).optional(),
  reasoningEffort: z.string().trim().min(1).optional(),
  workingDirectory: z.string().trim().min(1).optional(),
})

const UpdateAcpAgentSchema = z
  .object({
    name: z.string().trim().min(1).max(80).optional(),
    pinned: z.boolean().optional(),
  })
  .refine((patch) => patch.name !== undefined || patch.pinned !== undefined, {
    message: 'At least one field is required',
  })

type AgentRouteStore = Pick<
  AcpAgentStore,
  'list' | 'get' | 'create' | 'update' | 'delete'
>

export function createAgentRoutes(options: { store?: AgentRouteStore } = {}) {
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
    .patch(
      '/:agentId',
      zValidator('param', AgentIdParamsSchema),
      zValidator('json', UpdateAcpAgentSchema),
      async (c) => {
        const agent = await store.update(
          c.req.valid('param').agentId,
          c.req.valid('json'),
        )
        if (!agent) return c.json({ error: 'Unknown agent' }, 404)
        return c.json({ agent })
      },
    )
    .delete(
      '/:agentId',
      zValidator('param', AgentIdParamsSchema),
      async (c) => {
        const deleted = await store.delete(c.req.valid('param').agentId)
        if (!deleted) return c.json({ error: 'Unknown agent' }, 404)
        return c.json({ success: true })
      },
    )
}
