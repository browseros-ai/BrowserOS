/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Browser } from '@browseros/browser-core/browser'
import { Hono } from 'hono'
import type { ServerActivity } from '../services/server-activity'

interface StatusDeps {
  activity?: ServerActivity
  browser?: Browser
}

export function createStatusRoute(deps: StatusDeps = {}) {
  return new Hono().get('/', (c) => {
    const cdpConnected = deps.browser?.isCdpConnected()
    const canUpdate = !deps.activity?.isBusy()
    return c.json(
      cdpConnected === undefined
        ? { status: 'ok', can_update: canUpdate }
        : { status: 'ok', cdpConnected, can_update: canUpdate },
    )
  })
}
