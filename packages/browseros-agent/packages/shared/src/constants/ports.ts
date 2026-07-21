/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Centralized port configuration.
 * All port values in the monorepo should reference this file.
 *
 * in sync with:
 *   chromium/src/chrome/browser/browseros/server/browseros_server_prefs.h
 */

/**
 * Production ports (base 9000, offset 100)
 * Matches Chromium defaults in browseros_server_prefs.h
 */
export const DEFAULT_PORTS = {
  cdp: 9000,
  server: 9100,
  extension: 9300,
} as const

/**
 * Test ports (base 9005, offset 100)
 * Uses 05 suffix to avoid conflicts with dev server
 */
export const TEST_PORTS = {
  cdp: 9005,
  server: 9105,
  extension: 9305,
} as const

/**
 * Development ports (base 9010, offset 100)
 * Offset by 10 from default to avoid conflicts with production
 */
export const DEV_PORTS = {
  cdp: 9010,
  server: 9110,
  extension: 9310,
} as const

/** Default loopback port for the standalone BrowserClaw API. */
export const CLAW_API_PORT_DEFAULT = 9200

/**
 * Default CDP port BrowserClaw dials when attaching to BrowserOS Chromium.
 * The non-round private-range value avoids registered services and common local tools.
 */
export const CLAW_CDP_PORT_DEFAULT = 49337

// OAuth callback port — must match the redirect_uri registered with OpenAI's Codex client ID
export const OAUTH_CALLBACK_PORT = 1455

export type Ports = typeof DEFAULT_PORTS
