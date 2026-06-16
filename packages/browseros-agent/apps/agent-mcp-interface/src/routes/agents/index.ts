/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * /agents route chain. Thin Hono layer over `./service`: translate
 * HTTP shape in and out, surface 404 when the service returns null,
 * and let `zValidator` reject malformed bodies with structured 400s.
 *
 * The chained `.post / .get / .patch / .delete` calls preserve the
 * inferred shape `AppType` needs; do not break the chain.
 */

import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { HttpError } from '../../lib/errors'
import { newAgentValuesSchema } from './schemas'
import {
  create,
  getDetail,
  list,
  regenerateMcpUrl,
  remove,
  update,
} from './service'

export const agentsRoute = new Hono()
  .post('/agents', zValidator('json', newAgentValuesSchema), async (c) => {
    const body = c.req.valid('json')
    const created = await create(body)
    return c.json(created, 201)
  })
  .get('/agents', async (c) => c.json(await list()))
  .get('/agents/:id', async (c) => {
    const detail = await getDetail(c.req.param('id'))
    if (!detail) throw new HttpError(404, 'agent not found')
    return c.json(detail)
  })
  .patch('/agents/:id', zValidator('json', newAgentValuesSchema), async (c) => {
    const id = c.req.param('id')
    const body = c.req.valid('json')
    const updated = await update(id, body)
    if (!updated) throw new HttpError(404, 'agent not found')
    return c.json(updated)
  })
  .delete('/agents/:id', async (c) => {
    const removed = await remove(c.req.param('id'))
    if (!removed) throw new HttpError(404, 'agent not found')
    return c.json(removed)
  })
  .post('/agents/:id/mcp-url:regenerate', async (c) => {
    const rotated = await regenerateMcpUrl(c.req.param('id'))
    if (!rotated) throw new HttpError(404, 'agent not found')
    return c.json(rotated)
  })
