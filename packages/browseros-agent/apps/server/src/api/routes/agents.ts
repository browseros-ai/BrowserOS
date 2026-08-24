/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  AcpAgentTypeSchema,
  CustomAcpAgentConfigSchema,
} from '@browseros/shared/schemas/agent'
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
    customConfig: CustomAcpAgentConfigSchema.optional(),
  })
  .strict()
  .superRefine((value, ctx) => {
    if (value.type === 'custom' && !value.customConfig) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'customConfig is required for custom agents',
        path: ['customConfig'],
      })
    }
    if (value.type !== 'custom' && value.customConfig) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'customConfig is only allowed for custom agents',
        path: ['customConfig'],
      })
    }
  })

const UpdateAcpAgentSchema = z
  .object({
    name: z.string().trim().min(1).max(80).optional(),
    modelId: z.string().trim().min(1).nullable().optional(),
    reasoningEffort: z.string().trim().min(1).nullable().optional(),
    workingDirectory: z.string().trim().min(1).nullable().optional(),
    customConfig: CustomAcpAgentConfigSchema.optional(),
  })
  .strict()

type AgentRouteStore = Pick<
  AcpAgentStore,
  'list' | 'get' | 'create' | 'update' | 'delete'
>

export function createAgentRoutes(
  options: {
    store?: AgentRouteStore
    onDelete?: (agentId: string) => Promise<unknown>
    onUpdate?: (agentId: string) => Promise<unknown>
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
    .put(
      '/:agentId',
      zValidator('param', AgentIdParamsSchema),
      zValidator('json', UpdateAcpAgentSchema),
      async (c) => {
        const { agentId } = c.req.valid('param')
        const patch = c.req.valid('json')
        const existing = await store.get(agentId)
        if (!existing) return c.json({ error: 'Unknown agent' }, 404)
        if (patch.customConfig && existing.type !== 'custom') {
          return c.json(
            { error: 'customConfig is only allowed for custom agents' },
            400,
          )
        }
        const agent = await store.update(agentId, patch)
        if (!agent) return c.json({ error: 'Unknown agent' }, 404)
        // Drop any running sessions so the next turn re-spawns with the new
        // command/config (a no-op when nothing is running).
        await options.onUpdate?.(agentId).catch((error) => {
          logger.warn('Failed to refresh updated ACP agent sessions', {
            agentId,
            error: error instanceof Error ? error.message : String(error),
          })
        })
        return c.json({ agent })
      },
    )
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
