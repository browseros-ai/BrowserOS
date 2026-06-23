/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * /permissions route chain. Returns the system-wide approval catalog
 * baked into `lib/approval-catalog.ts`. Customisation (PUT/PATCH on
 * the catalog) is intentionally deferred; the UI keeps its own copy as
 * a fetch-failure fallback.
 */

import { Hono } from 'hono'
import { APPROVAL_CATEGORIES } from '../../lib/approval-catalog'

export const permissionsRoute = new Hono().get('/permissions/catalog', (c) =>
  c.json(APPROVAL_CATEGORIES),
)
