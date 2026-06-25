/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * /connections route chain. Drives the v2 MCP page's per-harness
 * "Connect" buttons: GET lists install state for every supported
 * harness, POST :harness/connect installs BrowserOS as an MCP server
 * in that harness's config file, POST :harness/disconnect removes
 * it. The actual filesystem writes live in
 * `services/browseros-connect`; this layer is the HTTP shape and
 * validation.
 */

import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import {
  connectBrowserosToHarness,
  disconnectBrowserosFromHarness,
  listBrowserosConnections,
} from '../../services/browseros-connect'
import { harnessEnum } from '../agents/schemas'

const harnessParamSchema = z.object({ harness: harnessEnum })

export const connectionsRoute = new Hono()
  .get('/connections', async (c) => {
    return c.json({ connections: await listBrowserosConnections() })
  })
  .post(
    '/connections/:harness/connect',
    zValidator('param', harnessParamSchema),
    async (c) => {
      const { harness } = c.req.valid('param')
      const state = await connectBrowserosToHarness(harness)
      return c.json(state)
    },
  )
  .post(
    '/connections/:harness/disconnect',
    zValidator('param', harnessParamSchema),
    async (c) => {
      const { harness } = c.req.valid('param')
      const state = await disconnectBrowserosFromHarness(harness)
      return c.json(state)
    },
  )
