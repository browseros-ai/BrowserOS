/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

/**
 * Production API port for the cockpit. The cockpit's Hono app mounts
 * inside `@browseros/server`'s HTTP runtime (port 9100 by default)
 * under the `/cockpit` prefix, so the UI base URL is
 * `http://127.0.0.1:9100/cockpit`.
 *
 * The standalone `src/main.ts` still binds on `DEV_STANDALONE_PORT`
 * (9200) for solo dev and tests, but production traffic goes through
 * `apps/server`.
 *
 * Existing BrowserOS port allocations (per
 * apps/server/.env.example): CDP=9000, server=9100, extension=9300.
 */
export const PROD_API_PORT = 9100

/** Mount prefix the cockpit's app is routed under inside apps/server. */
export const COCKPIT_MOUNT_PREFIX = '/cockpit'

/** Standalone dev port for `src/main.ts` when running detached. */
export const DEV_STANDALONE_PORT = 9200
