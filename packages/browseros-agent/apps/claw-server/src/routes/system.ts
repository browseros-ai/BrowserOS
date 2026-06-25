/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Hono } from 'hono'
// Package version is read off the bundled package.json. resolveJsonModule
// + bun's loader resolve this without a build step.
import pkg from '../../package.json' with { type: 'json' }
import { getLocalServerUrl } from '../local-server-url'

export const systemRoute = new Hono()
  .get('/system/health', (c) => c.json({ status: 'ok' as const }))
  .get('/system/version', (c) =>
    c.json({ name: pkg.name, version: pkg.version }),
  )
  .get('/system/url', (c) => c.json({ url: getLocalServerUrl() }))
