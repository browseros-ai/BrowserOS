/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Thin orchestration over RemoteHermesService. When the service is null
 * the provider is not configured (no AGENT_RUNNER_JWT_SECRET in env);
 * we return a soft response so the agent UI can degrade gracefully.
 */

import { Hono } from 'hono'
import type { RemoteHermesService } from '../services/remote-hermes/remote-hermes-service'

export interface RemoteHermesRouteDeps {
  service: RemoteHermesService | null
}

const NOT_CONFIGURED = { ok: false, reason: 'not_configured' } as const

export function createRemoteHermesRoutes(deps: RemoteHermesRouteDeps) {
  const { service } = deps
  return new Hono()
    .post('/start', (c) => {
      if (!service) return c.json(NOT_CONFIGURED)
      void service.warm()
      return c.json({ ok: true })
    })
    .post('/destroy', (c) => {
      if (!service) return c.json(NOT_CONFIGURED)
      void service.teardown()
      return c.json({ ok: true })
    })
    .get('/status', async (c) => {
      if (!service) return c.json({ error: 'not_configured' }, 500)
      try {
        const view = await service.status(c.req.raw.signal)
        return c.json(view)
      } catch (err) {
        return c.json(
          {
            error: 'upstream_unreachable',
            message: err instanceof Error ? err.message : String(err),
          },
          502,
        )
      }
    })
}
