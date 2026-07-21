/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Hono } from 'hono'

interface SystemRouteConfig {
  onShutdown?: () => void
}

export function createSystemRoute(config: SystemRouteConfig = {}) {
  return new Hono()
    .get('/system/health', (c) => c.json({ status: 'ok' as const }))
    .post('/system/shutdown', (c) => {
      setImmediate(() => config.onShutdown?.())
      return c.json({ status: 'ok' as const })
    })
}
