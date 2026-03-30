/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Hono } from 'hono'
import type { Browser } from '../../browser/browser'

interface HealthDeps {
  browser?: Browser
}

export function createHealthRoute(deps: HealthDeps = {}) {
  return new Hono().get('/', (c) => {
    const cdpConnected = deps.browser?.isCdpConnected()
    return c.json(
      cdpConnected === undefined
        ? { status: 'ok' }
        : { status: 'ok', cdpConnected },
    )
  })
}
