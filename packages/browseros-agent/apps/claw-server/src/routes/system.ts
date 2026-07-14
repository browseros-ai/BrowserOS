/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Hono } from 'hono'
import pkg from '../../package.json' with { type: 'json' }
import { getLocalServerUrl } from '../local-server-url'
import { VERSION } from '../version'

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
    .get('/system/version', (c) => c.json({ name: pkg.name, version: VERSION }))
    .get('/system/url', (c) => c.json({ url: getLocalServerUrl() }))
}
