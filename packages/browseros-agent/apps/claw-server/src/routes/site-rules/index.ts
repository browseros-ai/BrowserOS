/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * /site-rules route chain. Thin Hono layer over `./service`: zValidator
 * rejects malformed bodies with structured 400s, and DELETE surfaces
 * the service's null-return as 404. The chained `.get / .post /
 * .delete` calls preserve the inferred shape `AppType` needs; do not
 * break the chain.
 */

import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { HttpError } from '../../lib/errors'
import { addSiteRuleSchema } from './schemas'
import { add, list, remove } from './service'

export const siteRulesRoute = new Hono()
  .get('/site-rules', async (c) => c.json(await list()))
  .post('/site-rules', zValidator('json', addSiteRuleSchema), async (c) => {
    const body = c.req.valid('json')
    const created = await add(body)
    return c.json(created, 201)
  })
  .delete('/site-rules/:id', async (c) => {
    const removed = await remove(c.req.param('id'))
    if (!removed) throw new HttpError(404, 'site rule not found')
    return c.json(removed)
  })
