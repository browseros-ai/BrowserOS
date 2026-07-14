/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Hono } from 'hono'
import pkg from '../../package.json' with { type: 'json' }
import { getLocalServerUrl } from '../local-server-url'
import { getTelemetryState } from '../services/analytics'
import { VERSION } from '../version'

interface SystemRouteConfig {
  onShutdown?: () => void
}

export function createSystemRoute(config: SystemRouteConfig = {}) {
  return (
    new Hono()
      .get('/system/health', (c) => c.json({ status: 'ok' as const }))
      .post('/system/shutdown', (c) => {
        setImmediate(() => config.onShutdown?.())
        return c.json({ status: 'ok' as const })
      })
      .get('/system/version', (c) =>
        c.json({ name: pkg.name, version: VERSION }),
      )
      .get('/system/url', (c) => c.json({ url: getLocalServerUrl() }))
      // The anonymous install id + opt-out state, so the cockpit UI can
      // share one anonymous identity with the server and reflect the
      // current telemetry setting. No PII: distinctId is a random UUID.
      .get('/system/telemetry', (c) => c.json(getTelemetryState()))
  )
}
