/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Streams the persisted screenshot JPEG for a given dispatch id.
 * The file is served directly via Bun.file() with a long immutable
 * cache header so the UI can render thumbs without re-fetching on
 * every refetchInterval tick.
 */

import { existsSync } from 'node:fs'
import { Hono } from 'hono'
import { screenshotPath } from '../../services/screenshots'

export const auditScreenshotsRoute = new Hono().get(
  '/audit/screenshot/:dispatchId',
  (c) => {
    const id = Number(c.req.param('dispatchId'))
    if (!Number.isInteger(id) || id <= 0) {
      return c.json({ error: 'bad dispatchId' }, 400)
    }
    const path = screenshotPath(id)
    if (!existsSync(path)) {
      return c.json({ error: 'not found' }, 404)
    }
    return new Response(Bun.file(path), {
      headers: {
        'content-type': 'image/jpeg',
        'cache-control': 'public, max-age=86400, immutable',
      },
    })
  },
)
