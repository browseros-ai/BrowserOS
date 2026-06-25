/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * v2 task-centric read routes. Tasks are derived at read time from
 * tool_dispatches + agent_session_starts + agent_session_ends.
 *
 *   GET /cockpit/audit/tasks             paginated task list
 *   GET /cockpit/audit/tasks/:sessionId  full task detail incl. dispatches
 *
 * The flat `/cockpit/audit/dispatches` route stays as a low-level
 * fallback for callers that want raw rows.
 */

import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import { getTask, listTasks } from '../../services/tasks'

const listQuerySchema = z.object({
  agentId: z.string().optional(),
  status: z.enum(['live', 'done', 'failed']).optional(),
  site: z.string().optional(),
  search: z.string().optional(),
  since: z.coerce.number().int().nonnegative().optional(),
  cursor: z.coerce.number().int().positive().optional(),
  limit: z.coerce.number().int().min(1).max(100).optional(),
})

export const auditTasksRoute = new Hono()
  .get('/audit/tasks', zValidator('query', listQuerySchema), (c) => {
    const query = c.req.valid('query')
    return c.json(listTasks(query))
  })
  .get('/audit/tasks/:sessionId', (c) => {
    const sessionId = c.req.param('sessionId')
    const task = getTask(sessionId)
    if (!task) return c.json({ error: 'not found' }, 404)
    return c.json(task)
  })
