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

/**
 * Default CDP port the cockpit dials when attaching to the BrowserOS
 * Chromium. Lives in IANA's dynamic / private range (49152-65535)
 * so it cannot collide with a registered service, and is not a round
 * number so a hand-rolled local script is unlikely to pick the same
 * value. Override via `BROWSEROS_COCKPIT_CDP_PORT`.
 *
 * Important: this is the port BrowserOS's Chromium exposes its
 * DevTools on, not a port the cockpit itself opens. Until the
 * BrowserOS browser shell defaults to the same value, the env var
 * is the bridge — set it to whatever DevTools port BrowserOS is
 * currently bound on.
 */
export const COCKPIT_CDP_PORT_DEFAULT = 49337
