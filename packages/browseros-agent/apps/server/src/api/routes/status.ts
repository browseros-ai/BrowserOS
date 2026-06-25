/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Browser } from '@browseros/browser-core/browser'
import { Hono } from 'hono'

interface StatusDeps {
  browser?: Browser
}

export function createStatusRoute(deps: StatusDeps = {}) {
  return new Hono().get('/', (c) => {
    const cdpConnected = deps.browser?.isCdpConnected()
    return c.json(
      cdpConnected === undefined
        ? { status: 'ok' }
        : { status: 'ok', cdpConnected },
    )
  })
}
