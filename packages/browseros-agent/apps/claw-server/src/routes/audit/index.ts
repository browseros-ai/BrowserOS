/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * v2 audit-log read route. Cursor-paginated over `id DESC` so newest
 * dispatches surface first. Filters by `agentId` and `sessionId` for
 * the operator's drill-down.
 */

import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import { listDispatches } from '../../services/audit-log'

const listQuerySchema = z.object({
  agentId: z.string().optional(),
  sessionId: z.string().optional(),
  cursor: z.coerce.number().int().positive().optional(),
  limit: z.coerce.number().int().positive().max(500).optional(),
})

export const auditRoute = new Hono().get(
  '/audit/dispatches',
  zValidator('query', listQuerySchema),
  (c) => {
    const query = c.req.valid('query')
    const result = listDispatches(query)
    return c.json(result)
  },
)
