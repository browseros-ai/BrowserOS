/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Hono } from 'hono'

interface ShutdownRouteConfig {
  onShutdown: () => void
}

export function createShutdownRoute(config: ShutdownRouteConfig) {
  return new Hono().post('/', (c) => {
    // Shipped BrowserOS 0.46.x browsers call this from C++ managed restarts and require 200 + exit code 0; do not remove as unused.
    setImmediate(config.onShutdown)
    return c.json({ status: 'ok' })
  })
}
