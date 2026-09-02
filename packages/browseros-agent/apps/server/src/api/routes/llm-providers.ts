/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import {
  dbLlmProviderStore,
  type LlmProviderStore,
} from '../../lib/llm-providers/provider-store'
import type { Env } from '../types'

const IdParamSchema = z.object({ providerId: z.string().min(1) })

/**
 * Mirrors the extension's LlmProviderConfig. `id` comes from the client rather
 * than the database so a provider keeps one identity across the extension, the
 * migration and this table, which is what makes re-importing idempotent.
 */
const UpsertProviderSchema = z.object({
  profileId: z.string().nullish(),
  type: z.string().min(1),
  name: z.string().min(1),
  baseUrl: z.string().nullish(),
  modelId: z.string().min(1),
  supportsImages: z.boolean().optional(),
  contextWindow: z.number(),
  temperature: z.number().optional(),
  apiKey: z.string().nullish(),
  accessKeyId: z.string().nullish(),
  secretAccessKey: z.string().nullish(),
  sessionToken: z.string().nullish(),
  resourceName: z.string().nullish(),
  region: z.string().nullish(),
  reasoningEffort: z.string().nullish(),
  reasoningSummary: z.string().nullish(),
  createdAt: z.number().optional(),
})

export function createLlmProviderRoutes(
  options: { store?: LlmProviderStore } = {},
) {
  const store = options.store ?? dbLlmProviderStore

  return new Hono<Env>()
    .get('/', async (c) => c.json({ providers: await store.list() }))
    .get('/:providerId', zValidator('param', IdParamSchema), async (c) => {
      const provider = await store.get(c.req.valid('param').providerId)
      if (!provider) return c.json({ error: 'Unknown provider' }, 404)
      return c.json({ provider })
    })
    .put(
      '/:providerId',
      zValidator('param', IdParamSchema),
      zValidator('json', UpsertProviderSchema),
      async (c) => {
        const provider = await store.upsert({
          ...c.req.valid('json'),
          id: c.req.valid('param').providerId,
        })
        return c.json({ provider })
      },
    )
    .delete('/:providerId', zValidator('param', IdParamSchema), async (c) => {
      const deleted = await store.remove(c.req.valid('param').providerId)
      if (!deleted) return c.json({ error: 'Unknown provider' }, 404)
      return c.json({ success: true })
    })
}
