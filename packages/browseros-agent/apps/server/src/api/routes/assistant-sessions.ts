/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Hono } from 'hono'
import type { Env } from '../types'

/**
 * Stub routes for assistant sessions.
 * Returns 501 Not Implemented for all endpoints until the assistant
 * session store is implemented.
 */
export function createAssistantSessionRoutes() {
  return new Hono<Env>()
    .get('/sessions', (c) =>
      c.json({ error: 'Not Implemented' }, 501),
    )
    .post('/sessions', (c) =>
      c.json({ error: 'Not Implemented' }, 501),
    )
    .get('/sessions/:id', (c) =>
      c.json({ error: 'Not Implemented' }, 501),
    )
    .patch('/sessions/:id', (c) =>
      c.json({ error: 'Not Implemented' }, 501),
    )
    .delete('/sessions/:id', (c) =>
      c.json({ error: 'Not Implemented' }, 501),
    )
}
