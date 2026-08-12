/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import {
  type ConversationStore,
  DbConversationStore,
} from '../../lib/conversations/conversation-store'
import type { Env } from '../types'
import { ConversationIdParamSchema } from '../utils/validation'

type ConversationRouteStore = Pick<ConversationStore, 'list' | 'get' | 'delete'>

export function createConversationRoutes(
  options: { store?: ConversationRouteStore } = {},
) {
  const store = options.store ?? new DbConversationStore()

  return new Hono<Env>()
    .get('/', async (c) => c.json({ conversations: await store.list() }))
    .get(
      '/:conversationId',
      zValidator('param', ConversationIdParamSchema),
      async (c) => {
        const conversation = await store.get(
          c.req.valid('param').conversationId,
        )
        if (!conversation) return c.json({ error: 'Unknown conversation' }, 404)
        return c.json({ conversation })
      },
    )
    .delete(
      '/:conversationId',
      zValidator('param', ConversationIdParamSchema),
      async (c) => {
        const deleted = await store.delete(c.req.valid('param').conversationId)
        if (!deleted) return c.json({ error: 'Unknown conversation' }, 404)
        return c.json({ success: true })
      },
    )
}
